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

namespace {
constexpr int kCallTimeoutMs = 2000;
constexpr std::size_t kMaxSent = 512;
constexpr int kMaxRequests = 64;
constexpr int kRetryPendingMs = 2000;
constexpr int kRetryAbsentMs = 5000;
} // namespace

Envelope Envelope::parse(const QVariant &v)
{
    Envelope e;
    const QVariantMap m = KdenliveDBusClient::normalize(v).toMap();
    if (!m.contains(QStringLiteral("ok"))) {
        return e;
    }
    e.valid = true;
    e.ok = m.value(QStringLiteral("ok")).toBool();
    e.session = m.value(QStringLiteral("session")).toString();
    e.result = m.value(QStringLiteral("result")).toMap();
    const QVariantMap err = m.value(QStringLiteral("error")).toMap();
    e.code = err.value(QStringLiteral("code")).toString();
    e.message = err.value(QStringLiteral("message")).toString();
    e.field = err.value(QStringLiteral("field")).toString();
    if (!e.ok && e.code.isEmpty()) {
        e.code = QStringLiteral("unknown");
    }
    return e;
}

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

QVariantMap KdenliveDBusClient::commonOptions() const
{
    return {{contract::kOptSession, m_session}, {contract::kOptEpoch, QVariant::fromValue<qulonglong>(m_epoch)}};
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
        } else {
            retry();
        }
        return;
    }
    if (pid == m_pid && m_attached) {
        retry();  // refocus: ask again if the previous answer was not definitive
        return;
    }
    m_pid = pid;
    if (pid <= 0) {
        detach();
        return;
    }
    attachToService(contract::kServicePrefix + QString::number(pid));
}

void KdenliveDBusClient::retry()
{
    if (!m_attached || !m_lastAttempt.isValid()) {
        return;
    }
    // Pending: Kdenlive may have been busy or starting. Absent: the user may
    // have enabled the (default-off) interface since.
    if ((m_state == State::Pending && m_lastAttempt.elapsed() > kRetryPendingMs) || (m_state == State::Absent && m_lastAttempt.elapsed() > kRetryAbsentMs)) {
        attachToService(m_service);
    }
}

void KdenliveDBusClient::detach()
{
    if (!m_attached) {
        return;
    }
    if (m_conn.isConnected()) {
        if (!m_session.isEmpty()) {
            auto msg = call(QStringLiteral("Unsubscribe"));
            msg << m_session;
            m_conn.asyncCall(msg, kCallTimeoutMs);  // reply not needed
        }
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ContextChanged"), this, SLOT(onContextChanged(QVariantMap)));
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ControlAck"), this,
                          SLOT(onControlAck(qulonglong, QString, QVariantMap)));
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ActionFinished"), this,
                          SLOT(onActionFinished(qulonglong, QVariantMap)));
        m_conn.disconnect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ActionsChanged"), this, SLOT(onActionsChanged()));
    }
    if (m_watcher) {
        m_watcher->disconnect(this);
        m_watcher->deleteLater();  // detach() may run inside the watcher's own signal
        m_watcher = nullptr;
    }
    m_attached = false;
    ++m_generation;
    m_session.clear();
    m_context.clear();
    m_epoch = 0;
    m_haveSerial = false;
    m_caps.clear();
    m_controls.clear();
    m_commands.clear();
    m_actions.clear();
    m_sent.clear();
    m_latestForKey.clear();
    m_requests.clear();
    setState(State::Detached);
}

void KdenliveDBusClient::attachToService(const QString &service)
{
    detach();
    m_service = service;
    m_attached = true;
    m_lastAttempt.start();
    const quint64 gen = ++m_generation;
    setState(State::Pending);
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ContextChanged"), this, SLOT(onContextChanged(QVariantMap)));
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ControlAck"), this, SLOT(onControlAck(qulonglong, QString, QVariantMap)));
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ActionFinished"), this, SLOT(onActionFinished(qulonglong, QVariantMap)));
    m_conn.connect(m_service, contract::kPath, contract::kInterface, QStringLiteral("ActionsChanged"), this, SLOT(onActionsChanged()));
    if (!service.isEmpty()) {
        m_watcher = new QDBusServiceWatcher(service, m_conn, QDBusServiceWatcher::WatchForUnregistration, this);
        connect(m_watcher, &QDBusServiceWatcher::serviceUnregistered, this, [this] {
            Q_EMIT message(QStringLiteral("%1 went away").arg(m_service));
            detach();
            m_pid = 0;
        });
    }
    stepCapabilities(gen);
}

