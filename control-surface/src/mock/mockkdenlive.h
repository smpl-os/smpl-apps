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
    void setPosition(int frame);                                        // serial only; ends editing gestures
    // MR2 context as Kdenlive publishes it: focused wheel + colorWheels (three
    // explicit handles) of a Lift/Gamma/Gain widget, or a focused scalar parameter.
    void focusWheels(const QString &focusedWheel);  // "" clears
    void hoverWheel(const QString &wheel);           // "" clears
    void focusParam(const QString &name);            // "" clears
    void setGroupedPropagation(bool on) { m_grouped = on; }
    // Live grading: static/single-key parameters may be edited while playing;
    // multi-key parameters (keys at frames 0 and 100) need stopped playback.
    void setPlaying(bool on);
    void setParamMultiKey(const QString &name, bool on);
    // MR3 timeline: focused track (visual order trk-4 V2, trk-3 V1, trk-7 A1,
    // trk-8 A2) and the selected clip (clip-21/clip-22 linked A/V pair,
    // clip-31 with a multi-key volume). "" clears the selection.
    void focusTrack(const QString &trackId);
    void selectClip(const QString &clipId);
    void selectClipGroup(const QStringList &clipIds);
    void setAudioRouting(const QVariantMap &routing);
    void setTrimGestureSteps(int n) { m_trimGestureSteps = n; }  // limits.trimGestureSteps (default 128)
    // K23 MR1b-B effect and bin helpers.
    void showBin(bool on = true);
    void setBinClips(const QList<QVariantMap> &clips);
    void selectBinClips(const QStringList &clipIds);
    void showEffectStack(const QList<QVariantMap> &effects, bool compareAvailable = true);
    void hideEffectStack();
    void focusEffect(int index);
    void focusEffectParameter(const QString &name);
    // MR1a action context: an open clip-monitor source (insert/overwrite and
    // clip markers need one) and a native drag in progress (editing actions busy).
    void setSourceOpen(bool on);
    void setDragging(bool on) { m_dragging = on; }
    // Native monitor trimming preview (entered with the Slip tool here). While
    // the project monitor shows it, the seven playback actions are listed
    // disabled and refused busy, and so is nonzero shuttle; pause still works.
    void setTrimmingPreview(bool on);
    static QStringList editingActions();   // the 30 ids of the writer/drag check
    static QStringList playbackActions();  // the 7 ids the trimming preview disables
    static QStringList candidateActions();  // the 71 MR1a candidate ids
    void announceActionsChanged();           // test helper: ActionsChanged to every subscriber now
    static QString wheelTarget(const QString &wheel) { return QStringLiteral("cw-") + wheel; }
    QVariantMap context() const { return m_context; }
    QVariantMap state() const;
    void setApplyDelayMs(int ms) { m_applyDelayMs = ms; }
    // Test hook: ContextChanged goes out this late (0 = only the 30 Hz limit),
    // so a client that sends the epoch of an outdated context fails every time.
    void setContextLagMs(int ms) { m_contextLagMs = ms; }
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
        QString owner, id, control, target;
        // Subject captured at gesture start: parameter name, wheel, "track:<id>" /
        // "clip:<id>" (gain) or the comma-joined clips of a trim scope.
        QString param;
        QVariantMap semantic;
        QVariant start;
        int frame = -1;  // captured edit frame of a multi-key parameter; -1: whole clip
        int steps = 0;   // applied batches (a trim retains each one)
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
    void emitContext(bool deferred);
    void bumpSerial(bool epoch);
    void dispatchActions();
    void record(const QString &line);
    QVariantMap wheelDescriptor(const QString &wheel) const;
    QVariantMap paramDescriptor(const QString &name) const;
    QVariantMap findWheelTarget(const QString &target) const;  // descriptor or empty
    void refreshDescriptors();
    QVariant gestureValue(const Gesture &g) const;
    void restoreGestureValue(const Gesture &g, const QVariant &v);
    QVariantMap preflightEdit(const Pending &p, int *frame) const;  // {code, message} or empty
    QString gestureSubject(const Pending &p) const;
    QString gainTrack(const QString &target, QString *clip = nullptr) const;  // owning track of a gain handle
    QString panTrack(const QString &target) const;
    QVariantMap timelineDescriptor() const;
    QVariantMap effectDescriptor() const;
    QVariantMap binDescriptor() const;
    QVariantMap effectParamDescriptor(const QString &name, int effectIndex) const;
    QVariantMap findParamTarget(const QString &target) const;
    QStringList selectedTrimClips() const;
    QStringList rippleScopeClips() const;
    bool rippleAvailable() const;
    QStringList nudgeClips() const;
    QStringList visibleBinClipIds() const;
    QString binSelectionTarget() const;
    bool validBinSelection() const;
    QStringList binTagIds() const;
    int effectIndexForTarget(const QString &target) const;
    void finishFrameBoundGestures();
    QVariantMap stateCheck() const;  // ready/closing/active/modal, at admission and at dispatch
    // MR1a: host context restrictions (no caller: the descriptor's "enabled"),
    // plus caller-specific writer ownership and drag when a caller is given.
    QVariantMap actionRefusal(const QString &id, const Caller *caller) const;
    void applyAction(const QString &id);
    void scheduleActionsCheck();  // ActionsChanged when ids or enabled/checked change
    QList<QVariantMap> actionList() const;
    void checkGestureIdle();

    int m_stage = 3;
    bool m_grouped = false;
    QVariantMap m_context;
    quint64 m_serial = 0;
    quint64 m_epoch = 1;
    QHash<QString, Lease> m_leases;  // owner -> lease
    quint64 m_sessionCounter = 0;
    QHash<QString, Pending> m_pending;
    QStringList m_pendingOrder;
    bool m_applyScheduled = false;
    int m_applyDelayMs = 0;
    int m_contextLagMs = 0;
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
    QVariantMap m_wheels;  // lift/gamma/gain -> {r,g,b}
    QVariantMap m_params;  // name -> static value
    QHash<QString, QMap<int, double>> m_keys;  // multi-key parameter -> frame -> value
    QStringList m_paramOrder;
    QStringList m_triggered;
    int m_track = 2;  // index into m_trackOrder
    QStringList m_trackOrder;
    QVariantMap m_tracks;  // id -> {id, type, label, mute, hide, lock, solo, target, gainDb}
    QVariantMap m_clips;   // id -> {track, start, end, minStart, maxEnd, linked, audio, volumeDb, staticVolume}
    QString m_selectedClip;
    QStringList m_nudgeSelection;
    QString m_videoTarget;
    QVariantMap m_audioTargets;  // physical stream id -> track id
    int m_trimGestureSteps = 128;
    bool m_binVisible = false;
    QVariantList m_binClips;
    QStringList m_binSelection;
    int m_binCursor = 0;
    QStringList m_binFilterTags;
    QVariantList m_binFilterRatings;
    QVariantList m_binFilterTypes;
    int m_binFilterUsage = 0;
    QVariantList m_effectStack;
    bool m_effectStackShown = false;
    bool m_effectStackEnabled = true;
    bool m_effectCompare = false;
    bool m_effectCompareAvailable = true;
    int m_focusedEffect = -1;
    QString m_focusedEffectParam;
    bool m_sourceOpen = true;
    bool m_dragging = false;
    bool m_trimming = false;
    bool playbackBlocked(const QString &monitor) const;  // trimming preview on the (resolved) project monitor
    QString m_editMode = QStringLiteral("normal");
    int m_zoneIn = 0, m_zoneOut = 0;
    QStringList m_redo;
    QList<QVariantMap> m_actionsSent;  // ListActions result last announced
    bool m_actionsCheckScheduled = false;

    friend class MockControlSurfaceAdaptor;
};

} // namespace cs
