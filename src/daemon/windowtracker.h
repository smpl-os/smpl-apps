// SPDX-License-Identifier: GPL-2.0-or-later
// Foreground-window tracking. Backends: Hyprland (IPC sockets), static (tests,
// simulate). KDE Plasma/KWin is a planned backend (see docs/design.md).
#pragma once

#include <QObject>
#include <QString>
#include <memory>

class QLocalSocket;
class QTimer;

namespace cs {

struct WindowInfo {
    QString cls;
    QString title;
    qint64 pid = 0;
    QString address;
    bool operator==(const WindowInfo &) const = default;
};

class WindowTracker : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    virtual void start() = 0;
    virtual QString backendName() const = 0;
    WindowInfo current() const { return m_current; }

Q_SIGNALS:
    void activeWindowChanged(const cs::WindowInfo &w);
    void message(const QString &text);

protected:
    void setCurrent(const WindowInfo &w);

private:
    WindowInfo m_current;
};

class StaticWindowTracker : public WindowTracker
{
    Q_OBJECT
public:
    using WindowTracker::WindowTracker;
    void start() override {}
    QString backendName() const override { return QStringLiteral("static"); }
    void set(const WindowInfo &w) { setCurrent(w); }
};

class HyprlandTracker : public WindowTracker
{
    Q_OBJECT
public:
    // socketDir: $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE (auto if empty)
    explicit HyprlandTracker(const QString &socketDir = {}, QObject *parent = nullptr);
    void start() override;
    QString backendName() const override { return QStringLiteral("hyprland"); }
    QString socketDir() const { return m_dir; }

    // Exposed for tests.
    void handleEventLine(const QString &line);

private:
    void connectEvents();
    void queryActive();
    QString resolveDir() const;

    QString m_dir;
    bool m_fixedDir = false;
    QLocalSocket *m_events = nullptr;
    QTimer *m_reconnect = nullptr;
    QByteArray m_buffer;
    bool m_queryInFlight = false;
    bool m_queryDirty = false;
};

std::unique_ptr<WindowTracker> createWindowTracker(const QString &backend, QObject *parent = nullptr);

} // namespace cs
