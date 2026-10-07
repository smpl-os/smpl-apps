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
    void daemonGone();

private Q_SLOTS:
    void onInputEvent(const QString &slot, const QString &event, int delta);
};

} // namespace cs
