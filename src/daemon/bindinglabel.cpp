// SPDX-License-Identifier: GPL-2.0-or-later
#include "bindinglabel.h"
#include "kdenlivecatalog.h"
#include "kdenlivecontract.h"

#include <QFileInfo>
#include <QHash>

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

QString bindingState(const Binding &b, const LabelEnv &env)
{
    if (b.kind == Binding::Cycle && env.modeValue) {
        return env.modeValue(b.name);
    }
    return {};
}

} // namespace cs
