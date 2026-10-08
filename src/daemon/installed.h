// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "config.h"

#include <QString>
#include <functional>

namespace cs {

// Whether a program or application is installed: an executable on PATH (or
// an absolute path to one), or a desktop entry <name>.desktop in the XDG
// applications directories. Answers are cached for 30 s, so asking on every
// input event stays cheap and a newly installed app shows up soon.
bool isInstalled(const QString &name);
// Every name in the binding's "ifInstalled" is installed (true when it has none).
bool bindingAvailable(const Binding &b);
// Tests: replace the check (and drop the cache); nullptr restores the real one.
void setInstalledCheck(std::function<bool(const QString &)> check);

} // namespace cs
