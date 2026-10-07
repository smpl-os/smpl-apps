// SPDX-License-Identifier: GPL-2.0-or-later
// Names fixed by the K23 ControlSurface1 wire contract, revision 2
// (docs/kdenlive-api-contract.md). Shared by the daemon's client and the mock.
#pragma once

#include <QString>
#include <QStringList>

namespace cs::contract {

inline const QString kServicePrefix = QStringLiteral("org.kde.kdenlive-");  // + pid
inline const QString kPath = QStringLiteral("/ControlSurface");
inline const QString kInterface = QStringLiteral("org.kde.kdenlive.ControlSurface1");
constexpr uint kVersion = 1;
constexpr uint kRevision = 2;

// MR1: view and transport (no target/gesture).
inline const QString kJog = QStringLiteral("playhead.jog");          // integral frames
inline const QString kShuttle = QStringLiteral("playhead.shuttle");  // integral index steps, -7..7, 0 pauses
inline const QString kZoom = QStringLiteral("timeline.zoom");        // integral steps, + zooms in
// MR2: parameter and colour-wheel editing.
inline const QString kParamFocus = QStringLiteral("param.focus");      // integral, no data change
inline const QString kParamNudge = QStringLiteral("param.nudge");      // editing
inline const QString kColorWheel = QStringLiteral("colorwheel.nudge"); // editing
// MR3: timeline and audio.
inline const QString kTrackFocus = QStringLiteral("timeline.track");  // integral visual steps
inline const QString kScroll = QStringLiteral("timeline.scroll");     // tenths of visible width
inline const QString kAudioGain = QStringLiteral("audio.gain");       // editing, 0.1 dB
inline const QString kTrim = QStringLiteral("edit.trim");             // editing, integral frames

// Commands (Invoke).
inline const QString kCmdParamReset = QStringLiteral("param.reset");
inline const QString kCmdWheelReset = QStringLiteral("colorwheel.reset");
inline const QString kCmdTrackSet = QStringLiteral("track.set");

// Editing controls carry target + gesture (+ phase); the others only the common options.
inline bool isEditingControl(const QString &c)
{
    return c == kParamNudge || c == kColorWheel || c == kAudioGain || c == kTrim;
}

// Options.
inline const QString kOptSession = QStringLiteral("session");
inline const QString kOptEpoch = QStringLiteral("epoch");
inline const QString kOptTarget = QStringLiteral("target");
inline const QString kOptGesture = QStringLiteral("gesture");
inline const QString kOptPhase = QStringLiteral("phase");  // update (default) | end | cancel

// Context keys.
inline const QString kCtxSerial = QStringLiteral("serial");  // observational, ticks with the playhead
inline const QString kCtxEpoch = QStringLiteral("epoch");    // target/context generation

// Error codes.
namespace err {
inline const QString InvalidArguments = QStringLiteral("invalid_arguments");
inline const QString NotSubscribed = QStringLiteral("not_subscribed");
inline const QString StaleContext = QStringLiteral("stale_context");
inline const QString StaleSequence = QStringLiteral("stale_sequence");
inline const QString Inactive = QStringLiteral("inactive");
inline const QString Modal = QStringLiteral("modal");
inline const QString NotReady = QStringLiteral("not_ready");
inline const QString Closing = QStringLiteral("closing");
inline const QString Busy = QStringLiteral("busy");
inline const QString ResourceLimit = QStringLiteral("resource_limit");
inline const QString UnknownAction = QStringLiteral("unknown_action");
inline const QString ActionDisabled = QStringLiteral("action_disabled");
inline const QString UnsupportedControl = QStringLiteral("unsupported_control");
inline const QString UnsupportedMode = QStringLiteral("unsupported_mode");
inline const QString TargetNotFound = QStringLiteral("target_not_found");
inline const QString UnsupportedParameter = QStringLiteral("unsupported_parameter");
inline const QString KeyframeRequired = QStringLiteral("keyframe_required");
inline const QString ManagedParameter = QStringLiteral("managed_parameter");
inline const QString TrackLocked = QStringLiteral("track_locked");
inline const QString HistoryConflict = QStringLiteral("history_conflict");
inline const QString EditFailed = QStringLiteral("edit_failed");
}

// D-Bus errors that prove the interface (or Kdenlive's service) is absent: the
// only cases in which the daemon may fall back to stock keyboard shortcuts.
inline const QStringList kAbsentErrors{QStringLiteral("org.freedesktop.DBus.Error.ServiceUnknown"),
                                       QStringLiteral("org.freedesktop.DBus.Error.NameHasNoOwner"),
                                       QStringLiteral("org.freedesktop.DBus.Error.UnknownObject"),
                                       QStringLiteral("org.freedesktop.DBus.Error.UnknownInterface"),
                                       QStringLiteral("org.freedesktop.DBus.Error.UnknownMethod")};

// Default limits (servers advertise theirs in Capabilities.result.limits).
constexpr int kMaxLeases = 8;
constexpr int kMaxPendingKeys = 64;
constexpr int kMaxQueuedActions = 32;
constexpr int kMaxOptions = 32;
constexpr int kMaxStringInput = 4096;
constexpr int kMaxNotifyText = 256;
constexpr int kMinContextIntervalMs = 34;  // <= 30 Hz; 33 ms would be 30.3 Hz
constexpr int kGestureIdleMs = 600;        // host ends a gesture after this idle time

} // namespace cs::contract
