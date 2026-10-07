// SPDX-License-Identifier: GPL-2.0-or-later
// Maps the chords the pad emits to physical controls (key1..key15, knob1..knob3).
#pragma once

#include "keynames.h"
#include "scheme.h"

#include <QHash>
#include <QJsonObject>
#include <optional>

namespace cs {

enum class Role { Key, Ccw, Cw, Press };
QString roleName(Role r);
std::optional<Role> roleFromName(const QString &s);

struct PadTarget {
    QString control;  // "key7", "knob2"
    Role role = Role::Key;
    bool operator==(const PadTarget &) const = default;
    QString name() const;  // "key7" or "knob2.cw"
};

class HardwareMap
{
public:
    static HardwareMap fromScheme(ch552::Numbering numbering);

    // {"chords": {"F14": "key1", "ctrl+F17": "knob1.ccw", ...}}
    static std::optional<HardwareMap> fromJson(const QJsonObject &o, QString *error = nullptr);
    QJsonObject toJson() const;

    std::optional<PadTarget> lookup(const KeyChord &c) const;
    void insert(const KeyChord &c, const PadTarget &t);
    int size() const { return int(m_map.size()); }
    QList<KeyChord> chords() const;

private:
    QHash<quint32, PadTarget> m_map;
    QHash<quint32, KeyChord> m_chords;
};

KeyChord chordFromSchemeSlot(const ch552::SlotCode &sc);

} // namespace cs
