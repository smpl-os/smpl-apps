// SPDX-License-Identifier: GPL-2.0-or-later
#include "hardwaremap.h"

#include <algorithm>

namespace cs {

QString roleName(Role r)
{
    switch (r) {
    case Role::Key:
        return QStringLiteral("key");
    case Role::Ccw:
        return QStringLiteral("ccw");
    case Role::Cw:
        return QStringLiteral("cw");
    case Role::Press:
        return QStringLiteral("press");
    }
    return {};
}

std::optional<Role> roleFromName(const QString &s)
{
    if (s == QLatin1String("key")) {
        return Role::Key;
    }
    if (s == QLatin1String("ccw")) {
        return Role::Ccw;
    }
    if (s == QLatin1String("cw")) {
        return Role::Cw;
    }
    if (s == QLatin1String("press")) {
        return Role::Press;
    }
    return std::nullopt;
}

QString PadTarget::name() const
{
    return role == Role::Key ? control : control + QLatin1Char('.') + roleName(role);
}

KeyChord chordFromSchemeSlot(const ch552::SlotCode &sc)
{
    // HID left-modifier bits equal our Mod bits for ctrl/shift/alt/gui.
    return KeyChord{sc.chord.mods, ch552::kEvdevF14 + (sc.chord.usage - ch552::kUsageF14)};
}

HardwareMap HardwareMap::fromScheme(ch552::Numbering numbering)
{
    HardwareMap m;
    for (const auto &sc : ch552::defaultScheme()) {
        const auto t = ch552::slotTarget(numbering, sc.slot);
        m.insert(chordFromSchemeSlot(sc), PadTarget{QString::fromStdString(t.control), *roleFromName(QString::fromStdString(t.role))});
    }
    return m;
}

std::optional<HardwareMap> HardwareMap::fromJson(const QJsonObject &o, QString *error)
{
    const QJsonObject chords = o.value(QStringLiteral("chords")).toObject();
    if (chords.isEmpty()) {
        if (error) {
            *error = QStringLiteral("hardware map has no \"chords\" object");
        }
        return std::nullopt;
    }
    HardwareMap m;
    for (auto it = chords.begin(); it != chords.end(); ++it) {
        const auto c = parseChord(it.key(), error);
        if (!c) {
            return std::nullopt;
        }
        const QString target = it.value().toString();
        const int dot = target.indexOf(QLatin1Char('.'));
        PadTarget t;
        t.control = dot < 0 ? target : target.left(dot);
        const auto role = dot < 0 ? std::optional<Role>(Role::Key) : roleFromName(target.mid(dot + 1));
        const bool keyOk = t.control.startsWith(QLatin1String("key")) && *role == Role::Key;
        const bool knobOk = t.control.startsWith(QLatin1String("knob")) && role && *role != Role::Key;
        if (!role || !(keyOk || knobOk)) {
            if (error) {
                *error = QStringLiteral("bad hardware target '%1'").arg(target);
            }
            return std::nullopt;
        }
        t.role = *role;
        m.insert(*c, t);
    }
    return m;
}

QJsonObject HardwareMap::toJson() const
{
    QJsonObject chords;
    for (auto it = m_map.begin(); it != m_map.end(); ++it) {
        chords.insert(chordName(m_chords.value(it.key())), it.value().name());
    }
    return QJsonObject{{QStringLiteral("chords"), chords}};
}

std::optional<PadTarget> HardwareMap::lookup(const KeyChord &c) const
{
    const auto it = m_map.constFind(c.id());
    if (it == m_map.constEnd()) {
        return std::nullopt;
    }
    return *it;
}

void HardwareMap::insert(const KeyChord &c, const PadTarget &t)
{
    m_map.insert(c.id(), t);
    m_chords.insert(c.id(), c);
}

QList<KeyChord> HardwareMap::chords() const
{
    QList<KeyChord> l = m_chords.values();
    std::sort(l.begin(), l.end(), [](const KeyChord &a, const KeyChord &b) { return a.id() < b.id(); });
    return l;
}

} // namespace cs
