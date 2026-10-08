// SPDX-License-Identifier: GPL-2.0-or-later
// (Not "features.h": that name belongs to glibc and src/daemon is on the include path.)
#pragma once

#include <QJsonObject>

namespace cs {

// What this daemon understands, for editors that build configs:
// binding kinds with examples, key/modifier/mouse names, slot grammar and
// limits, layouts, device options and the API names.
// `control-surfaced features --json` and GetFeatures() return it.
QJsonObject featuresJson();

} // namespace cs
