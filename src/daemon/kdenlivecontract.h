// SPDX-License-Identifier: GPL-2.0-or-later
// Names fixed by docs/kdenlive-api-contract.md (shared by client and mock).
#pragma once

#include <QString>

namespace cs::contract {

inline const QString kServicePrefix = QStringLiteral("org.kde.kdenlive-");  // + pid
inline const QString kPath = QStringLiteral("/ControlSurface");
inline const QString kInterface = QStringLiteral("org.kde.kdenlive.ControlSurface1");
constexpr int kVersion = 1;

// Continuous controls (Control(s,d,a{sv},u)); see the contract for units.
inline const QString kJog = QStringLiteral("playhead.jog");          // frames
inline const QString kShuttle = QStringLiteral("playhead.shuttle");  // shuttle steps
inline const QString kZoom = QStringLiteral("timeline.zoom");        // zoom steps
inline const QString kScroll = QStringLiteral("timeline.scroll");    // tenths of a visible page
inline const QString kTrackFocus = QStringLiteral("timeline.track");  // tracks
inline const QString kParamNudge = QStringLiteral("param.nudge");     // parameter steps
inline const QString kColorWheel = QStringLiteral("colorwheel.nudge"); // wheel steps
inline const QString kCurveNudge = QStringLiteral("automation.nudge"); // value steps of selected points
inline const QString kTrim = QStringLiteral("edit.trim");             // frames, tool-dependent
inline const QString kAudioGain = QStringLiteral("audio.gain");       // 0.1 dB steps

} // namespace cs::contract
