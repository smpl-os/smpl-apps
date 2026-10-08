// SPDX-License-Identifier: GPL-2.0-or-later
#include "kdenlivecatalog.h"

#include <cmath>
#include "kdenlivecontract.h"

#include <QJsonArray>
#include <QRegularExpression>

namespace cs::catalog {

namespace {
struct Row {
    const char *id, *text, *shortcut;
    bool checkable;
    const char *group;
};
// K23-MR1a curated candidates, in the contract's order, then K23 MR1b-A's
// additions (k23-mr1b-a-contract.md, sha256 9e29cd2f…; ids, texts and default
// shortcuts as Kdenlive registers them). 100 fixed ids; the families
// (cameras, layouts, tags, effects) follow after the table.
const Row kRows[] = {
    {"monitor_play", "Play/Pause", "Space", false, "playback"},
    {"monitor_pause", "Pause", "K", false, "playback"},
    {"monitor_seek_backward", "Rewind", "J", false, "playback"},
    {"monitor_seek_forward", "Forward", "L", false, "playback"},
    {"monitor_play_zone", "Play Zone", "Ctrl+Space", false, "playback"},
    {"monitor_play_zone_cursor", "Play Zone From Cursor", "", false, "playback"},
    {"monitor_loop_zone", "Loop Zone", "Ctrl+Shift+Space", false, "playback"},
    {"monitor_loop_clip", "Loop Selected Clip", "", false, "playback"},
    {"switch_monitor", "Switch Monitor", "T", false, "monitor"},
    {"monitor_zoomin", "Zoom In Monitor", "", false, "monitor"},
    {"monitor_zoomout", "Zoom Out Monitor", "", false, "monitor"},
    {"monitor_zoomreset", "Reset Monitor Zoom", "", false, "monitor"},
    {"zoom_fit", "Fit Zoom to Project", "", false, "navigation"},
    {"view_zoom_in", "Zoom In", "Ctrl+=", false, "navigation"},
    {"view_zoom_out", "Zoom Out", "Ctrl+-", false, "navigation"},
    {"seek_start", "Go to Project Start", "Home", false, "navigation"},
    {"seek_end", "Go to Project End", "End", false, "navigation"},
    {"seek_clip_start", "Go to Clip Start", "", false, "navigation"},
    {"seek_clip_end", "Go to Clip End", "", false, "navigation"},
    {"seek_zone_start", "Go to Zone Start", "Shift+I", false, "navigation"},
    {"seek_zone_end", "Go to Zone End", "Shift+O", false, "navigation"},
    {"monitor_seek_snap_backward", "Go to Previous Snap Point", "Alt+Left", false, "navigation"},
    {"monitor_seek_snap_forward", "Go to Next Snap Point", "Alt+Right", false, "navigation"},
    {"monitor_seek_guide_backward", "Go to Previous Guide", "Ctrl+Left", false, "navigation"},
    {"monitor_seek_guide_forward", "Go to Next Guide", "Ctrl+Right", false, "navigation"},
    {"mark_in", "Set Zone In", "I", false, "zone"},
    {"mark_out", "Set Zone Out", "O", false, "zone"},
    {"add_marker_guide_quickly", "Add Marker/Guide quickly", "Num+*", false, "markers"},
    {"add_marker_guide_1", "Add Marker/Guide (Category 1)", "", false, "markers"},
    {"add_marker_guide_2", "Add Marker/Guide (Category 2)", "", false, "markers"},
    {"add_marker_guide_3", "Add Marker/Guide (Category 3)", "", false, "markers"},
    {"add_marker_guide_4", "Add Marker/Guide (Category 4)", "", false, "markers"},
    {"add_marker_guide_5", "Add Marker/Guide (Category 5)", "", false, "markers"},
    {"add_marker_guide_6", "Add Marker/Guide (Category 6)", "", false, "markers"},
    {"add_marker_guide_7", "Add Marker/Guide (Category 7)", "", false, "markers"},
    {"add_marker_guide_8", "Add Marker/Guide (Category 8)", "", false, "markers"},
    {"add_marker_guide_9", "Add Marker/Guide (Category 9)", "", false, "markers"},
    {"add_marker_guide_10", "Add Marker/Guide (Category 10)", "", false, "markers"},
    {"delete_clip_marker", "Delete Clip Marker", "", false, "markers"},
    {"delete_sequence_marker", "Delete Guide", "", false, "markers"},
    {"insert_to_in_point", "Insert Clip Zone in Timeline", "V", false, "editing"},
    {"overwrite_to_in_point", "Overwrite Clip Zone in Timeline", "B", false, "editing"},
    {"remove_lift", "Lift Zone", "Z", false, "editing"},
    {"remove_extract", "Extract Zone", "Shift+X", false, "editing"},
    {"cut_timeline_clip", "Cut Clip", "Shift+R", false, "editing"},
    {"cut_timeline_all_clips", "Cut All Clips", "Ctrl+Shift+R", false, "editing"},
    {"delete_timeline_clip", "Delete Selected Item", "Del", false, "editing"},
    {"extract_clip", "Extract Clip", "", false, "editing"},
    {"resize_timeline_clip_start", "Resize Item Start", "(", false, "editing"},
    {"resize_timeline_clip_end", "Resize Item End", ")", false, "editing"},
    {"delete_space", "Remove Space", "", false, "editing"},
    {"delete_space_all_tracks", "Remove Space in All Tracks", "", false, "editing"},
    {"select_tool", "Selection Tool", "S", true, "tools"},
    {"razor_tool", "Razor Tool", "X", true, "tools"},
    {"spacer_tool", "Spacer Tool", "M", true, "tools"},
    {"ripple_tool", "Ripple Tool", "", true, "tools"},
    {"slip_tool", "Slip Tool", "", true, "tools"},
    {"normal_mode", "Normal Mode", "", true, "tools"},
    {"overwrite_mode", "Overwrite Mode", "", true, "tools"},
    {"insert_mode", "Insert Mode", "", true, "tools"},
    {"select_timeline_clip", "Select Clip", "+", false, "selection"},
    {"deselect_timeline_clip", "Deselect Clip", "-", false, "selection"},
    {"select_add_timeline_clip", "Add Clip To Selection", "Alt++", false, "selection"},
    {"select_timeline_zone", "Select Zone", "", false, "selection"},
    {"select_track", "Select All in Current Track", "Shift+A", false, "selection"},
    {"select_all_tracks", "Select All", "Ctrl+A", false, "selection"},
    {"keyframe_add", "Add/Remove Keyframe", "", false, "keyframes"},
    {"keyframe_next", "Go to Next Keyframe", "", false, "keyframes"},
    {"keyframe_previous", "Go to Previous Keyframe", "", false, "keyframes"},
    {"edit_undo", "Undo", "Ctrl+Z", false, "history"},
    {"edit_redo", "Redo", "Ctrl+Shift+Z", false, "history"},
    // K23 MR1b-A
    {"mix_clip", "Mix Clips", "U", false, "clips"},
    {"group_clip", "Group Clips", "Ctrl+G", false, "clips"},
    {"ungroup_clip", "Ungroup Clips", "Ctrl+Shift+G", false, "clips"},
    {"clip_switch", "Disable Clip", "", false, "clips"},
    {"clip_split", "Restore Audio", "", false, "clips"},
    {"edit_copy", "Copy", "Ctrl+C", false, "clips"},
    {"paste_effects", "Paste Effects", "", false, "effects"},
    {"delete_effects", "Remove Effects", "", false, "effects"},
    {"master_effects", "Sequence effects", "", false, "effects"},
    {"insert_project_tree", "Insert Zone in Project Bin", "Ctrl+I", false, "bin"},
    {"clip_in_project_tree", "Clip in Project Bin", "", false, "bin"},
    {"multicam_tool", "Multicam Tool", "", true, "multicam"},
    {"perform_multitrack_mode", "Perform Multitrack Operation", "", false, "multicam"},
    {"switch_active_target", "Toggle Track Active", "A", false, "tracks"},
    {"restore_all_sources", "Restore Current Clip Target Tracks", "", false, "tracks"},
    {"fit_all_tracks", "Fit all Tracks in View", "", true, "tracks"},
    {"snap", "Snap", "", true, "tools"},
    {"sequence_next", "Switch to next Sequence", "Ctrl+Tab", false, "sequences"},
    {"sequence_previous", "Switch to previous Sequence", "Ctrl+Shift+Tab", false, "sequences"},
    {"monitor_fullscreen", "Switch Monitor Fullscreen", "F11", false, "monitor"},
    {"monitor_multitrack", "Multitrack View", "F12", true, "monitor"},
    {"monitor_overlay", "Monitor Info Overlay", "", true, "monitor"},
    {"extract_frame_to_clipboard", "Extract Frame to Clipboard", "", false, "monitor"},
    {"zoom_audio_in", "Zoom In Audio Waveforms", "", false, "audio"},
    {"zoom_audio_out", "Zoom Out Audio Waveforms", "", false, "audio"},
    {"zoom_audio_reset", "Reset Audio Waveform Zoom", "", false, "audio"},
    {"audiomixer_button", "Audio Mixer", "", true, "audio"},
    {"mlt_scrub", "Audio Scrubbing", "", true, "audio"},
    {"mlt_mute", "Mute Monitor", "", true, "audio"},
};
// Members of MR1b-A's dynamic families that every Kdenlive has: cameras 1-9
// (Multicam tool only), layout slots 1-9 (empty slots disabled) and the five
// default project tags. effect_<id> exists for every installed effect; see
// families() for the rules.
const char *const kTagNames[] = {"Red", "Green", "Blue", "Yellow", "Cyan"};
// Clarification E (k23-mr1a-mock-clarifications.md, ccb3d676): exactly these 30.
// MR1b-A adds 9 (Kdenlive's EditingActions); camera, tag and effect family
// members are editing actions too.
const char *const kEditing[] = {"mark_in", "mark_out", "add_marker_guide_quickly", "add_marker_guide_1", "add_marker_guide_2", "add_marker_guide_3",
                                "add_marker_guide_4", "add_marker_guide_5", "add_marker_guide_6", "add_marker_guide_7", "add_marker_guide_8",
                                "add_marker_guide_9", "add_marker_guide_10", "delete_clip_marker", "delete_sequence_marker", "insert_to_in_point",
                                "overwrite_to_in_point", "remove_lift", "remove_extract", "extract_clip", "cut_timeline_clip", "cut_timeline_all_clips",
                                "delete_timeline_clip", "resize_timeline_clip_start", "resize_timeline_clip_end", "delete_space",
                                "delete_space_all_tracks", "keyframe_add", "edit_undo", "edit_redo",
                                "mix_clip", "group_clip", "ungroup_clip", "clip_switch", "clip_split", "paste_effects", "delete_effects",
                                "insert_project_tree", "perform_multitrack_mode"};
// Clarification F: the playback actions the trimming preview disables.
const char *const kPlayback[] = {"monitor_play", "monitor_play_zone", "monitor_play_zone_cursor", "monitor_loop_zone",
                                 "monitor_loop_clip", "monitor_seek_backward", "monitor_seek_forward"};

template<std::size_t N>
QStringList toList(const char *const (&a)[N])
{
    QStringList l;
    for (const char *s : a) {
        l << QString::fromLatin1(s);
    }
    return l;
}
} // namespace

QStringList editingActionIds()
{
    return toList(kEditing);
}

QString actionFamily(const QString &id)
{
    static const QRegularExpression camera(QStringLiteral("^activate_video_[1-9]$")), layout(QStringLiteral("^load_layout[1-9]$")),
        tag(QStringLiteral("^tag_[1-9][0-9]{0,2}$")), effect(QStringLiteral("^effect_[A-Za-z0-9_.-]+$"));
    if (camera.match(id).hasMatch()) {
        return QStringLiteral("camera");
    }
    if (layout.match(id).hasMatch()) {
        return QStringLiteral("layout");
    }
    if (tag.match(id).hasMatch()) {
        return QStringLiteral("tag");
    }
    if (effect.match(id).hasMatch() && id.size() <= 128) {
        return QStringLiteral("effect");
    }
    return {};
}

bool isEditingAction(const QString &id)
{
    const QString family = actionFamily(id);
    return editingActionIds().contains(id) || family == QLatin1String("camera") || family == QLatin1String("tag") || family == QLatin1String("effect");
}

bool isOffered(const QString &id)
{
    if (!actionFamily(id).isEmpty()) {
        return true;
    }
    for (const Action &a : actions()) {
        if (a.id == id) {
            return true;
        }
    }
    return false;
}

QStringList excludedActionIds()
{
    // MR1b-A, explicitly: downstream dialogs (send_sequence, add_sequence_marker),
    // no native undo (disable_timeline_effects); recording, replacement and file
    // extraction stay out.
    return {QStringLiteral("send_sequence"), QStringLiteral("add_sequence_marker"), QStringLiteral("disable_timeline_effects"),
            QStringLiteral("audio_record")};
}

QJsonArray families()
{
    auto f = [](const char *id, const char *pattern, const char *members, bool editing, const char *rule) {
        return QJsonObject{{QStringLiteral("family"), QLatin1String(id)},
                           {QStringLiteral("pattern"), QLatin1String(pattern)},
                           {QStringLiteral("members"), QLatin1String(members)},
                           {QStringLiteral("editing"), editing},
                           {QStringLiteral("rule"), QLatin1String(rule)}};
    };
    return QJsonArray{
        f("camera", "^activate_video_[1-9]$", "activate_video_1..9", true,
          "only in the Multicam tool; the camera must exist (no clamping to another); stopped: selects an angle, during a multicam interval: cuts"),
        f("layout", "^load_layout[1-9]$", "load_layout1..9", false, "registered layout slots only; empty slots stay disabled; no save/manage dialogs"),
        f("tag", "^tag_[1-9][0-9]{0,2}$", "tag_<n> for the project's tags (default 5: Red, Green, Blue, Yellow, Cyan)", true,
          "the action's colour must match the project's tag n; exactly one bin clip selected"),
        f("effect", "^effect_<id>$", "effect_<id> for every installed effect (the id as in Kdenlive's effect list)", true,
          "a compatible, unlocked owner (timeline clip or track); from the bin exactly one selected clip"),
    };
}

QStringList playbackActionIds()
{
    return toList(kPlayback);
}

const QList<Action> &actions()
{
    static const QList<Action> list = [] {
        const QStringList editing = editingActionIds(), playback = playbackActionIds();
        QList<Action> l;
        for (const Row &r : kRows) {
            const QString id = QString::fromLatin1(r.id);
            l << Action{id, QString::fromUtf8(r.text), QString::fromLatin1(r.shortcut), r.checkable, QString::fromLatin1(r.group), editing.contains(id), playback.contains(id), QString()};
        }
        for (int i = 1; i <= 9; ++i) {
            // Native text "Select Video Track N" (shortcut N): in the Multicam tool it picks camera N.
            l << Action{QStringLiteral("activate_video_%1").arg(i), QStringLiteral("Camera %1").arg(i), QString::number(i), false, QStringLiteral("multicam"), true, false, QStringLiteral("camera")};
        }
        for (int i = 1; i <= 9; ++i) {
            l << Action{QStringLiteral("load_layout%1").arg(i), QStringLiteral("Load Layout %1").arg(i), QString(), false, QStringLiteral("layouts"), false, false, QStringLiteral("layout")};
        }
        for (int i = 1; i <= 5; ++i) {
            l << Action{QStringLiteral("tag_%1").arg(i), QStringLiteral("Tag %1 (%2)").arg(i).arg(QLatin1String(kTagNames[i - 1])), QString(), false, QStringLiteral("bin"), true, false, QStringLiteral("tag")};
        }
        return l;
    }();
    return list;
}

const QList<Control> &controls()
{
    using namespace cs::contract;
    const QString t = QStringLiteral("target"), g = QStringLiteral("gesture"), ph = QStringLiteral("phase");
    static const QList<Control> list{
        {kJog, QStringLiteral("MR1"), QStringLiteral("frame"), false, QStringLiteral("Move the playhead frame by frame"), {QStringLiteral("monitor"), QStringLiteral("scrub")}},
        {kShuttle, QStringLiteral("MR1"), QStringLiteral("speed step (-7..7, 0 pauses)"), false, QStringLiteral("Shuttle playback speed"), {QStringLiteral("monitor")}},
        {kZoom, QStringLiteral("MR1"), QStringLiteral("zoom step (+ zooms in)"), false, QStringLiteral("Timeline zoom"), {QStringLiteral("anchor")}},
        {kParamFocus, QStringLiteral("MR2"), QStringLiteral("parameter"), false, QStringLiteral("Focus the next or previous effect parameter"), {}},
        {kParamNudge, QStringLiteral("MR2"), QStringLiteral("parameter step"), true, QStringLiteral("Change the focused effect parameter (or any effect.params.<name>.target)"),
         {t, g, ph, QStringLiteral("step"), QStringLiteral("keyframe")}},
        {kColorWheel, QStringLiteral("MR2"), QStringLiteral("wheel step"), true, QStringLiteral("Lift/Gamma/Gain wheel value or R/G/B"),
         {t, g, ph, QStringLiteral("step"), QStringLiteral("keyframe"), QStringLiteral("wheel"), QStringLiteral("axis")}},
        {kTrackFocus, QStringLiteral("MR3"), QStringLiteral("track"), false, QStringLiteral("Move the track focus up or down"), {}},
        {kScroll, QStringLiteral("MR3"), QStringLiteral("tenth of the visible width"), false, QStringLiteral("Scroll the timeline"), {}},
        {kAudioGain, QStringLiteral("MR3"), QStringLiteral("0.1 dB"), true, QStringLiteral("Track or clip gain"), {t, g, ph}},
        {kTrim, QStringLiteral("MR3"), QStringLiteral("frame"), true,
         QStringLiteral("Trim the selected clip: mode resize (MR3); slip and ripple from MR1b-B, as advertised in timeline.trim.modes"),
         {t, g, ph, QStringLiteral("edge"), QStringLiteral("mode")}},
        {kTimelineTarget, QStringLiteral("MR1b-B"), QStringLiteral("track"), false,
         QStringLiteral("Move the video target track or the lowest assigned audio stream's target (kind video|audio)"), {QStringLiteral("kind")}},
        {kPan, QStringLiteral("MR1b-B"), QStringLiteral("pan unit (-50..50)"), true, QStringLiteral("Pan of the active mixer's audio track"), {t, g, ph}},
        {kNudge, QStringLiteral("MR1b-B"), QStringLiteral("frame (unit second: one second)"), true,
         QStringLiteral("Move the selected clips on their tracks; refuses collisions"), {t, g, ph, QStringLiteral("unit")}},
        {kEffectFocus, QStringLiteral("MR1b-B"), QStringLiteral("effect"), false, QStringLiteral("Focus the next or previous effect of the shown stack"), {}},
        {kBinCursor, QStringLiteral("MR1b-B"), QStringLiteral("row"), false, QStringLiteral("Move the bin cursor through the visible rows (extend: grow the selection)"),
         {QStringLiteral("extend")}},
        {kBinRating, QStringLiteral("MR1b-B"), QStringLiteral("star (0..5)"), true, QStringLiteral("Rating of the selected bin clips"), {t, g, ph}},
    };
    return list;
}

const QList<Command> &commands()
{
    using namespace cs::contract;
    const QString t = QStringLiteral("target"), w = QStringLiteral("what"), v = QStringLiteral("value");
    static const QList<Command> list{
        {kCmdParamReset, QStringLiteral("MR2"), QStringLiteral("Reset the focused parameter"), {t, QStringLiteral("keyframe")}},
        {kCmdWheelReset, QStringLiteral("MR2"), QStringLiteral("Reset a colour wheel"), {t, QStringLiteral("wheel"), QStringLiteral("keyframe")}},
        {kCmdTrackSet, QStringLiteral("MR3"), QStringLiteral("Set a track's mute, hide, lock, solo or target"), {t, w, v, QStringLiteral("soloMode")}},
        {kCmdEffectAdd, QStringLiteral("MR1b-B"), QStringLiteral("Add an effect (id) or a registered or saved preset (preset) to the shown stack"),
         {t, QStringLiteral("id"), QStringLiteral("preset")}},
        {kCmdEffectSet, QStringLiteral("MR1b-B"), QStringLiteral("Enable or disable an effect (what: enabled)"), {t, w, v}},
        {kCmdEffectMove, QStringLiteral("MR1b-B"), QStringLiteral("Move an effect up or down the stack (delta rows, |delta| <= 64)"), {t, QStringLiteral("delta")}},
        {kCmdEffectRemove, QStringLiteral("MR1b-B"), QStringLiteral("Remove an effect (not built-ins)"), {t}},
        {kCmdStackSet, QStringLiteral("MR1b-B"), QStringLiteral("Enable or bypass the whole stack, or the compare split view (what: enabled|compare)"), {t, w, v}},
        {kCmdBinTag, QStringLiteral("MR1b-B"), QStringLiteral("Set or clear a tag colour (an id from bin.tags) on the selected bin clips"), {t, QStringLiteral("tag"), v}},
        {kCmdBinSelect, QStringLiteral("MR1b-B"), QStringLiteral("Select the visible bin clips with a tag"), {QStringLiteral("tag")}},
        {kCmdBinFilter, QStringLiteral("MR1b-B"), QStringLiteral("Filter the bin by tag and/or rating, or clear the filters (keeps the search text)"),
         {QStringLiteral("tag"), QStringLiteral("rating"), QStringLiteral("clear")}},
    };
    return list;
}

QStringList optionProblems(const QString &name, bool command, const QVariantMap &options)
{
    using namespace cs::contract;
    const QStringList *accepted = nullptr;
    if (command) {
        for (const Command &c : commands()) {
            if (c.name == name) {
                accepted = &c.options;
            }
        }
    } else {
        for (const Control &c : controls()) {
            if (c.name == name) {
                accepted = &c.options;
            }
        }
    }
    if (!accepted) {
        return {};
    }
    QStringList problems;
    for (auto it = options.cbegin(); it != options.cend(); ++it) {
        if (!accepted->contains(it.key()) && it.key() != kOptSession && it.key() != QLatin1String("epoch")) {
            problems << QStringLiteral("%1 does not accept option \"%2\" (accepted: %3)")
                            .arg(name, it.key(), accepted->isEmpty() ? QStringLiteral("none") : accepted->join(QStringLiteral(", ")));
        }
    }
    auto fixed = [&](const char *key, QString *out) {
        const QVariant v = options.value(QLatin1String(key));
        if (!v.isValid()) {
            return false;
        }
        *out = v.toString();
        return !(v.typeId() == QMetaType::QString && out->startsWith(QLatin1Char('$')));  // a mode or context value
    };
    auto oneOf = [&](const char *key, const QStringList &values) {
        QString v;
        if (fixed(key, &v) && !values.contains(v)) {
            problems << QStringLiteral("%1 option \"%2\" must be %3").arg(name, QLatin1String(key), values.join(QLatin1Char('|')));
        }
    };
    auto boolean = [&](const char *key) {
        const QVariant v = options.value(QLatin1String(key));
        if (v.isValid() && v.typeId() != QMetaType::Bool && !(v.typeId() == QMetaType::QString && v.toString().startsWith(QLatin1Char('$')))) {
            problems << QStringLiteral("%1 option \"%2\" must be true or false").arg(name, QLatin1String(key));
        }
    };
    if (name == kTrim) {
        // Kdenlive has no default mode; resize and ripple need an edge.
        const QString mode = options.value(QStringLiteral("mode")).toString();
        oneOf("mode", kTrimModes);
        oneOf("edge", {QStringLiteral("start"), QStringLiteral("end")});
        if (!options.contains(QStringLiteral("mode"))) {
            problems << QStringLiteral("%1 needs \"mode\": %2").arg(name, kTrimModes.join(QLatin1Char('|')));
        } else if (mode != QLatin1String("slip") && !options.contains(QStringLiteral("edge"))) {
            problems << QStringLiteral("%1 mode %2 needs \"edge\": start|end").arg(name, mode);
        }
    } else if (name == kNudge) {
        oneOf("unit", {QStringLiteral("frame"), QStringLiteral("second")});
    } else if (name == kTimelineTarget) {
        oneOf("kind", {QStringLiteral("video"), QStringLiteral("audio")});
        if (!options.contains(QStringLiteral("kind"))) {
            problems << QStringLiteral("%1 needs \"kind\": video|audio").arg(name);
        }
    } else if (name == kBinCursor) {
        boolean("extend");
    } else if (name == kCmdEffectAdd) {
        if (options.contains(QStringLiteral("id")) == options.contains(QStringLiteral("preset"))) {
            problems << QStringLiteral("%1 needs exactly one of \"id\" or \"preset\"").arg(name);
        }
    } else if (name == kCmdEffectSet) {
        oneOf("what", {QStringLiteral("enabled")});
        boolean("value");
    } else if (name == kCmdStackSet) {
        oneOf("what", {QStringLiteral("enabled"), QStringLiteral("compare")});
        boolean("value");
    } else if (name == kCmdEffectMove) {
        const QVariant d = options.value(QStringLiteral("delta"));
        const double v = d.toDouble();
        if (!d.isValid() || d.typeId() == QMetaType::Bool || (d.typeId() != QMetaType::QString && (v != std::trunc(v) || std::abs(v) > 64 || v == 0))) {
            problems << QStringLiteral("%1 needs \"delta\": a whole number of rows, -64..64").arg(name);
        }
    } else if (name == kCmdBinTag) {
        boolean("value");
        if (!options.contains(QStringLiteral("tag"))) {
            problems << QStringLiteral("%1 needs \"tag\": a colour id from bin.tags").arg(name);
        }
    } else if (name == kCmdBinSelect) {
        if (!options.contains(QStringLiteral("tag"))) {
            problems << QStringLiteral("%1 needs \"tag\": a colour id from bin.tags").arg(name);
        }
    } else if (name == kCmdBinFilter) {
        boolean("clear");
        const bool clear = options.value(QStringLiteral("clear")).toBool();
        const bool filter = options.contains(QStringLiteral("tag")) || options.contains(QStringLiteral("rating"));
        if (clear && filter) {
            problems << QStringLiteral("%1: \"clear\" cannot be combined with tag or rating").arg(name);
        } else if (!clear && !filter) {
            problems << QStringLiteral("%1 needs \"tag\", \"rating\" (0..5) or \"clear\": true").arg(name);
        }
        const QVariant r = options.value(QStringLiteral("rating"));
        const double rv = r.toDouble();
        if (r.isValid() && r.typeId() != QMetaType::QString && (r.typeId() == QMetaType::Bool || rv != std::trunc(rv) || rv < 0 || rv > 5)) {
            problems << QStringLiteral("%1 option \"rating\" is a whole number of stars, 0..5").arg(name);
        }
    }
    return problems;
}

QJsonObject toJson()
{
    QJsonArray a, c, m;
    for (const Action &x : actions()) {
        a.append(QJsonObject{{QStringLiteral("id"), x.id},
                             {QStringLiteral("text"), x.text},
                             {QStringLiteral("shortcut"), x.shortcut},
                             {QStringLiteral("checkable"), x.checkable},
                             {QStringLiteral("group"), x.group},
                             {QStringLiteral("editing"), x.editing},
                             {QStringLiteral("playback"), x.playback},
                             {QStringLiteral("family"), x.family.isEmpty() ? QJsonValue() : QJsonValue(x.family)}});
    }
    for (const Control &x : controls()) {
        c.append(QJsonObject{{QStringLiteral("name"), x.name},
                             {QStringLiteral("stage"), x.stage},
                             {QStringLiteral("unit"), x.unit},
                             {QStringLiteral("editing"), x.editing},
                             {QStringLiteral("description"), x.description},
                             {QStringLiteral("options"), QJsonArray::fromStringList(x.options)}});
    }
    for (const Command &x : commands()) {
        m.append(QJsonObject{{QStringLiteral("name"), x.name},
                             {QStringLiteral("stage"), x.stage},
                             {QStringLiteral("description"), x.description},
                             {QStringLiteral("options"), QJsonArray::fromStringList(x.options)}});
    }
    return QJsonObject{{QStringLiteral("contract"), QJsonObject{{QStringLiteral("interface"), cs::contract::kInterface},
                                                                {QStringLiteral("version"), int(cs::contract::kVersion)},
                                                                {QStringLiteral("revision"), int(cs::contract::kRevision)},
                                                                {QStringLiteral("implementation"), QStringLiteral("Kdenlive K23 MR1b-B")},
                                                                {QStringLiteral("actionsSource"), QStringLiteral("K23 MR1a (71) + MR1b-A (29): 100 fixed ids, plus the families")}}},
                       {QStringLiteral("actions"), a},
                       {QStringLiteral("families"), families()},
                       {QStringLiteral("excluded"), QJsonArray::fromStringList(excludedActionIds())},
                       {QStringLiteral("controls"), c},
                       {QStringLiteral("commands"), m}};
}

} // namespace cs::catalog
