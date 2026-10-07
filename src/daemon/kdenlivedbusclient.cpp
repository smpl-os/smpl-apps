// SPDX-License-Identifier: GPL-2.0-or-later
#include "kdenlivedbusclient.h"
#include "kdenlivecontract.h"

#include <QDBusArgument>
#include <QDBusMetaType>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDBusServiceWatcher>
#include <QDBusVariant>

namespace cs {

KdenliveDBusClient::KdenliveDBusClient(const QDBusConnection &connection, QObject *parent)
    : KdenliveClient(parent)
    , m_conn(connection)
{
}

KdenliveDBusClient::~KdenliveDBusClient()
{
    detach();
}

QVariant KdenliveDBusClient::normalize(const QVariant &v)
{
    if (v.metaType() == QMetaType::fromType<QDBusVariant>()) {
        return normalize(v.value<QDBusVariant>().variant());
    }
    if (v.metaType() == QMetaType::fromType<QVariantMap>()) {
        QVariantMap out;
        const QVariantMap in = v.toMap();
        for (auto it = in.begin(); it != in.end(); ++it) {
            out.insert(it.key(), normalize(it.value()));
        }
        return out;
    }
    if (v.metaType() == QMetaType::fromType<QVariantList>()) {
        QVariantList out;
        for (const auto &e : v.toList()) {
            out << normalize(e);
        }
        return out;
    }
    if (v.metaType() != QMetaType::fromType<QDBusArgument>()) {
        return v;
    }
    const QDBusArgument a = v.value<QDBusArgument>();
    switch (a.currentType()) {
    case QDBusArgument::MapType: {
        QVariantMap m;
        a.beginMap();
        while (!a.atEnd()) {
            a.beginMapEntry();
            const QVariant key = a.asVariant();
            const QVariant val = a.asVariant();
            a.endMapEntry();
            m.insert(key.toString(), normalize(val));
        }
        a.endMap();
        return m;
    }
    case QDBusArgument::ArrayType: {
        QVariantList l;
        a.beginArray();
        while (!a.atEnd()) {
            l << normalize(a.asVariant());
        }
        a.endArray();
        return l;
    }
    case QDBusArgument::StructureType: {
        QVariantList l;
        a.beginStructure();
        while (!a.atEnd()) {
            l << normalize(a.asVariant());
        }
        a.endStructure();
        return l;
    }
    default:
        return a.asVariant();
    }
}

QDBusMessage KdenliveDBusClient::call(const QString &method) const
{
    auto msg = QDBusMessage::createMethodCall(m_service, contract::kPath, contract::kInterface, method);
    msg.setAutoStartService(false);
    return msg;
}

void KdenliveDBusClient::setServiceOverride(const QString &service)
{
    m_hasOverride = true;
    m_override = service;
}

void KdenliveDBusClient::attachToPid(qint64 pid)
{
    if (m_hasOverride) {
        m_pid = pid;
        if (pid <= 0) {
            detach();
        } else if (!m_attached) {
            attachToService(m_override);
        }
        return;
    }
    if (pid == m_pid && m_attached) {
        // Kdenlive may still have been starting when we first asked: retry
        // when its window is focused again.
        if (!m_available && pid > 0 && m_lastAttempt.isValid() && m_lastAttempt.elapsed() > 3000) {
            attachToService(m_service);
        }
        return;
    }
    m_pid = pid;
    if (pid <= 0) {
        detach();
        return;
    }
    attachToService(contract::kServicePrefix + QString::number(pid));
}

void KdenliveDBusClient::detach()
{
    if (!m_attached) {
        return;
    }
    if (m_conn.isConnected()) {
        m_conn.send(call(QStringLiteral("Unsubscribe")));
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ContextChanged"), this,
                          SLOT(onContextChanged(QVariantMap)));
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ControlAck"), this,
                          SLOT(onControlAck(uint, QString, QVariantMap)));
    }
    delete m_watcher;
    m_watcher = nullptr;
    m_attached = false;
    ++m_generation;
    m_contextSerial = -1;
    m_context.clear();
    setAvailable(false);
}

