// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QDBusConnection>
#include <QObject>

namespace cs {

// `control-surfaced monitor`: follows InputEvent from a running daemon (or the
// mock) on the session bus. Without one, main.cpp reads the pad directly.
class InputMonitor : public QObject
{
    Q_OBJECT
public:
    explicit InputMonitor(QObject *parent = nullptr) : QObject(parent) {}
    // True when org.smplos.ControlSurface is on the bus and the signal is subscribed.
    bool attach(const QDBusConnection &bus);
    // One JSON line: {"slot":"key7","event":"press","delta":0,"ms":<epoch ms>}
    static QString jsonLine(const QString &slot, const QString &event, int delta, qint64 ms);
    static QString textLine(const QString &slot, const QString &event, int delta);

Q_SIGNALS:
    void input(const QString &slot, const QString &event, int delta);
    // The daemon left the bus, or a daemon (a restarted one) took the name
    // again; the subscription stays, so its events come through as before.
    void daemonGone();
    void daemonBack();

private Q_SLOTS:
    void onInputEvent(const QString &slot, const QString &event, int delta);
};

// `control-surfaced cheatsheet --follow`: reports every cheatsheet change of a
// running daemon (shown, hidden, new content) so a desktop overlay can follow.
class CheatsheetFollower : public QObject
{
    Q_OBJECT
public:
    explicit CheatsheetFollower(QObject *parent = nullptr) : QObject(parent) {}
    bool attach(const QDBusConnection &bus);

Q_SIGNALS:
    void changed();
    void daemonGone();

private Q_SLOTS:
    void onContent(const QString &json);
    void onVisibility(bool visible);
};

} // namespace cs