void KdenliveDBusClient::handleTransportError(const QString &what, const QString &errorName, const QString &errorMessage, const QString &actionId)
{
    const QString who = m_service.isEmpty() ? QStringLiteral("peer") : m_service;
    if (contract::kAbsentErrors.contains(errorName)) {
        // Proven absent: plain stock Kdenlive (the interface defaults to off).
        Q_EMIT message(QStringLiteral("%1: %2 not available (%3); stock shortcuts will be used").arg(who, contract::kInterface, errorName));
        setState(State::Absent);
        if (!actionId.isEmpty()) {
            Q_EMIT actionFailed(actionId);
        }
        return;
    }
    // Timeouts and other transport failures prove nothing: never fall back.
    Q_EMIT message(QStringLiteral("%1: %2 failed: %3 %4").arg(who, what, errorName, errorMessage));
    if (m_state != State::Available) {
        setState(State::Pending);
    }
}

void KdenliveDBusClient::stepCapabilities(quint64 gen)
{
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(call(QStringLiteral("Capabilities")), kCallTimeoutMs), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, gen] {
        w->deleteLater();
        if (gen != m_generation) {
            return;
        }
        const QDBusMessage reply = w->reply();
        if (reply.type() == QDBusMessage::ErrorMessage) {
            handleTransportError(QStringLiteral("Capabilities"), reply.errorName(), reply.errorMessage());
            return;
        }
        const Envelope e = Envelope::parse(reply.arguments().value(0));
        if (!e.valid || !e.ok) {
            Q_EMIT message(QStringLiteral("Capabilities refused: %1 %2").arg(e.code, e.message));
            setState(State::Pending);
            return;
        }
        const uint version = e.result.value(QStringLiteral("version")).toUInt();
        const uint revision = e.result.value(QStringLiteral("revision")).toUInt();
        if (version != contract::kVersion || revision < contract::kRevision) {
            // An incompatible interface is as good as none: behave like stock Kdenlive.
            Q_EMIT message(QStringLiteral("incompatible %1 version %2 revision %3; stock shortcuts will be used").arg(contract::kInterface).arg(version).arg(revision));
            setState(State::Absent);
            return;
        }
        m_caps = e.result;
        const QStringList controls = m_caps.value(QStringLiteral("controls")).toStringList();
        const QStringList commands = m_caps.value(QStringLiteral("commands")).toStringList();
        m_controls = QSet<QString>(controls.begin(), controls.end());
        m_commands = QSet<QString>(commands.begin(), commands.end());
        stepSubscribe(gen);
    });
}

void KdenliveDBusClient::stepSubscribe(quint64 gen)
{
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(call(QStringLiteral("Subscribe")), kCallTimeoutMs), this);
    const QString service = m_service;
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, gen, service] {
        w->deleteLater();
        const QDBusMessage reply = w->reply();
        if (gen != m_generation) {
            // We detached meanwhile, but the host may have granted a lease: give it
            // back, unless we are attached to the same instance again (Subscribe is
            // idempotent per sender, so that live attachment shares the session).
            const Envelope late = Envelope::parse(reply.arguments().value(0));
            const QString session = late.result.value(QStringLiteral("session")).toString();
            if (reply.type() == QDBusMessage::ReplyMessage && late.ok && !session.isEmpty() && !(m_attached && m_service == service)) {
                auto msg = QDBusMessage::createMethodCall(service, contract::kPath, contract::kInterface, QStringLiteral("Unsubscribe"));
                msg.setAutoStartService(false);
                msg << session;
                m_conn.asyncCall(msg, kCallTimeoutMs);
            }
            return;
        }
        if (reply.type() == QDBusMessage::ErrorMessage) {
            handleTransportError(QStringLiteral("Subscribe"), reply.errorName(), reply.errorMessage());
            return;
        }
        const Envelope e = Envelope::parse(reply.arguments().value(0));
        if (!e.valid || !e.ok || e.result.value(QStringLiteral("session")).toString().isEmpty()) {
            Q_EMIT refused(QStringLiteral("Subscribe"), e.code, e.message);
            setState(State::Pending);
            return;
        }
        m_session = e.result.value(QStringLiteral("session")).toString();
        m_haveSerial = false;
        onContextChanged(e.result.value(QStringLiteral("context")).toMap());
        stepListActions(gen, true);
    });
}