void KdenliveDBusClient::attachToService(const QString &service)
{
    detach();
    m_service = service;
    m_attached = true;
    m_lastAttempt.start();
    const quint64 gen = ++m_generation;
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ContextChanged"), this, SLOT(onContextChanged(QVariantMap)));
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ControlAck"), this,
                   SLOT(onControlAck(uint, QString, QVariantMap)));
    if (!service.isEmpty()) {
        m_watcher = new QDBusServiceWatcher(service, m_conn, QDBusServiceWatcher::WatchForUnregistration, this);
        connect(m_watcher, &QDBusServiceWatcher::serviceUnregistered, this, [this] {
            Q_EMIT message(QStringLiteral("%1 went away").arg(m_service));
            detach();
            m_pid = 0;
        });
    }
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(call(QStringLiteral("Subscribe")), 2000), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, gen] {
        w->deleteLater();
        if (gen != m_generation) {
            return;  // re-attached meanwhile
        }
        const QDBusMessage reply = w->reply();
        if (reply.type() != QDBusMessage::ReplyMessage || reply.arguments().isEmpty()) {
            Q_EMIT message(QStringLiteral("%1 has no %2 (%3); key fallbacks stay active")
                               .arg(m_service.isEmpty() ? QStringLiteral("peer") : m_service, contract::kInterface, reply.errorName()));
            setAvailable(false);
            return;
        }
        onContextChanged(normalize(reply.arguments().constFirst()).toMap());
        setAvailable(true);
        Q_EMIT message(QStringLiteral("attached to %1").arg(m_service.isEmpty() ? QStringLiteral("peer") : m_service));
    });
}

void KdenliveDBusClient::setAvailable(bool on)
{
    if (m_available != on) {
        m_available = on;
        Q_EMIT availabilityChanged(on);
    }
}

void KdenliveDBusClient::onContextChanged(const QVariantMap &context)
{
    const QVariantMap ctx = normalize(context).toMap();
    const qint64 serial = ctx.value(QStringLiteral("serial"), -1).toLongLong();
    if (serial >= 0 && serial <= m_contextSerial) {
        return;  // stale or duplicate
    }
    m_contextSerial = serial;
    m_context = ctx;
    Q_EMIT contextChanged(m_context);
}

void KdenliveDBusClient::onControlAck(uint seq, const QString &control, const QVariantMap &state)
{
    Q_UNUSED(seq)
    Q_EMIT controlAcked(control, normalize(state).toMap());
}

void KdenliveDBusClient::triggerAction(const QString &id)
{
    if (!m_attached) {
        Q_EMIT actionFailed(id);
        return;
    }
    auto msg = call(QStringLiteral("TriggerAction"));
    msg << id;
    const quint64 gen = m_generation;
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(msg, 1000), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, id, gen] {
        w->deleteLater();
        if (gen != m_generation) {
            return;  // focus/instance changed meanwhile: never act on a stale reply
        }
        QDBusPendingReply<bool> r = *w;
        if (!r.isError()) {
            if (!r.value()) {
                Q_EMIT message(QStringLiteral("action %1 not triggered (unknown, disabled or dialog open)").arg(id));
                Q_EMIT actionFailed(id);
            }
            return;
        }
        // Only a definitely absent interface justifies typing fallback keys. A
        // timeout may still execute later in Kdenlive, so it must not double up.
        static const QStringList absent{QStringLiteral("org.freedesktop.DBus.Error.ServiceUnknown"),
                                        QStringLiteral("org.freedesktop.DBus.Error.NameHasNoOwner"),
                                        QStringLiteral("org.freedesktop.DBus.Error.UnknownObject"),
                                        QStringLiteral("org.freedesktop.DBus.Error.UnknownInterface"),
                                        QStringLiteral("org.freedesktop.DBus.Error.UnknownMethod")};
        Q_EMIT message(QStringLiteral("action %1: %2").arg(id, r.error().message()));
        if (absent.contains(r.error().name())) {
            Q_EMIT actionFailed(id);
        }
    });
}

void KdenliveDBusClient::control(const QString &name, double delta, const QVariantMap &options)
{
    if (!m_attached) {
        return;
    }
    auto msg = call(QStringLiteral("Control"));
    msg << name << delta << QVariant::fromValue(options) << ++m_seq;
    m_conn.send(msg);  // no reply expected; acknowledged by the ControlAck signal
}

void KdenliveDBusClient::invoke(const QString &command, const QVariantMap &args)
{
    if (!m_attached) {
        return;
    }
    auto msg = call(QStringLiteral("Invoke"));
    msg << command << QVariant::fromValue(args);
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(msg, 2000), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, command] {
        w->deleteLater();
        if (w->isError()) {
            Q_EMIT message(QStringLiteral("invoke %1 failed: %2").arg(command, w->error().message()));
        }
    });
}

void KdenliveDBusClient::notify(const QString &text)
{
    if (!m_attached) {
        return;
    }
    auto msg = call(QStringLiteral("Notify"));
    msg << text << 1500;
    m_conn.send(msg);
}

} // namespace cs
