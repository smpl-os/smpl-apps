// SPDX-License-Identifier: GPL-2.0-or-later
#include "windowtracker.h"

#include <QDir>

namespace cs {

void WindowTracker::setCurrent(const WindowInfo &w)
{
    if (w == m_current) {
        return;
    }
    m_current = w;
    Q_EMIT activeWindowChanged(w);
}

std::unique_ptr<WindowTracker> createWindowTracker(const QString &backend, QObject *parent)
{
    const bool hyprPresent = !qEnvironmentVariableIsEmpty("HYPRLAND_INSTANCE_SIGNATURE")
        || !QDir(qEnvironmentVariable("XDG_RUNTIME_DIR") + QStringLiteral("/hypr")).isEmpty(QDir::Dirs | QDir::NoDotAndDotDot);
    // Started before the compositor (udev, an early systemd unit): nothing says
    // which desktop this is yet. Hyprland's tracker then waits and connects once
    // a Hyprland instance appears; a desktop that names itself is left alone.
    const QString desktop = qEnvironmentVariable("XDG_CURRENT_DESKTOP");
    const bool undecided = desktop.isEmpty() || desktop.contains(QLatin1String("Hyprland"), Qt::CaseInsensitive);
    const bool hypr = backend == QLatin1String("hyprland") || (backend == QLatin1String("auto") && (hyprPresent || undecided));
    if (hypr) {
        return std::make_unique<HyprlandTracker>(QString(), parent);
    }
    // KWin: planned (KWin script reporting workspace.activeWindow over D-Bus).
    return std::make_unique<StaticWindowTracker>(parent);
}

} // namespace cs