void KdenliveDBusClient::stepListActions(quint64 gen, bool becomeAvailable)
{
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(call(QStringLiteral("ListActions")), kCallTimeoutMs), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, gen, becomeAvailable] {
        w->deleteLater();
        if (gen != m_generation) {
            return;
        }
        const QDBusMessage reply = w->reply();
        if (reply.type() == QDBusMessage::ErrorMessage) {
            // Absent-class errors become Absent; anything else keeps the previous
            // allowlist (refresh) or stays Pending so the retry path asks again.
            handleTransportError(QStringLiteral("ListActions"), reply.errorName(), reply.errorMessage());
            return;
        }
        const Envelope e = Envelope::parse(reply.arguments().value(0));
        if (!e.ok) {
            Q_EMIT refused(QStringLiteral("ListActions"), e.code, e.message);
            if (becomeAvailable) {
                setState(State::Pending);
            }
            return;
        }
        m_actions.clear();
        for (const auto &a : e.result.value(QStringLiteral("actions")).toList()) {
            m_actions.insert(a.toMap().value(QStringLiteral("id")).toString());
        }
        if (becomeAvailable) {
            Q_EMIT message(QStringLiteral("attached to %1: %2 controls, %3 actions, %4 commands")
                               .arg(m_service.isEmpty() ? QStringLiteral("peer") : m_service)
                               .arg(m_controls.size())
                               .arg(m_actions.size())
                               .arg(m_commands.size()));
            setState(State::Available);
        }
    });
}

void KdenliveDBusClient::onActionsChanged()
{
    if (m_state == State::Available) {
        stepListActions(m_generation, false);
    }
}

void KdenliveDBusClient::setState(State s)
{
    if (m_state != s) {
        m_state = s;
        Q_EMIT stateChanged(s);
    }
}

void KdenliveDBusClient::onContextChanged(const QVariantMap &context)
{
    if (!m_attached) {
        return;
    }
    const QVariantMap ctx = normalize(context).toMap();
    const quint64 serial = ctx.value(contract::kCtxSerial).toULongLong();
    if (m_haveSerial && serial <= m_serial) {
        return;  // stale or duplicate snapshot
    }
    m_haveSerial = true;
    m_serial = serial;
    m_context = ctx;
    const quint64 epoch = ctx.value(contract::kCtxEpoch).toULongLong();
    const bool epochChangedNow = epoch != m_epoch;
    m_epoch = epoch;
    Q_EMIT contextChanged(m_context);
    if (epochChangedNow) {
        Q_EMIT epochChanged(epoch);
    }
}

bool KdenliveDBusClient::control(const QString &key, const QString &name, double delta, const QVariantMap &options)
{
    if (m_state != State::Available || m_session.isEmpty() || !m_controls.contains(name)) {
        return false;
    }
    QVariantMap opts = options;
    opts.insert(commonOptions());
    const quint64 seq = ++m_seq;
    m_sent[seq] = Sent{key, name, opts.value(contract::kOptTarget).toString(), opts.value(contract::kOptGesture).toString()};
    m_latestForKey.insert(key, seq);
    while (m_sent.size() > kMaxSent) {
        m_sent.erase(m_sent.begin());  // ancient, never-acked messages
    }
    auto msg = call(QStringLiteral("Control"));
    msg << name << delta << QVariant::fromValue(opts) << QVariant::fromValue<qulonglong>(seq);
    m_conn.send(msg);  // NoReply; the outcome arrives as a directed ControlAck
    return true;
}

