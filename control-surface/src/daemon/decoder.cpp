// SPDX-License-Identifier: GPL-2.0-or-later
#include "decoder.h"

#include <linux/input-event-codes.h>

namespace cs {

QString PadEvent::describe() const
{
    switch (type) {
    case KeyDown:
        return control + QStringLiteral(" down");
    case KeyUp:
        return control + QStringLiteral(" up");
    case Turn:
        return control + QStringLiteral(" turn %1%2").arg(delta > 0 ? QStringLiteral("+") : QString()).arg(delta);
    case PressDown:
        return control + QStringLiteral(" press");
    case PressUp:
        return control + QStringLiteral(" release");
    }
    return control;
}

QList<ChordEvent> Decoder::feed(int type, int code, int value, qint64 usec)
{
    QList<ChordEvent> out;
    if (type != EV_KEY) {
        return out;
    }
    if (const quint8 bit = modifierBit(code)) {
        if (value == 1) {
            m_mods |= bit;
        } else if (value == 0) {
            m_mods &= ~bit;
        }
        return out;
    }
    if (value == 2) {
        return out;  // kernel autorepeat: the daemon decides about repetition
    }
    if (value == 1) {
        // A second down without an up (lost event) still counts as a fresh press.
        const KeyChord c{m_mods, code};
        quint8 other = 0;
        for (auto it = m_down.cbegin(); it != m_down.cend(); ++it) {
            if (it.key() != code) {
                other |= it->chord.mods;
            }
        }
        m_down.insert(code, Down{c, other});
        out.append({c, true, usec, other});
    } else if (value == 0) {
        const Down d = m_down.contains(code) ? m_down.take(code) : Down{KeyChord{m_mods, code}, 0};
        out.append({d.chord, false, usec, d.otherMods});
    }
    return out;
}

QList<ChordEvent> Decoder::releaseAll()
{
    QList<ChordEvent> out;
    for (auto it = m_down.cbegin(); it != m_down.cend(); ++it) {
        out.append({it->chord, false, 0, it->otherMods, true});
    }
    reset();
    return out;
}

void Decoder::reset()
{
    m_mods = 0;
    m_down.clear();
}

std::optional<PadEvent> toPadEvent(const HardwareMap &map, const ChordEvent &e)
{
    auto t = map.lookup(e.chord);
    if (!t && (e.chord.mods & e.otherMods)) {
        t = map.lookup(KeyChord{quint8(e.chord.mods & ~e.otherMods), e.chord.key});
    }
    if (!t) {
        return std::nullopt;
    }
    PadEvent p;
    p.control = t->control;
    p.usec = e.usec;
    p.synthetic = e.synthetic;
    switch (t->role) {
    case Role::Key:
        p.type = e.down ? PadEvent::KeyDown : PadEvent::KeyUp;
        return p;
    case Role::Press:
        p.type = e.down ? PadEvent::PressDown : PadEvent::PressUp;
        return p;
    case Role::Ccw:
    case Role::Cw:
        if (!e.down) {
            return std::nullopt;
        }
        p.type = PadEvent::Turn;
        p.delta = t->role == Role::Cw ? 1 : -1;
        return p;
    }
    return std::nullopt;
}

} // namespace cs
