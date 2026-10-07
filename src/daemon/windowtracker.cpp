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
    const bool hypr = backend == QLatin1String("hyprland") || (backend == QLatin1String("auto") && hyprPresent);
    if (hypr) {
        return std::make_unique<HyprlandTracker>(QString(), parent);
    }
    // KWin: planned (KWin script reporting workspace.activeWindow over D-Bus).
    return std::make_unique<StaticWindowTracker>(parent);
}

} // namespace cs
