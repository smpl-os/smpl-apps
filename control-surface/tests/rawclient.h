// SPDX-License-Identifier: GPL-2.0-or-later
// Raw ControlSurface1 client for wire-level tests (no daemon logic).
#pragma once

#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QObject>
#include <QVariantMap>

class RawClient : public QObject
{
    Q_OBJECT
public:
    RawClient(const QDBusConnection &conn, const QString &service, QObject *parent = nullptr)
        : QObject(parent)
        , m_conn(conn)
        , m_service(service)
    {
        using namespace cs::contract;
        m_conn.connect(m_service, kPath, kInterface, QStringLiteral("ControlAck"), this, SLOT(onAck(qulonglong, QString, QVariantMap)));
        m_conn.connect(m_service, kPath, kInterface, QStringLiteral("ContextChanged"), this, SLOT(onContext(QVariantMap)));
        m_conn.connect(m_service, kPath, kInterface, QStringLiteral("ActionFinished"), this, SLOT(onFinished(qulonglong, QVariantMap)));
        m_conn.connect(m_service, kPath, kInterface, QStringLiteral("ActionsChanged"), this, SLOT(onActionsChanged()));
    }
    ~RawClient() override
    {
        using namespace cs::contract;
        m_conn.disconnect(m_service, kPath, kInterface, QStringLiteral("ControlAck"), this, SLOT(onAck(qulonglong, QString, QVariantMap)));
        m_conn.disconnect(m_service, kPath, kInterface, QStringLiteral("ContextChanged"), this, SLOT(onContext(QVariantMap)));
        m_conn.disconnect(m_service, kPath, kInterface, QStringLiteral("ActionFinished"), this, SLOT(onFinished(qulonglong, QVariantMap)));
        m_conn.disconnect(m_service, kPath, kInterface, QStringLiteral("ActionsChanged"), this, SLOT(onActionsChanged()));
    }

    QDBusMessage message(const QString &method) const
    {
        return QDBusMessage::createMethodCall(m_service, cs::contract::kPath, cs::contract::kInterface, method);
    }
    QDBusMessage rawCall(const QString &method, const QVariantList &args = {})
    {
        auto msg = message(method);
        msg.setArguments(args);
        return m_conn.call(msg, QDBus::BlockWithGui, 3000);
    }
    QVariantMap call(const QString &method, const QVariantList &args = {})
    {
        const QDBusMessage r = rawCall(method, args);
        if (r.type() != QDBusMessage::ReplyMessage) {
            return {{QStringLiteral("dbusError"), r.errorName()}};
        }
        return cs::KdenliveDBusClient::normalize(r.arguments().value(0)).toMap();
    }
    QVariantMap subscribe()
    {
        const QVariantMap e = call(QStringLiteral("Subscribe"));
        session = e.value(QStringLiteral("result")).toMap().value(QStringLiteral("session")).toString();
        context = e.value(QStringLiteral("result")).toMap().value(QStringLiteral("context")).toMap();
        return e;
    }
    quint64 epoch() const { return context.value(QStringLiteral("epoch")).toULongLong(); }
    QVariantMap common() const
    {
        return {{QStringLiteral("session"), session}, {QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(epoch())}};
    }
    void control(const QString &control, double delta, const QVariantMap &extra, quint64 seq)
    {
        QVariantMap opts = common();
        opts.insert(extra);
        auto msg = message(QStringLiteral("Control"));
        msg << control << delta << QVariant::fromValue(opts) << QVariant::fromValue<qulonglong>(seq);
        m_conn.send(msg);
    }
    QVariantMap ackFor(quint64 seq) const
    {
        for (const auto &a : acks) {
            if (a.value(QStringLiteral("seq")).toULongLong() == seq) {
                return a;
            }
        }
        return {};
    }
    static QString code(const QVariantMap &envelope)
    {
        return envelope.value(QStringLiteral("error")).toMap().value(QStringLiteral("code")).toString();
    }

    QString session;
    QVariantMap context;
    QList<QVariantMap> acks;      // {seq, control, outcome}
    QList<QVariantMap> finished;  // {requestId, outcome}
    int contexts = 0;
    int actionsChanged = 0;
    // ListActions as {id -> descriptor}.
    QVariantMap actionMap()
    {
        QVariantMap out;
        for (const auto &a : call(QStringLiteral("ListActions")).value(QStringLiteral("result")).toMap().value(QStringLiteral("actions")).toList()) {
            out.insert(a.toMap().value(QStringLiteral("id")).toString(), a);
        }
        return out;
    }

public Q_SLOTS:
    void onAck(qulonglong seq, const QString &control, const QVariantMap &outcome)
    {
        acks << QVariantMap{{QStringLiteral("seq"), QVariant::fromValue<qulonglong>(seq)},
                            {QStringLiteral("control"), control},
                            {QStringLiteral("outcome"), cs::KdenliveDBusClient::normalize(outcome)}};
    }
    void onContext(const QVariantMap &ctx)
    {
        ++contexts;
        context = cs::KdenliveDBusClient::normalize(ctx).toMap();
    }
    void onActionsChanged() { ++actionsChanged; }
    void onFinished(qulonglong requestId, const QVariantMap &outcome)
    {
        finished << QVariantMap{{QStringLiteral("requestId"), QVariant::fromValue<qulonglong>(requestId)},
                                {QStringLiteral("outcome"), cs::KdenliveDBusClient::normalize(outcome)}};
    }

private:
    QDBusConnection m_conn;
    QString m_service;
};
