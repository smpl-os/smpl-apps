// SPDX-License-Identifier: GPL-2.0-or-later
// Routes pad events to bindings of the focused app's profile and Kdenlive
// context layer, then to uinput keys, Kdenlive actions/controls or commands.
#pragma once

#include "coalescer.h"
#include "config.h"
#include "decoder.h"
#include "kdenliveclient.h"
#include "keysink.h"
#include "windowtracker.h"

#include <QElapsedTimer>
#include <QHash>
#include <QObject>
#include <QSet>
#include <optional>

class QTimer;

namespace cs {

class Engine : public QObject
{
    Q_OBJECT
public:
    Engine(KeySink *keys, KdenliveClient *kdenlive, QObject *parent = nullptr);

    void setConfig(const Config &cfg);
    const Config &config() const { return m_cfg; }
    void setActiveWindow(const WindowInfo &w);
    void handle(const PadEvent &e);

    struct Resolution {
        Binding binding;
        QString profile;
        QString layer;  // empty: profile base bindings
        QString slot;   // which of the requested slots matched
    };
    // Slots are tried in order at each precedence level (layer, base, next
    // profile); the first hit wins, and an explicit "none" ends the search.
    std::optional<Resolution> resolve(const QStringList &candidates) const;
    std::optional<Resolution> resolve(const QString &slot) const { return resolve(QStringList{slot}); }
    static QStringList turnSlots(const QString &control, int delta);

    const Profile *activeProfile() const { return m_profile; }
    bool kdenliveActive() const;  // interface available
    bool kdenliveStock() const;   // interface proven absent and the profile opted into keyFallback
    bool kdenliveAbsent() const;  // Kdenlive focused, interface proven absent (or no client)
    static QStringList shiftSlots(const QString &control, int delta);
    static QString absentNotice();
    QString modeValue(const QString &mode) const;
    int pendingTaps() const { return int(m_tapQueue.size()); }
    int activeGestures() const { return int(m_gestures.size()); }
    void endAllGestures(bool dropPending);

Q_SIGNALS:
    void runCommand(const QStringList &argv);
    void message(const QString &text);
    // User-facing notices worth a desktop notification (also sent as message).
    void notice(const QString &text);
    // A binding is about to run (dry-run and simulate print these).
    void dispatched(const QString &slot, const QString &binding, const QString &layer);

private:
    struct Tap {
        QString group;
        int dir = 0;
        KeyChord chord;
    };
    struct Gesture {
        int batches = 0;          // update batches sent (edit.trim: limits.trimGestureSteps)
        bool frameBound = false;  // edits a key at a captured frame: a seek ends it
        QVariant position;        // playhead when the gesture started
        QString id;
        QString key;  // coalescer key
        QString control;
        QString target;
        quint64 epoch = 0;
        QVariantMap options;
        QElapsedTimer last;
    };
    void execute(const Resolution &r, const QString &slot, double detents, bool isTurn, double accel = 1.0);
    void executeControl(const Binding &b, const QString &slot, const QString &group, int dir, double delta);
    void enqueueTaps(const QString &group, int dir, const QList<KeyChord> &chords, int count);
    void drainTap();
    // "$mode" and "$ctx:path" / "$!ctx:path" (negated) references. A context
    // reference that is absent is reported through missing (nothing is guessed).
    QVariantMap expandOptions(const QVariantMap &opts, QString *missing = nullptr) const;
    QVariantMap modeContext() const;  // current mode values, as "$mode" in layer conditions
    // Opaque target handle for an editing control/command. Colour-wheel targets
    // are per wheel: the colorWheels entry for the requested wheel, else the
    // focused colorWheel, and a targetFrom descriptor whose wheel differs from
    // the requested one yields no target (Kdenlive would refuse it).
    QString resolveTarget(const Binding &b, const QString &name, const QVariantMap &options) const;
    const QHash<QString, QStringList> *modesFor(const QString &mode, QString *owner) const;
    void onFlush(const QString &key, double delta, int merged, const QVariantMap &payload, bool isEnd);
    void endGesture(const QString &bindingId, bool dropPending);
    void checkIdleGestures();
    void say(const QString &text);
    void sayOnce(const QString &key, const QString &text);
    void dropPendingWork();
    bool kdenliveProfile() const;
    bool kdenliveAttachedToFocus() const;

    KeySink *m_keys;
    KdenliveClient *m_kd;
    Config m_cfg;
    WindowInfo m_window;
    const Profile *m_profile = nullptr;
    DeltaCoalescer m_coalescer;
    QHash<QString, Gesture> m_gestures;  // binding identity -> open editing gesture
    quint64 m_gestureCounter = 0;
    QTimer *m_gestureTimer;
    // Navigation that retargets (timeline.track, param.focus): after an ack that
    // changed something, the key's next batch waits for the new epoch, so it is
    // not sent against the context it just replaced.
    QHash<QString, quint64> m_navInFlight;  // key -> epoch when sent
    QSet<QString> m_navAwaitingEpoch;
    QTimer *m_navTimer = nullptr;
    void releaseNavigation();
    QHash<QString, int> m_modeIndex;  // "profile/mode" -> index
    QHash<QString, QElapsedTimer> m_lastTurn;
    struct Fallback {
        QList<KeyChord> keys;
        qint64 pid = 0;
        QString address;
    };
    QHash<QString, Fallback> m_actionFallback;
    QSet<QString> m_said;
    // Press+turn shift: knobs held down, presses deferred to release because a
    // shift binding exists, and holds that turned (their press is swallowed).
    QSet<QString> m_held;
    QSet<QString> m_deferredPress;
    QSet<QString> m_shiftTurned;
    void noticeAbsent(const QString &what);
    void clearHeld();
    QList<Tap> m_tapQueue;
    QTimer *m_tapTimer;
};

} // namespace cs
