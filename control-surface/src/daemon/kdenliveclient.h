// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QObject>
#include <QSet>
#include <QStringList>
#include <QVariantMap>

namespace cs {

// Client side of the K23 ControlSurface1 contract (docs/kdenlive-api-contract.md,
// revision 2). All calls are asynchronous; the daemon never blocks on Kdenlive.
class KdenliveClient : public QObject
{
    Q_OBJECT
public:
    enum class State {
        Detached,   // no Kdenlive window focused
        Pending,    // asked, no definite answer yet, or a timeout: never type fallback keys
        Absent,     // no service, or the interface is off (its default): plain stock Kdenlive
        Available,  // subscribed; capabilities and action allowlist known
    };
    Q_ENUM(State)

    using QObject::QObject;
    virtual State state() const = 0;
    bool isAvailable() const { return state() == State::Available; }
    bool isAbsent() const { return state() == State::Absent; }
    virtual QVariantMap context() const = 0;
    virtual quint64 epoch() const = 0;
    virtual void attachToPid(qint64 pid) = 0;  // 0 detaches
    virtual qint64 attachedPid() const = 0;
    virtual void retry() = 0;  // re-ask an instance that has not answered definitively

    // Staged capabilities advertised by Capabilities/ListActions. Names that are
    // not advertised are never sent.
    virtual bool supportsControl(const QString &name) const = 0;
    virtual bool supportsAction(const QString &id) const = 0;
    virtual bool supportsCommand(const QString &name) const = 0;
    // The host's current ListActions "enabled" (informational; the host
    // revalidates every TriggerAction). Unknown ids are reported enabled.
    virtual bool actionEnabled(const QString &id) const
    {
        Q_UNUSED(id)
        return true;
    }
    // Capabilities.limits entry (e.g. "trimGestureSteps"), or fallback.
    virtual int limit(const QString &name, int fallback) const
    {
        Q_UNUSED(name)
        return fallback;
    }

    virtual void triggerAction(const QString &id) = 0;
    // key is the caller's correlation key. The acknowledgement for the newest
    // message sent under that key comes back as controlAcked(key, outcome).
    // Returns false if nothing was sent.
    virtual bool control(const QString &key, const QString &name, double delta, const QVariantMap &options) = 0;
    virtual void invoke(const QString &command, const QVariantMap &args) = 0;
    virtual void notify(const QString &text) = 0;

Q_SIGNALS:
    void stateChanged(cs::KdenliveClient::State state);
    void contextChanged(const QVariantMap &context);
    void epochChanged(quint64 epoch);
    void controlAcked(const QString &key, const QVariantMap &outcome);
    // Only when the interface turned out to be absent: stock keys are allowed.
    void actionFailed(const QString &id);
    // Domain refusal (structured error code): report it, never fall back to keys.
    void refused(const QString &what, const QString &code, const QString &message);
    void message(const QString &text);
};

// Records calls; used by tests, --dry-run and --simulate.
class FakeKdenliveClient : public KdenliveClient
{
    Q_OBJECT
public:
    explicit FakeKdenliveClient(bool print = false, QObject *parent = nullptr);
    State state() const override { return m_state; }
    QVariantMap context() const override { return m_context; }
    quint64 epoch() const override { return m_context.value(QStringLiteral("epoch")).toULongLong(); }
    void attachToPid(qint64 pid) override;
    qint64 attachedPid() const override { return m_pid; }
    void retry() override { ++retries; }
    bool supportsControl(const QString &name) const override { return !m_restricted || m_controls.contains(name); }
    bool supportsAction(const QString &id) const override { return !m_restrictedActions || m_actions.contains(id); }
    bool supportsCommand(const QString &name) const override { return !m_restricted || m_commands.contains(name); }
    bool actionEnabled(const QString &id) const override { return !disabledActions.contains(id); }
    int limit(const QString &name, int fallback) const override { return m_limits.value(name, fallback).toInt(); }
    void setLimits(const QVariantMap &limits) { m_limits = limits; }
    void triggerAction(const QString &id) override;
    bool control(const QString &key, const QString &name, double delta, const QVariantMap &options) override;
    void invoke(const QString &command, const QVariantMap &args) override;
    void notify(const QString &text) override;

    void setState(State s);
    void setContext(const QVariantMap &ctx);
    void setAutoAck(bool on) { m_autoAck = on; }
    void setCapabilities(const QStringList &controls, const QStringList &actions, const QStringList &commands);
    void setControlCapabilities(const QStringList &controls, const QStringList &commands);  // actions stay unrestricted
    void ackAll(const QVariantMap &outcome = {{QStringLiteral("ok"), true}});
    QStringList unackedKeys() const { return m_unacked; }

    QStringList calls;
    QList<QVariantMap> invokeArgs;
    QList<double> controlDeltas;
    QList<QVariantMap> controlOptions;
    QStringList controlKeys;
    QStringList disabledActions;
    int retries = 0;

private:
    void record(const QString &line);
    bool m_print;
    State m_state = State::Available;
    bool m_autoAck = true;
    bool m_restricted = false;
    bool m_restrictedActions = false;
    qint64 m_pid = 0;
    QVariantMap m_context;
    QStringList m_unacked;
    QSet<QString> m_controls, m_actions, m_commands;
    QVariantMap m_limits;
};

} // namespace cs
