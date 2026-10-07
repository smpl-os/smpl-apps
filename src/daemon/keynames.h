// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QList>
#include <QString>
#include <optional>

namespace cs {

namespace Mod {
constexpr quint8 Ctrl = 0x01;
constexpr quint8 Shift = 0x02;
constexpr quint8 Alt = 0x04;
constexpr quint8 Meta = 0x08;
}

// A key with modifiers, evdev key codes (linux/input-event-codes.h).
struct KeyChord {
    quint8 mods = 0;
    int key = 0;
    bool operator==(const KeyChord &) const = default;
    quint32 id() const { return (quint32(mods) << 16) | quint32(key & 0xffff); }
    bool isValid() const { return key > 0; }
};

int keyCodeFromName(const QString &name);  // -1 if unknown; accepts "F14", "key_f14", "space", "a"
QString keyName(int code);                 // "F14", "SPACE", ... or "KEY_<n>"
quint8 modifierBit(int code);              // 0 if not a modifier key
int modifierKey(quint8 bit);               // left-hand key for a single modifier bit

// "ctrl+shift+s", "alt+F14", "F15", "super+1". Returns nullopt on error.
std::optional<KeyChord> parseChord(const QString &text, QString *error = nullptr);
QString chordName(const KeyChord &c);  // canonical "ctrl+shift+F14"
// "ctrl+k x" or a list -> sequence of chords
std::optional<QList<KeyChord>> parseChordSequence(const QString &text, QString *error = nullptr);

} // namespace cs
