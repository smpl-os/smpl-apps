// SPDX-License-Identifier: GPL-2.0-or-later
#include "bindinglabel.h"
#include "kdenlivecatalog.h"
#include "kdenlivecontract.h"

#include <QFileInfo>
#include <QHash>
#include <QSet>
#include <linux/input-event-codes.h>

namespace cs {

namespace {
QString optionValue(const QVariant &v, const LabelEnv &env)
{
    const QString s = v.toString();
    if (s.startsWith(QLatin1String("$ctx:"))) {
        return valueAtPath(env.context, s.mid(5)).toString();
    }
    if (s.startsWith(QLatin1String("$!ctx:"))) {
        return QString();  // a toggle: the label says what, not which way
    }
    if (s.startsWith(QLatin1Char('$')) && env.modeValue) {
        return env.modeValue(s.mid(1));
    }
    return s;
}

QString opt(const Binding &b, const char *key, const LabelEnv &env)
{
    return optionValue(b.options.value(QLatin1String(key)), env);
}

QString capitalized(const QString &s)
{
    return s.isEmpty() ? s : s.left(1).toUpper() + s.mid(1);
}

QString axisName(const QString &axis)
{
    if (axis == QLatin1String("r") || axis == QLatin1String("g") || axis == QLatin1String("b")) {
        return axis.toUpper();
    }
    if (axis == QLatin1String("value") || axis.isEmpty()) {
        return {};
    }
    return prettyIdentifier(axis).toLower();
}
} // namespace

QString prettyIdentifier(const QString &id)
{
    QString out;
    for (int i = 0; i < id.size(); ++i) {
        const QChar c = id.at(i);
        if (c == QLatin1Char('_') || c == QLatin1Char('.') || c == QLatin1Char('-')) {
            if (!out.endsWith(QLatin1Char(' '))) {
                out += QLatin1Char(' ');
            }
        } else if (c.isUpper() && i > 0 && id.at(i - 1).isLower()) {
            out += QLatin1Char(' ');
            out += c.toLower();
        } else {
            out += c;
        }
    }
    return capitalized(out.trimmed());
}

QString prettyKeyName(int code)
{
    static const QHash<QString, QString> names{
        {QStringLiteral("ESC"), QStringLiteral("Esc")},
        {QStringLiteral("BACKSPACE"), QStringLiteral("Backspace")},
        {QStringLiteral("TAB"), QStringLiteral("Tab")},
        {QStringLiteral("ENTER"), QStringLiteral("Enter")},
        {QStringLiteral("SPACE"), QStringLiteral("Space")},
        {QStringLiteral("MINUS"), QStringLiteral("-")},
        {QStringLiteral("EQUAL"), QStringLiteral("=")},
        {QStringLiteral("LEFTBRACE"), QStringLiteral("[")},
        {QStringLiteral("RIGHTBRACE"), QStringLiteral("]")},
        {QStringLiteral("SEMICOLON"), QStringLiteral(";")},
        {QStringLiteral("APOSTROPHE"), QStringLiteral("'")},
        {QStringLiteral("GRAVE"), QStringLiteral("`")},
        {QStringLiteral("BACKSLASH"), QStringLiteral("\\")},
        {QStringLiteral("COMMA"), QStringLiteral(",")},
        {QStringLiteral("DOT"), QStringLiteral(".")},
        {QStringLiteral("SLASH"), QStringLiteral("/")},
        {QStringLiteral("LEFTCTRL"), QStringLiteral("Ctrl")},
        {QStringLiteral("RIGHTCTRL"), QStringLiteral("Right Ctrl")},
        {QStringLiteral("LEFTSHIFT"), QStringLiteral("Shift")},
        {QStringLiteral("RIGHTSHIFT"), QStringLiteral("Right Shift")},
        {QStringLiteral("LEFTALT"), QStringLiteral("Alt")},
        {QStringLiteral("RIGHTALT"), QStringLiteral("AltGr")},
        {QStringLiteral("LEFTMETA"), QStringLiteral("Super")},
        {QStringLiteral("RIGHTMETA"), QStringLiteral("Right Super")},
        {QStringLiteral("CAPSLOCK"), QStringLiteral("Caps Lock")},
        {QStringLiteral("NUMLOCK"), QStringLiteral("Num Lock")},
        {QStringLiteral("SCROLLLOCK"), QStringLiteral("Scroll Lock")},
        {QStringLiteral("KPASTERISK"), QStringLiteral("Num *")},
        {QStringLiteral("KPMINUS"), QStringLiteral("Num -")},
        {QStringLiteral("KPPLUS"), QStringLiteral("Num +")},
        {QStringLiteral("KPDOT"), QStringLiteral("Num .")},
        {QStringLiteral("KPENTER"), QStringLiteral("Num Enter")},
        {QStringLiteral("KPSLASH"), QStringLiteral("Num /")},
        {QStringLiteral("KPEQUAL"), QStringLiteral("Num =")},
        {QStringLiteral("SYSRQ"), QStringLiteral("Print")},
        {QStringLiteral("HOME"), QStringLiteral("Home")},
        {QStringLiteral("END"), QStringLiteral("End")},
        {QStringLiteral("PAGEUP"), QStringLiteral("Page Up")},
        {QStringLiteral("PAGEDOWN"), QStringLiteral("Page Down")},
        {QStringLiteral("UP"), QStringLiteral("Up")},
        {QStringLiteral("DOWN"), QStringLiteral("Down")},
        {QStringLiteral("LEFT"), QStringLiteral("Left")},
        {QStringLiteral("RIGHT"), QStringLiteral("Right")},
        {QStringLiteral("INSERT"), QStringLiteral("Insert")},
        {QStringLiteral("DELETE"), QStringLiteral("Delete")},
        {QStringLiteral("PAUSE"), QStringLiteral("Pause")},
        {QStringLiteral("COMPOSE"), QStringLiteral("Menu")},
        {QStringLiteral("MUTE"), QStringLiteral("Mute")},
        {QStringLiteral("VOLUMEDOWN"), QStringLiteral("Volume down")},
        {QStringLiteral("VOLUMEUP"), QStringLiteral("Volume up")},
        {QStringLiteral("PLAYPAUSE"), QStringLiteral("Play/Pause")},
        {QStringLiteral("NEXTSONG"), QStringLiteral("Next track")},
        {QStringLiteral("PREVIOUSSONG"), QStringLiteral("Previous track")},
        {QStringLiteral("STOPCD"), QStringLiteral("Stop")},
        {QStringLiteral("BRIGHTNESSDOWN"), QStringLiteral("Brightness down")},
        {QStringLiteral("BRIGHTNESSUP"), QStringLiteral("Brightness up")},
        {QStringLiteral("MICMUTE"), QStringLiteral("Microphone mute")},
    };
    const QString n = keyName(code);
    if (names.contains(n)) {
        return names.value(n);
    }
    if (n.startsWith(QLatin1String("KP")) && n.size() == 3 && n.at(2).isDigit()) {
        return QStringLiteral("Num ") + n.at(2);
    }
    return n;  // letters, digits, F1..F24 read fine as they are
}

QString prettyChord(const KeyChord &c)
{
    QStringList parts;
    if (c.mods & Mod::Ctrl) {
        parts << QStringLiteral("Ctrl");
    }
    if (c.mods & Mod::Shift) {
        parts << QStringLiteral("Shift");
    }
    if (c.mods & Mod::Alt) {
        parts << QStringLiteral("Alt");
    }
    if (c.mods & Mod::Meta) {
        parts << QStringLiteral("Super");
    }
    parts << prettyKeyName(c.key);
    return parts.join(QLatin1Char('+'));
}

QString bindingKindName(Binding::Kind k)
{
    switch (k) {
    case Binding::None:
        return QStringLiteral("none");
    case Binding::Keys:
        return QStringLiteral("keys");
    case Binding::Action:
        return QStringLiteral("action");
    case Binding::Control:
        return QStringLiteral("control");
    case Binding::Command:
        return QStringLiteral("command");
    case Binding::Cycle:
        return QStringLiteral("cycle");
    case Binding::Request:
        return QStringLiteral("request");
    case Binding::Mouse:
        return QStringLiteral("mouse");
    case Binding::Cheatsheet:
        return QStringLiteral("cheatsheet");
    }
    return QStringLiteral("none");
}

QString bindingLabel(const Binding &b, const LabelEnv &env)
{
    return b.label.isEmpty() ? autoLabel(b, env) : b.label;
}

QString autoLabel(const Binding &b, const LabelEnv &env)
{
    using namespace cs::contract;
    switch (b.kind) {
    case Binding::None:
        return {};
    case Binding::Keys: {
        QStringList l;
        for (const KeyChord &c : b.keys) {
            l << prettyChord(c);
        }
        return l.join(QLatin1Char(' '));
    }
    case Binding::Mouse: {
        static const QHash<QString, QString> mouse{
            {QStringLiteral("left"), QStringLiteral("Left click")},     {QStringLiteral("right"), QStringLiteral("Right click")},
            {QStringLiteral("middle"), QStringLiteral("Middle click")}, {QStringLiteral("back"), QStringLiteral("Back")},
            {QStringLiteral("forward"), QStringLiteral("Forward")},     {QStringLiteral("wheel-up"), QStringLiteral("Scroll up")},
            {QStringLiteral("wheel-down"), QStringLiteral("Scroll down")}, {QStringLiteral("wheel-left"), QStringLiteral("Scroll left")},
            {QStringLiteral("wheel-right"), QStringLiteral("Scroll right")},
        };
        return mouse.value(b.name, prettyIdentifier(b.name));
    }
    case Binding::Action:
        for (const auto &a : catalog::actions()) {
            if (a.id == b.name) {
                return a.text;
            }
        }
        if (catalog::actionFamily(b.name) == QLatin1String("effect")) {
            // effect_avfilter.gblur -> "Add gblur"; the effect's own name is Kdenlive's
            QString e = b.name.mid(7);
            e = e.section(QLatin1Char('.'), -1);
            return QStringLiteral("Add ") + prettyIdentifier(e).toLower();
        }
        if (catalog::actionFamily(b.name) == QLatin1String("tag")) {
            return QStringLiteral("Tag ") + b.name.mid(4);
        }
        return prettyIdentifier(b.name);
    case Binding::Control: {
        const QString name = b.name.startsWith(QLatin1Char('$')) && env.modeValue ? env.modeValue(b.name.mid(1)) : b.name;
        const bool fine = opt(b, "step", env) == QLatin1String("fine");
        QString label;
        if (name == kJog) {
            label = QStringLiteral("Jog");
        } else if (name == kShuttle) {
            label = QStringLiteral("Shuttle");
        } else if (name == kZoom) {
            label = QStringLiteral("Zoom");
        } else if (name == kParamFocus) {
            label = QStringLiteral("Select parameter");
        } else if (name == kParamNudge) {
            const QString param = valueAtPath(env.context, QStringLiteral("param.name")).toString();
            label = param.isEmpty() ? QStringLiteral("Parameter") : prettyIdentifier(param);
            if (opt(b, "keyframe", env) == QLatin1String("create")) {
                label += QStringLiteral(" (+key)");
            }
        } else if (name == kColorWheel) {
            const QString wheel = opt(b, "wheel", env);
            label = wheel.isEmpty() ? QStringLiteral("Color wheel") : capitalized(wheel);
            const QString axis = axisName(opt(b, "axis", env));
            if (!axis.isEmpty()) {
                label += QLatin1Char(' ') + axis;
            }
        } else if (name == kTrackFocus) {
            label = QStringLiteral("Track");
        } else if (name == kScroll) {
            label = QStringLiteral("Scroll");
        } else if (name == kAudioGain) {
            label = QStringLiteral("Gain");
        } else if (name == kTrim) {
            const QString edge = opt(b, "edge", env);
            label = edge == QLatin1String("start") ? QStringLiteral("Trim in") : edge == QLatin1String("end") ? QStringLiteral("Trim out") : QStringLiteral("Trim");
        } else {
            label = prettyIdentifier(name);
        }
        return fine ? label + QStringLiteral(" (fine)") : label;
    }
    case Binding::Request:
        if (b.name == kCmdParamReset) {
            return QStringLiteral("Reset parameter");
        }
        if (b.name == kCmdWheelReset) {
            const QString wheel = opt(b, "wheel", env);
            return wheel.isEmpty() ? QStringLiteral("Reset wheel") : QStringLiteral("Reset ") + wheel.toLower();
        }
        if (b.name == kCmdTrackSet) {
            const QString what = opt(b, "what", env);
            return what.isEmpty() ? QStringLiteral("Track") : capitalized(what) + QStringLiteral(" track");
        }
        return prettyIdentifier(b.name);
    case Binding::Cycle:
        return prettyIdentifier(b.name);
    case Binding::Command:
        return b.argv.isEmpty() ? QStringLiteral("Run") : QStringLiteral("Run ") + QFileInfo(b.argv.first()).fileName();
    case Binding::Cheatsheet:
        return QStringLiteral("Cheatsheet");
    }
    return {};
}

// ---------------------------------------------------------------------------------
// Icons (the smplOS overlay's contract: Tabler outline names)
// ---------------------------------------------------------------------------------

namespace {
const QHash<int, QString> &mediaIcons()
{
    static const QHash<int, QString> m{
        {KEY_PLAYPAUSE, QStringLiteral("player-play")},          {KEY_NEXTSONG, QStringLiteral("player-track-next")},
        {KEY_PREVIOUSSONG, QStringLiteral("player-track-prev")}, {KEY_STOPCD, QStringLiteral("player-stop")},
        {KEY_VOLUMEUP, QStringLiteral("volume")},                {KEY_VOLUMEDOWN, QStringLiteral("volume-2")},
        {KEY_MUTE, QStringLiteral("volume-3")},                  {KEY_MICMUTE, QStringLiteral("microphone-off")},
        {KEY_BRIGHTNESSUP, QStringLiteral("brightness-up")},     {KEY_BRIGHTNESSDOWN, QStringLiteral("brightness-down")},
    };
    return m;
}

const QHash<quint32, QString> &shortcutIcons()
{
    static const QHash<quint32, QString> m = [] {
        const QList<QPair<const char *, const char *>> list{
            {"ctrl+z", "arrow-back-up"},     {"ctrl+shift+z", "arrow-forward-up"}, {"ctrl+y", "arrow-forward-up"},
            {"ctrl+c", "copy"},              {"ctrl+v", "clipboard"},              {"ctrl+x", "cut"},
            {"ctrl+s", "device-floppy"},     {"ctrl+t", "square-plus"},            {"ctrl+w", "x"},
            {"ctrl+r", "refresh"},           {"f5", "refresh"},                    {"alt+left", "arrow-left"},
            {"alt+right", "arrow-right"},    {"ctrl+tab", "chevron-right"},        {"ctrl+shift+tab", "chevron-left"},
            {"ctrl+l", "link"},              {"ctrl+shift+t", "restore"},
        };
        QHash<quint32, QString> h;
        for (const auto &[chord, icon] : list) {
            if (auto c = parseChord(QLatin1String(chord))) {
                h.insert(c->id(), QLatin1String(icon));
            }
        }
        return h;
    }();
    return m;
}

const QHash<QString, QString> &mouseIcons()
{
    static const QHash<QString, QString> m{
        {QStringLiteral("left"), QStringLiteral("mouse")},          {QStringLiteral("right"), QStringLiteral("mouse")},
        {QStringLiteral("middle"), QStringLiteral("mouse")},        {QStringLiteral("back"), QStringLiteral("arrow-left")},
        {QStringLiteral("forward"), QStringLiteral("arrow-right")}, {QStringLiteral("wheel-up"), QStringLiteral("arrow-up")},
        {QStringLiteral("wheel-down"), QStringLiteral("arrow-down")}, {QStringLiteral("wheel-left"), QStringLiteral("arrow-left")},
        {QStringLiteral("wheel-right"), QStringLiteral("arrow-right")},
    };
    return m;
}

// Words in a command line (lower-cased basenames) and the app they name.
const QHash<QString, QString> &commandIcons()
{
    static const QHash<QString, QString> m{
        {QStringLiteral("brave"), QStringLiteral("world")},           {QStringLiteral("firefox"), QStringLiteral("world")},
        {QStringLiteral("chromium"), QStringLiteral("world")},        {QStringLiteral("github"), QStringLiteral("brand-github")},
        {QStringLiteral("kdenlive"), QStringLiteral("movie")},        {QStringLiteral("grafium"), QStringLiteral("chart-dots-3")},
        {QStringLiteral("terminal"), QStringLiteral("terminal-2")},   {QStringLiteral("st"), QStringLiteral("terminal-2")},
        {QStringLiteral("foot"), QStringLiteral("terminal-2")},       {QStringLiteral("kitty"), QStringLiteral("terminal-2")},
        {QStringLiteral("nemo"), QStringLiteral("folder")},           {QStringLiteral("files"), QStringLiteral("folder")},
        {QStringLiteral("nautilus"), QStringLiteral("folder")},       {QStringLiteral("thunar"), QStringLiteral("folder")},
        {QStringLiteral("smplos-settings"), QStringLiteral("settings")}, {QStringLiteral("settings"), QStringLiteral("settings")},
        {QStringLiteral("spotify"), QStringLiteral("brand-spotify")},
    };
    return m;
}
const QString kCommandDefaultIcon = QStringLiteral("terminal");

// The curated Kdenlive actions (kdenlivecatalog.cpp); tst_cheatsheet checks every one has an icon.
const QHash<QString, QString> &actionIcons()
{
    static const QHash<QString, QString> m{
        // playback
        {QStringLiteral("monitor_play"), QStringLiteral("player-play")},
        {QStringLiteral("monitor_pause"), QStringLiteral("player-pause")},
        {QStringLiteral("monitor_seek_backward"), QStringLiteral("chevrons-left")},
        {QStringLiteral("monitor_seek_forward"), QStringLiteral("chevrons-right")},
        {QStringLiteral("monitor_play_zone"), QStringLiteral("player-play")},
        {QStringLiteral("monitor_play_zone_cursor"), QStringLiteral("player-play")},
        {QStringLiteral("monitor_loop_zone"), QStringLiteral("repeat")},
        {QStringLiteral("monitor_loop_clip"), QStringLiteral("repeat")},
        // monitor
        {QStringLiteral("switch_monitor"), QStringLiteral("switch-horizontal")},
        {QStringLiteral("monitor_zoomin"), QStringLiteral("zoom-in")},
        {QStringLiteral("monitor_zoomout"), QStringLiteral("zoom-out")},
        {QStringLiteral("monitor_zoomreset"), QStringLiteral("zoom-reset")},
        // navigation
        {QStringLiteral("zoom_fit"), QStringLiteral("zoom-reset")},
        {QStringLiteral("view_zoom_in"), QStringLiteral("zoom-in")},
        {QStringLiteral("view_zoom_out"), QStringLiteral("zoom-out")},
        {QStringLiteral("seek_start"), QStringLiteral("arrow-bar-to-left")},
        {QStringLiteral("seek_end"), QStringLiteral("arrow-bar-to-right")},
        {QStringLiteral("seek_clip_start"), QStringLiteral("arrow-bar-to-left")},
        {QStringLiteral("seek_clip_end"), QStringLiteral("arrow-bar-to-right")},
        {QStringLiteral("seek_zone_start"), QStringLiteral("player-skip-back")},
        {QStringLiteral("seek_zone_end"), QStringLiteral("player-skip-forward")},
        {QStringLiteral("monitor_seek_snap_backward"), QStringLiteral("player-skip-back")},
        {QStringLiteral("monitor_seek_snap_forward"), QStringLiteral("player-skip-forward")},
        {QStringLiteral("monitor_seek_guide_backward"), QStringLiteral("arrow-left-bar")},
        {QStringLiteral("monitor_seek_guide_forward"), QStringLiteral("arrow-right-bar")},
        // zone
        {QStringLiteral("mark_in"), QStringLiteral("brackets-contain-start")},
        {QStringLiteral("mark_out"), QStringLiteral("brackets-contain-end")},
        // markers (add_marker_guide_1..10 below)
        {QStringLiteral("add_marker_guide_quickly"), QStringLiteral("bookmark")},
        {QStringLiteral("delete_clip_marker"), QStringLiteral("bookmark-off")},
        {QStringLiteral("delete_sequence_marker"), QStringLiteral("bookmark-off")},
        // editing
        {QStringLiteral("insert_to_in_point"), QStringLiteral("column-insert-right")},
        {QStringLiteral("overwrite_to_in_point"), QStringLiteral("replace")},
        {QStringLiteral("remove_lift"), QStringLiteral("eraser")},
        {QStringLiteral("remove_extract"), QStringLiteral("column-remove")},
        {QStringLiteral("cut_timeline_clip"), QStringLiteral("scissors")},
        {QStringLiteral("cut_timeline_all_clips"), QStringLiteral("scissors")},
        {QStringLiteral("delete_timeline_clip"), QStringLiteral("trash")},
        {QStringLiteral("extract_clip"), QStringLiteral("column-remove")},
        {QStringLiteral("resize_timeline_clip_start"), QStringLiteral("arrows-move-horizontal")},
        {QStringLiteral("resize_timeline_clip_end"), QStringLiteral("arrows-move-horizontal")},
        {QStringLiteral("delete_space"), QStringLiteral("space-off")},
        {QStringLiteral("delete_space_all_tracks"), QStringLiteral("space-off")},
        // tools and modes
        {QStringLiteral("select_tool"), QStringLiteral("pointer")},
        {QStringLiteral("razor_tool"), QStringLiteral("blade")},
        {QStringLiteral("spacer_tool"), QStringLiteral("space")},
        {QStringLiteral("ripple_tool"), QStringLiteral("ripple")},
        {QStringLiteral("slip_tool"), QStringLiteral("arrows-left-right")},
        {QStringLiteral("normal_mode"), QStringLiteral("pencil")},
        {QStringLiteral("overwrite_mode"), QStringLiteral("replace")},
        {QStringLiteral("insert_mode"), QStringLiteral("column-insert-right")},
        // selection
        {QStringLiteral("select_timeline_clip"), QStringLiteral("square-check")},
        {QStringLiteral("deselect_timeline_clip"), QStringLiteral("square")},
        {QStringLiteral("select_add_timeline_clip"), QStringLiteral("square-plus")},
        {QStringLiteral("select_timeline_zone"), QStringLiteral("brackets-contain")},
        {QStringLiteral("select_track"), QStringLiteral("list-check")},
        {QStringLiteral("select_all_tracks"), QStringLiteral("select-all")},
        // keyframes
        {QStringLiteral("keyframe_add"), QStringLiteral("keyframe")},
        {QStringLiteral("keyframe_next"), QStringLiteral("chevron-right")},
        {QStringLiteral("keyframe_previous"), QStringLiteral("chevron-left")},
        // history
        {QStringLiteral("edit_undo"), QStringLiteral("arrow-back-up")},
        {QStringLiteral("edit_redo"), QStringLiteral("arrow-forward-up")},
        // K23 MR1b-A
        {QStringLiteral("mix_clip"), QStringLiteral("transition-right")},
        {QStringLiteral("group_clip"), QStringLiteral("link")},
        {QStringLiteral("ungroup_clip"), QStringLiteral("unlink")},
        {QStringLiteral("clip_switch"), QStringLiteral("eye-off")},
        {QStringLiteral("clip_split"), QStringLiteral("volume")},
        {QStringLiteral("edit_copy"), QStringLiteral("copy")},
        {QStringLiteral("paste_effects"), QStringLiteral("clipboard")},
        {QStringLiteral("delete_effects"), QStringLiteral("trash")},
        {QStringLiteral("master_effects"), QStringLiteral("sparkles")},
        {QStringLiteral("insert_project_tree"), QStringLiteral("file-plus")},
        {QStringLiteral("clip_in_project_tree"), QStringLiteral("folder")},
        {QStringLiteral("multicam_tool"), QStringLiteral("video")},
        {QStringLiteral("perform_multitrack_mode"), QStringLiteral("cut")},
        {QStringLiteral("switch_active_target"), QStringLiteral("target")},
        {QStringLiteral("restore_all_sources"), QStringLiteral("restore")},
        {QStringLiteral("fit_all_tracks"), QStringLiteral("layout-rows")},
        {QStringLiteral("snap"), QStringLiteral("magnet")},
        {QStringLiteral("sequence_next"), QStringLiteral("arrow-bar-right")},
        {QStringLiteral("sequence_previous"), QStringLiteral("arrow-bar-left")},
        {QStringLiteral("monitor_fullscreen"), QStringLiteral("maximize")},
        {QStringLiteral("monitor_multitrack"), QStringLiteral("layout-grid")},
        {QStringLiteral("monitor_overlay"), QStringLiteral("info-circle")},
        {QStringLiteral("extract_frame_to_clipboard"), QStringLiteral("photo")},
        {QStringLiteral("zoom_audio_in"), QStringLiteral("zoom-in")},
        {QStringLiteral("zoom_audio_out"), QStringLiteral("zoom-out")},
        {QStringLiteral("zoom_audio_reset"), QStringLiteral("zoom-reset")},
        {QStringLiteral("audiomixer_button"), QStringLiteral("adjustments-horizontal")},
        {QStringLiteral("mlt_scrub"), QStringLiteral("wave-sine")},
        {QStringLiteral("mlt_mute"), QStringLiteral("volume-off")},
    };
    return m;
}

// MR1b-A families: camera N, layout slots, bin tags, effects.
QString familyIcon(const QString &action)
{
    const QString family = catalog::actionFamily(action);
    if (family == QLatin1String("camera")) {
        return QStringLiteral("number-") + action.right(1);
    }
    if (family == QLatin1String("layout")) {
        return QStringLiteral("layout-board");
    }
    if (family == QLatin1String("tag")) {
        return QStringLiteral("tag");
    }
    if (family == QLatin1String("effect")) {
        return QStringLiteral("wand");
    }
    return {};
}

QStringList familyIconNames()
{
    QStringList n{QStringLiteral("layout-board"), QStringLiteral("tag"), QStringLiteral("wand")};
    for (int i = 1; i <= 9; ++i) {
        n << QStringLiteral("number-%1").arg(i);
    }
    return n;
}

const QHash<QString, QString> &controlIcons()
{
    using namespace cs::contract;
    static const QHash<QString, QString> m{
        {kJog, QStringLiteral("arrows-horizontal")},
        {kShuttle, QStringLiteral("chevrons-right")},
        {kZoom, QStringLiteral("zoom-in")},
        {kTrackFocus, QStringLiteral("arrows-vertical")},
        {kScroll, QStringLiteral("arrows-horizontal")},
        {kAudioGain, QStringLiteral("volume")},
        {kTrim, QStringLiteral("arrows-move-horizontal")},
        {kParamNudge, QStringLiteral("adjustments-horizontal")},
        {kParamFocus, QStringLiteral("list-details")},
    };
    return m;
}

const QHash<QString, QString> &trackIcons()  // track.set "what"
{
    static const QHash<QString, QString> m{
        {QStringLiteral("mute"), QStringLiteral("volume-3")}, {QStringLiteral("solo"), QStringLiteral("headphones")},
        {QStringLiteral("lock"), QStringLiteral("lock")},     {QStringLiteral("target"), QStringLiteral("target")},
        {QStringLiteral("hide"), QStringLiteral("eye-off")},
    };
    return m;
}

const QString kCheatsheetIcon = QStringLiteral("help-circle");
const QString kCycleIcon = QStringLiteral("stack-2");
const QString kResetIcon = QStringLiteral("rotate");
const QString kColorIcon = QStringLiteral("color-filter");

QString markerIcon(const QString &action)
{
    return action.startsWith(QLatin1String("add_marker_guide_")) ? QStringLiteral("bookmark") : QString();
}

QString commandIcon(const QStringList &argv)
{
    // The first word that names a known app: "focus-or-launch brave-browser
    // brave", "gtk-launch org.kde.kdenlive" and "/usr/bin/nemo" all count.
    for (const QString &arg : argv) {
        const QString base = QFileInfo(arg).fileName().toLower();
        QString id = base;
        if (id.endsWith(QLatin1String(".desktop"))) {
            id.chop(8);
        }
        const QString lastDot = id.section(QLatin1Char('.'), -1);
        for (const QString &w : {base, id, lastDot, lastDot.section(QLatin1Char('-'), 0, 0)}) {
            if (const auto it = commandIcons().constFind(w); it != commandIcons().cend()) {
                return *it;
            }
        }
    }
    return kCommandDefaultIcon;
}
} // namespace

QString bindingIcon(const Binding &b, const LabelEnv &env)
{
    if (b.icon == QLatin1String("none")) {
        return {};
    }
    return b.icon.isEmpty() ? autoIcon(b, env) : b.icon;
}

QString autoIcon(const Binding &b, const LabelEnv &env)
{
    using namespace cs::contract;
    switch (b.kind) {
    case Binding::None:
        return {};
    case Binding::Keys: {
        if (b.keys.size() != 1) {
            return {};  // a sequence: the label says it
        }
        const KeyChord &c = b.keys.first();
        if (c.mods == 0 && mediaIcons().contains(c.key)) {
            return mediaIcons().value(c.key);
        }
        return shortcutIcons().value(c.id());
    }
    case Binding::Mouse:
        return mouseIcons().value(b.name);
    case Binding::Action: {
        const QString icon = actionIcons().value(b.name);
        if (!icon.isEmpty()) {
            return icon;
        }
        const QString marker = markerIcon(b.name);
        return marker.isEmpty() ? familyIcon(b.name) : marker;
    }
    case Binding::Control: {
        const QString name = b.name.startsWith(QLatin1Char('$')) && env.modeValue ? env.modeValue(b.name.mid(1)) : b.name;
        if (name.startsWith(QLatin1String("colorwheel."))) {
            return kColorIcon;
        }
        return controlIcons().value(name);
    }
    case Binding::Request:
        if (b.name.endsWith(QLatin1String(".reset"))) {
            return kResetIcon;
        }
        if (b.name.startsWith(QLatin1String("colorwheel."))) {
            return kColorIcon;
        }
        if (b.name == kCmdTrackSet) {
            return trackIcons().value(opt(b, "what", env));
        }
        return {};
    case Binding::Cycle:
        return kCycleIcon;
    case Binding::Command:
        return commandIcon(b.argv);
    case Binding::Cheatsheet:
        return kCheatsheetIcon;
    }
    return {};
}

QStringList autoIconNames()
{
    QSet<QString> all{kCheatsheetIcon, kCycleIcon, kResetIcon, kColorIcon, kCommandDefaultIcon, QStringLiteral("bookmark")};
    for (const QString &v : familyIconNames()) {
        all.insert(v);
    }
    for (const QString &v : mediaIcons()) {
        all.insert(v);
    }
    for (const QString &v : shortcutIcons()) {
        all.insert(v);
    }
    for (const auto *table : {&mouseIcons(), &commandIcons(), &actionIcons(), &controlIcons(), &trackIcons()}) {
        for (const QString &v : *table) {
            all.insert(v);
        }
    }
    QStringList out(all.cbegin(), all.cend());
    out.sort();
    return out;
}

QString bindingState(const Binding &b, const LabelEnv &env)
{
    if (b.kind == Binding::Cycle && env.modeValue) {
        return env.modeValue(b.name);
    }
    return {};
}

} // namespace cs
