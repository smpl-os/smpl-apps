// SPDX-License-Identifier: GPL-2.0-or-later
#include "keynames.h"

#include <QHash>
#include <QStringList>
#include <linux/input-event-codes.h>

namespace cs {

namespace {
struct Entry {
    const char *name;
    int code;
};

// Names follow KEY_* without the prefix; aliases come after the canonical name.
const Entry kKeys[] = {
    {"ESC", KEY_ESC}, {"ESCAPE", KEY_ESC}, {"1", KEY_1}, {"2", KEY_2}, {"3", KEY_3}, {"4", KEY_4}, {"5", KEY_5},
    {"6", KEY_6}, {"7", KEY_7}, {"8", KEY_8}, {"9", KEY_9}, {"0", KEY_0}, {"MINUS", KEY_MINUS}, {"EQUAL", KEY_EQUAL},
    {"BACKSPACE", KEY_BACKSPACE}, {"TAB", KEY_TAB}, {"Q", KEY_Q}, {"W", KEY_W}, {"E", KEY_E}, {"R", KEY_R}, {"T", KEY_T},
    {"Y", KEY_Y}, {"U", KEY_U}, {"I", KEY_I}, {"O", KEY_O}, {"P", KEY_P}, {"LEFTBRACE", KEY_LEFTBRACE},
    {"RIGHTBRACE", KEY_RIGHTBRACE}, {"ENTER", KEY_ENTER}, {"RETURN", KEY_ENTER}, {"LEFTCTRL", KEY_LEFTCTRL}, {"A", KEY_A},
    {"S", KEY_S}, {"D", KEY_D}, {"F", KEY_F}, {"G", KEY_G}, {"H", KEY_H}, {"J", KEY_J}, {"K", KEY_K}, {"L", KEY_L},
    {"SEMICOLON", KEY_SEMICOLON}, {"APOSTROPHE", KEY_APOSTROPHE}, {"GRAVE", KEY_GRAVE}, {"LEFTSHIFT", KEY_LEFTSHIFT},
    {"BACKSLASH", KEY_BACKSLASH}, {"Z", KEY_Z}, {"X", KEY_X}, {"C", KEY_C}, {"V", KEY_V}, {"B", KEY_B}, {"N", KEY_N},
    {"M", KEY_M}, {"COMMA", KEY_COMMA}, {"DOT", KEY_DOT}, {"PERIOD", KEY_DOT}, {"SLASH", KEY_SLASH},
    {"RIGHTSHIFT", KEY_RIGHTSHIFT}, {"KPASTERISK", KEY_KPASTERISK}, {"LEFTALT", KEY_LEFTALT}, {"SPACE", KEY_SPACE},
    {"CAPSLOCK", KEY_CAPSLOCK}, {"F1", KEY_F1}, {"F2", KEY_F2}, {"F3", KEY_F3}, {"F4", KEY_F4}, {"F5", KEY_F5},
    {"F6", KEY_F6}, {"F7", KEY_F7}, {"F8", KEY_F8}, {"F9", KEY_F9}, {"F10", KEY_F10}, {"NUMLOCK", KEY_NUMLOCK},
    {"SCROLLLOCK", KEY_SCROLLLOCK}, {"KP7", KEY_KP7}, {"KP8", KEY_KP8}, {"KP9", KEY_KP9}, {"KPMINUS", KEY_KPMINUS},
    {"KP4", KEY_KP4}, {"KP5", KEY_KP5}, {"KP6", KEY_KP6}, {"KPPLUS", KEY_KPPLUS}, {"KP1", KEY_KP1}, {"KP2", KEY_KP2},
    {"KP3", KEY_KP3}, {"KP0", KEY_KP0}, {"KPDOT", KEY_KPDOT}, {"F11", KEY_F11}, {"F12", KEY_F12},
    {"KPENTER", KEY_KPENTER}, {"RIGHTCTRL", KEY_RIGHTCTRL}, {"KPSLASH", KEY_KPSLASH}, {"SYSRQ", KEY_SYSRQ},
    {"PRINT", KEY_SYSRQ}, {"RIGHTALT", KEY_RIGHTALT}, {"HOME", KEY_HOME}, {"UP", KEY_UP}, {"PAGEUP", KEY_PAGEUP},
    {"LEFT", KEY_LEFT}, {"RIGHT", KEY_RIGHT}, {"END", KEY_END}, {"DOWN", KEY_DOWN}, {"PAGEDOWN", KEY_PAGEDOWN},
    {"INSERT", KEY_INSERT}, {"DELETE", KEY_DELETE}, {"MUTE", KEY_MUTE}, {"VOLUMEDOWN", KEY_VOLUMEDOWN},
    {"VOLUMEUP", KEY_VOLUMEUP}, {"KPEQUAL", KEY_KPEQUAL}, {"PAUSE", KEY_PAUSE}, {"LEFTMETA", KEY_LEFTMETA},
    {"RIGHTMETA", KEY_RIGHTMETA}, {"COMPOSE", KEY_COMPOSE}, {"MENU", KEY_COMPOSE}, {"F13", KEY_F13}, {"F14", KEY_F14},
    {"F15", KEY_F15}, {"F16", KEY_F16}, {"F17", KEY_F17}, {"F18", KEY_F18}, {"F19", KEY_F19}, {"F20", KEY_F20},
    {"F21", KEY_F21}, {"F22", KEY_F22}, {"F23", KEY_F23}, {"F24", KEY_F24}, {"PLAYPAUSE", KEY_PLAYPAUSE},
    {"NEXTSONG", KEY_NEXTSONG}, {"PREVIOUSSONG", KEY_PREVIOUSSONG}, {"STOPCD", KEY_STOPCD},
    {"BRIGHTNESSDOWN", KEY_BRIGHTNESSDOWN}, {"BRIGHTNESSUP", KEY_BRIGHTNESSUP},
};

const QHash<QString, int> &nameTable()
{
    static const QHash<QString, int> t = [] {
        QHash<QString, int> h;
        for (const auto &e : kKeys) {
            h.insert(QString::fromLatin1(e.name), e.code);
        }
        return h;
    }();
    return t;
}

const QHash<int, QString> &codeTable()
{
    static const QHash<int, QString> t = [] {
        QHash<int, QString> h;
        for (const auto &e : kKeys) {
            if (!h.contains(e.code)) {
                h.insert(e.code, QString::fromLatin1(e.name));
            }
        }
        return h;
    }();
    return t;
}

quint8 modifierFromWord(const QString &w)
{
    const QString s = w.toLower();
    if (s == QLatin1String("ctrl") || s == QLatin1String("control")) {
        return Mod::Ctrl;
    }
    if (s == QLatin1String("shift")) {
        return Mod::Shift;
    }
    if (s == QLatin1String("alt")) {
        return Mod::Alt;
    }
    if (s == QLatin1String("super") || s == QLatin1String("meta") || s == QLatin1String("win") || s == QLatin1String("logo")) {
        return Mod::Meta;
    }
    return 0;
}
} // namespace

QStringList keyNames()
{
    QStringList l;
    for (const auto &e : kKeys) {
        l << QString::fromLatin1(e.name);
    }
    return l;
}

QStringList modifierNames()
{
    return {QStringLiteral("ctrl"),  QStringLiteral("control"), QStringLiteral("shift"), QStringLiteral("alt"),
            QStringLiteral("super"), QStringLiteral("meta"),    QStringLiteral("win"),   QStringLiteral("logo")};
}

int keyCodeFromName(const QString &name)
{
    QString n = name.trimmed().toUpper();
    if (n.startsWith(QLatin1String("KEY_"))) {
        n = n.mid(4);
    }
    const auto it = nameTable().constFind(n);
    if (it != nameTable().constEnd()) {
        return *it;
    }
    if (n.startsWith(QLatin1Char('#'))) {
        bool ok = false;
        const int v = n.mid(1).toInt(&ok);
        return ok && v > 0 && v < KEY_MAX ? v : -1;
    }
    return -1;
}

QString keyName(int code)
{
    const auto it = codeTable().constFind(code);
    return it != codeTable().constEnd() ? *it : QStringLiteral("#%1").arg(code);
}

quint8 modifierBit(int code)
{
    switch (code) {
    case KEY_LEFTCTRL:
    case KEY_RIGHTCTRL:
        return Mod::Ctrl;
    case KEY_LEFTSHIFT:
    case KEY_RIGHTSHIFT:
        return Mod::Shift;
    case KEY_LEFTALT:
    case KEY_RIGHTALT:
        return Mod::Alt;
    case KEY_LEFTMETA:
    case KEY_RIGHTMETA:
        return Mod::Meta;
    default:
        return 0;
    }
}

int modifierKey(quint8 bit)
{
    switch (bit) {
    case Mod::Ctrl:
        return KEY_LEFTCTRL;
    case Mod::Shift:
        return KEY_LEFTSHIFT;
    case Mod::Alt:
        return KEY_LEFTALT;
    case Mod::Meta:
        return KEY_LEFTMETA;
    default:
        return 0;
    }
}

std::optional<KeyChord> parseChord(const QString &text, QString *error)
{
    const QStringList parts = text.trimmed().split(QLatin1Char('+'), Qt::SkipEmptyParts);
    if (parts.isEmpty()) {
        if (error) {
            *error = QStringLiteral("empty key chord");
        }
        return std::nullopt;
    }
    KeyChord c;
    for (int i = 0; i < parts.size(); ++i) {
        const QString p = parts.at(i).trimmed();
        if (i + 1 < parts.size()) {
            const quint8 m = modifierFromWord(p);
            if (!m) {
                if (error) {
                    *error = QStringLiteral("unknown modifier '%1' in '%2'").arg(p, text);
                }
                return std::nullopt;
            }
            c.mods |= m;
            continue;
        }
        const quint8 m = modifierFromWord(p);
        if (m && parts.size() == 1) {
            c.key = modifierKey(m);
            continue;
        }
        c.key = keyCodeFromName(p);
        if (c.key < 0) {
            if (error) {
                *error = QStringLiteral("unknown key '%1' in '%2'").arg(p, text);
            }
            return std::nullopt;
        }
    }
    return c;
}

QString chordName(const KeyChord &c)
{
    QStringList parts;
    if (c.mods & Mod::Ctrl) {
        parts << QStringLiteral("ctrl");
    }
    if (c.mods & Mod::Shift) {
        parts << QStringLiteral("shift");
    }
    if (c.mods & Mod::Alt) {
        parts << QStringLiteral("alt");
    }
    if (c.mods & Mod::Meta) {
        parts << QStringLiteral("super");
    }
    parts << keyName(c.key);
    return parts.join(QLatin1Char('+'));
}

std::optional<QList<KeyChord>> parseChordSequence(const QString &text, QString *error)
{
    QList<KeyChord> out;
    const QStringList words = text.split(QLatin1Char(' '), Qt::SkipEmptyParts);
    for (const auto &w : words) {
        auto c = parseChord(w, error);
        if (!c) {
            return std::nullopt;
        }
        out << *c;
    }
    if (out.isEmpty()) {
        if (error) {
            *error = QStringLiteral("empty key sequence");
        }
        return std::nullopt;
    }
    return out;
}

} // namespace cs