void KdenliveDBusClient::onControlAck(qulonglong seq, const QString &control, const QVariantMap &outcome)
{
    // Correlate on (session, seq): a late ack of an earlier lease must never
    // release a batch of the current one, whatever its sequence number.
    const Envelope e = Envelope::parse(outcome);
    if (m_session.isEmpty() || e.session != m_session) {
        return;
    }
    const auto it = m_sent.find(seq);
    if (it == m_sent.end()) {
        return;  // unknown or duplicate: never releases anything
    }
    const Sent sent = it->second;
    if (sent.control != control) {
        Q_EMIT message(QStringLiteral("ignoring ack %1: control %2 does not match %3").arg(seq).arg(control, sent.control));
        return;
    }
    if (e.ok && !sent.target.isEmpty() && e.result.value(QStringLiteral("target")).toString() != sent.target) {
        Q_EMIT message(QStringLiteral("ignoring ack %1: target does not match").arg(seq));
        return;
    }
    // Everything up to this sequence under the same key is settled (merged batches).
    for (auto i = m_sent.begin(); i != m_sent.end() && i->first <= seq;) {
        i = i->second.key == sent.key ? m_sent.erase(i) : std::next(i);
    }
    if (!e.ok) {
        Q_EMIT refused(control, e.code, e.message);
    }
    // A late ack must not release a newer batch still in flight under this key.
    if (m_latestForKey.value(sent.key) == seq) {
        m_latestForKey.remove(sent.key);
        Q_EMIT controlAcked(sent.key, normalize(outcome).toMap());
    }
}

void KdenliveDBusClient::triggerAction(const QString &id)
{
    if (m_state != State::Available || !m_actions.contains(id)) {
        return;
    }
    auto msg = call(QStringLiteral("TriggerAction"));
    msg << id << QVariant::fromValue(commonOptions());
    const quint64 gen = m_generation;
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(msg, kCallTimeoutMs), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, id, gen] {
        w->deleteLater();
        if (gen != m_generation) {
            return;  // focus/instance changed meanwhile: never act on a stale reply
        }
        const QDBusMessage reply = w->reply();
        if (reply.type() == QDBusMessage::ErrorMessage) {
            handleTransportError(QStringLiteral("TriggerAction ") + id, reply.errorName(), reply.errorMessage(), id);
            return;
        }
        const Envelope e = Envelope::parse(reply.arguments().value(0));
        if (!e.ok) {
            Q_EMIT refused(id, e.code, e.message);  // e.g. modal, action_disabled: no keyboard fallback
            return;
        }
        const quint64 requestId = e.result.value(QStringLiteral("requestId")).toULongLong();
        if (m_requests.size() >= kMaxRequests) {
            m_requests.erase(m_requests.begin());
        }
        m_requests.insert(requestId, id);
    });
}

void KdenliveDBusClient::onActionFinished(qulonglong requestId, const QVariantMap &outcome)
{
    const QString id = m_requests.take(requestId);
    if (id.isEmpty()) {
        return;
    }
    const Envelope e = Envelope::parse(outcome);
    if (!e.ok) {
        Q_EMIT refused(id, e.code, e.message);  // refused at dispatch: still no fallback
    }
}

void KdenliveDBusClient::invoke(const QString &command, const QVariantMap &args)
{
    if (m_state != State::Available || !m_commands.contains(command)) {
        return;
    }
    QVariantMap a = args;
    a.insert(commonOptions());
    auto msg = call(QStringLiteral("Invoke"));
    msg << command << QVariant::fromValue(a);
    const quint64 gen = m_generation;
    auto *w = new QDBusPendingCallWatcher(m_conn.asyncCall(msg, kCallTimeoutMs), this);
    connect(w, &QDBusPendingCallWatcher::finished, this, [this, w, command, gen] {
        w->deleteLater();
        if (gen != m_generation) {
            return;
        }
        const QDBusMessage reply = w->reply();
        if (reply.type() == QDBusMessage::ErrorMessage) {
            handleTransportError(QStringLiteral("Invoke ") + command, reply.errorName(), reply.errorMessage());
            return;
        }
        const Envelope e = Envelope::parse(reply.arguments().value(0));
        if (!e.ok) {
            Q_EMIT refused(command, e.code, e.message);
        }
    });
}

void KdenliveDBusClient::notify(const QString &text)
{
    if (m_state != State::Available) {
        return;
    }
    auto msg = call(QStringLiteral("Notify"));
    msg << text.left(contract::kMaxNotifyText) << 1500 << QVariant::fromValue(commonOptions());
    m_conn.asyncCall(msg, kCallTimeoutMs);  // informational only
}

} // namespace cs
