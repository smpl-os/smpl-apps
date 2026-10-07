// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "kdenliveclient.h"

#include <QDBusConnection>
#include <QElapsedTimer>
#include <QHash>
#include <map>

class QDBusServiceWatcher;

namespace cs {

// Structured reply of the contract: {ok, result} or {ok:false, error:{code,message,field}}.
struct Envelope {
    bool valid = false;  // was an envelope at all
    bool ok = false;
    QString session;     // ControlAck outcomes carry the lease, also on errors
    QVariantMap result;
    QString code, message, field;
    static Envelope parse(const QVariant &v);
};

// Talks to org.kde.kdenlive.ControlSurface1 (revision 2) on org.kde.kdenlive-<pid>
// (session bus) or on a peer-to-peer connection (tests).
class KdenliveDBusClient : public KdenliveClient
{
    Q_OBJECT
public:
    explicit KdenliveDBusClient(const QDBusConnection &connection, QObject *parent = nullptr);
    ~KdenliveDBusClient() override;

    // Fixed target, ignoring window pids (mock testing). Empty + peer connection = the peer.
    void setServiceOverride(const QString &service);
    void attachToService(const QString &service);

    State state() const override { return m_state; }
    QVariantMap context() const override { return m_context; }
    quint64 epoch() const override { return m_epoch; }
    void attachToPid(qint64 pid) override;
    qint64 attachedPid() const override { return m_pid; }
    void retry() override;
    bool supportsControl(const QString &name) const override { return m_controls.contains(name); }
    bool supportsAction(const QString &id) const override { return m_actions.contains(id); }
    bool supportsCommand(const QString &name) const override { return m_commands.contains(name); }
    void triggerAction(const QString &id) override;
    bool control(const QString &key, const QString &name, double delta, const QVariantMap &options) override;
    void invoke(const QString &command, const QVariantMap &args) override;
    void notify(const QString &text) override;

    QString session() const { return m_session; }
    QVariantMap capabilities() const { return m_caps; }
    int inFlightMessages() const { return int(m_sent.size()); }
    static QVariant normalize(const QVariant &v);  // QDBusArgument trees -> plain QVariant

private Q_SLOTS:
    void onContextChanged(const QVariantMap &context);
    void onControlAck(qulonglong seq, const QString &control, const QVariantMap &outcome);
    void onActionFinished(qulonglong requestId, const QVariantMap &outcome);
    void onActionsChanged();

private:
    struct Sent {
        QString key, control, target, gesture;
    };
    void detach();
    void setState(State s);
    void stepCapabilities(quint64 gen);
    void stepSubscribe(quint64 gen);
    void stepListActions(quint64 gen, bool becomeAvailable);
    void handleTransportError(const QString &what, const QString &errorName, const QString &errorMessage, const QString &actionId = {});
    QDBusMessage call(const QString &method) const;
    QVariantMap commonOptions() const;

    QDBusConnection m_conn;
    QString m_service;
    bool m_attached = false;
    bool m_hasOverride = false;
    QString m_override;
    State m_state = State::Detached;
    qint64 m_pid = 0;
    QString m_session;
    QVariantMap m_context;
    quint64 m_epoch = 0;
    quint64 m_serial = 0;
    bool m_haveSerial = false;
    QVariantMap m_caps;
    QSet<QString> m_controls, m_commands, m_actions;
    quint64 m_seq = 0;  // never reset in this process: late acks of an old lease cannot collide
    std::map<quint64, Sent> m_sent;
    QHash<QString, quint64> m_latestForKey;
    QHash<quint64, QString> m_requests;  // requestId -> action id
    quint64 m_generation = 0;
    QElapsedTimer m_lastAttempt;
    QDBusServiceWatcher *m_watcher = nullptr;
};

} // namespace cs
