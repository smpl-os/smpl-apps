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
    bool kdenliveStock() const;   // Kdenlive focused, interface proven absent: stock keys allowed
    QString modeValue(const QString &mode) const;
    int pendingTaps() const { return int(m_tapQueue.size()); }
    int activeGestures() const { return int(m_gestures.size()); }
    void endAllGestures(bool dropPending);

Q_SIGNALS:
    void runCommand(const QStringList &argv);
    void message(const QString &text);

private:
    struct Tap {
        QString group;
        int dir = 0;
        KeyChord chord;
    };
    struct Gesture {
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
    QVariantMap expandOptions(const QVariantMap &opts) const;
    QString resolveTarget(const Binding &b, const QString &name) const;
    const QHash<QString, QStringList> *modesFor(const QString &mode, QString *owner) const;
    void onFlush(const QString &key, double delta, int merged, const QVariantMap &payload, bool isEnd);
    void endGesture(const QString &bindingId, bool dropPending);
    void checkIdleGestures();
    void say(const QString &text);
    void sayOnce(const QString &key, const QString &text);
    void dropPendingWork();
    bool kdenliveProfile() const;

    KeySink *m_keys;
    KdenliveClient *m_kd;
    Config m_cfg;
    WindowInfo m_window;
    const Profile *m_profile = nullptr;
    DeltaCoalescer m_coalescer;
    QHash<QString, Gesture> m_gestures;  // binding identity -> open editing gesture
    quint64 m_gestureCounter = 0;
    QTimer *m_gestureTimer;
    QHash<QString, int> m_modeIndex;  // "profile/mode" -> index
    QHash<QString, QElapsedTimer> m_lastTurn;
    struct Fallback {
        QList<KeyChord> keys;
        qint64 pid = 0;
        QString address;
    };
    QHash<QString, Fallback> m_actionFallback;
    QSet<QString> m_said;
    QList<Tap> m_tapQueue;
    QTimer *m_tapTimer;
};

} // namespace cs
