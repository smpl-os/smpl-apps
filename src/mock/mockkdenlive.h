// SPDX-License-Identifier: GPL-2.0-or-later
// Reference mock of the K23 ControlSurface1 contract, revision 2
// (docs/kdenlive-api-contract.md). It keeps a small editing state, a history
// list and per-lease bookkeeping so client behaviour can be tested over a
// real (private) session bus or a peer connection.
#pragma once

#include <QDBusAbstractAdaptor>
#include <QDBusConnection>
#include <QDBusContext>
#include <QElapsedTimer>
#include <QHash>
#include <QObject>
#include <QStringList>
#include <QVariantMap>

class QDBusServiceWatcher;
class QTimer;

namespace cs {

class MockKdenlive;

// Exports exactly the contract members (ExportAdaptors only).
class MockControlSurfaceAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.kdenlive.ControlSurface1")
public:
    explicit MockControlSurfaceAdaptor(MockKdenlive *parent);

public Q_SLOTS:
    QVariantMap Capabilities();
    QVariantMap Subscribe();
    QVariantMap Unsubscribe(const QString &session);
    QVariantMap GetContext();
    QVariantMap ListActions();
    QVariantMap TriggerAction(const QString &id, const QVariantMap &options);
    Q_NOREPLY void Control(const QString &control, double delta, const QVariantMap &options, qulonglong seq);
    QVariantMap SetControlValue(const QString &control, double value, const QVariantMap &options);
    QVariantMap Invoke(const QString &command, const QVariantMap &args);
    QVariantMap Notify(const QString &text, int timeoutMs, const QVariantMap &options);

Q_SIGNALS:
    // Declared for introspection only: the mock delivers them destination-addressed.
    void ContextChanged(const QVariantMap &context);
    void ControlAck(qulonglong seq, const QString &control, const QVariantMap &outcome);
    void ActionFinished(qulonglong requestId, const QVariantMap &outcome);
    void ActionsChanged();

private:
    MockKdenlive *m;
};

class MockKdenlive : public QObject, protected QDBusContext
{
    Q_OBJECT
public:
    explicit MockKdenlive(QObject *parent = nullptr);
    ~MockKdenlive() override;
    // Registers the object with ExportAdaptors only.
    bool registerOn(QDBusConnection connection);

    // Test/console helpers (not exported).
    void setStage(int stage);
    int stage() const { return m_stage; }
    void setContextValue(const QString &key, const QVariant &value);  // bumps epoch for target keys
    void setPosition(int frame);                                        // serial only
    QVariantMap context() const { return m_context; }
    QVariantMap state() const;
    void setApplyDelayMs(int ms) { m_applyDelayMs = ms; }
    void setPrint(bool on) { m_print = on; }
    void addUnrelatedHistory(const QString &label);
    int leaseCount() const { return int(m_leases.size()); }
    int contextSignalsTo(const QString &owner) const { return m_contextSent.value(owner); }
    int contextSignalsTotal() const;
    QList<qint64> contextEmitTimesMs() const { return m_contextTimes; }
    int controlMessages() const { return m_controlMessages; }
    int applyBatches() const { return m_applyBatches; }
    QStringList history() const { return m_history; }
    QStringList owners() const { return m_leases.keys(); }
    QStringList log;

    // Contract implementation (called by the adaptor inside the D-Bus call).
    QVariantMap capabilities() const;
    QVariantMap subscribe();
    QVariantMap unsubscribe(const QString &session);
    QVariantMap getContext() const;
    QVariantMap listActions() const;
    QVariantMap triggerAction(const QString &id, const QVariantMap &options);
    void control(const QString &control, double delta, const QVariantMap &options, quint64 seq);
    QVariantMap setControlValue(const QString &control, double value, const QVariantMap &options);
    QVariantMap invoke(const QString &command, const QVariantMap &args);
    QVariantMap notify(const QString &text, int timeoutMs, const QVariantMap &options);

private:
    struct Caller {
        QString owner;        // unique bus name, or "peer:<connection>"
        QString destination;  // unique bus name; empty on peer connections
        QString connection;   // QDBusConnection name used to answer
    };
    struct Lease {
        Caller caller;
        QString session;
        quint64 lastSeq = 0;
        QDBusServiceWatcher *watcher = nullptr;
    };
    struct Pending {
        Caller caller;
        QString session, control, gesture, target, phase;
        quint64 epoch = 0;
        QVariantMap semantic;
        double delta = 0;
        quint64 firstSeq = 0, lastSeq = 0;
    };
    struct Gesture {
        QString owner, control, target;
        QVariantMap semantic;
        QVariant start;
        int historyAtStart = 0;
        bool changed = false;
        QElapsedTimer last;
    };
    struct QueuedAction {
        Caller caller;
        QString session, id;
        quint64 epoch = 0, requestId = 0;
    };

    Caller currentCaller() const;
    Lease *leaseFor(const Caller &c);
    static QVariantMap ok(const QVariantMap &result);
    static QVariantMap fail(const QString &code, const QString &message, const QString &field = {});
    QVariantMap admit(const Caller &c, const QVariantMap &options, const QStringList &allowed, int *stringBudget) const;
    QVariantMap validateControl(const QString &control, double delta, const QVariantMap &options, QVariantMap *semantic) const;
    QStringList controlsForStage() const;
    QStringList commandsForStage() const;
    QString targetFor(const QString &control) const;
    void sendTo(const Caller &c, const QString &member, const QVariantList &args);
    void ack(const Caller &c, const QString &session, quint64 seq, const QString &control, const QVariantMap &outcome);
    void applyPending();
    QVariantMap applyOne(const Pending &p, bool *changed);
    void finishGesture(const QString &key, bool cancel, QVariantMap *error);
    void finishAllGestures();
    void invalidatePending(const QString &reason);
    void dropLease(const QString &owner);
    void emitContext(bool force);
    void bumpSerial(bool epoch);
    void dispatchActions();
    void record(const QString &line);
    QVariant gestureValue(const QString &control, const QVariantMap &semantic) const;
    void restoreGestureValue(const QString &control, const QVariant &v);
    void checkGestureIdle();

    int m_stage = 3;
    QVariantMap m_context;
    quint64 m_serial = 0;
    quint64 m_epoch = 1;
    QHash<QString, Lease> m_leases;  // owner -> lease
    quint64 m_sessionCounter = 0;
    QHash<QString, Pending> m_pending;
    QStringList m_pendingOrder;
    bool m_applyScheduled = false;
    int m_applyDelayMs = 0;
    QHash<QString, Gesture> m_gestures;
    QTimer *m_gestureTimer;
    QList<QueuedAction> m_actions;
    quint64 m_requestCounter = 0;
    QTimer *m_contextTimer;
    QElapsedTimer m_lastContextEmit;
    QElapsedTimer m_clock;
    QHash<QString, int> m_contextSent;
    QList<qint64> m_contextTimes;
    bool m_print = false;
    int m_controlMessages = 0;
    int m_applyBatches = 0;
    QStringList m_history;

    // editing state
    int m_position = 0;
    int m_duration = 25 * 600;
    int m_shuttle = 0;
    int m_zoom = 10;
    double m_scroll = 0;
    int m_track = 0;
    int m_trim = 0;
    double m_gainDb = 0;
    QVariantMap m_wheels;  // lift/gamma/gain -> {r,g,b}
    QVariantMap m_params;  // name -> value
    QStringList m_paramOrder;
    QStringList m_triggered;
    QVariantMap m_tracks;  // "trk-1" -> {mute, lock, hide, solo, target}

    friend class MockControlSurfaceAdaptor;
};

} // namespace cs
