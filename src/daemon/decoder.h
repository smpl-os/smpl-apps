// SPDX-License-Identifier: GPL-2.0-or-later
// Turns raw evdev key events from the grabbed pad into chord and control events.
#pragma once

#include "hardwaremap.h"

#include <QHash>
#include <QList>

namespace cs {

struct PadEvent {
    enum Type { KeyDown, KeyUp, Turn, PressDown, PressUp };
    QString control;  // key1..key15 / knob1..knob3
    Type type = KeyDown;
    int delta = 0;    // Turn: -1 ccw, +1 cw
    qint64 usec = 0;  // kernel timestamp
    QString describe() const;
};

struct ChordEvent {
    KeyChord chord;
    bool down = true;
    qint64 usec = 0;
};

class Decoder
{
public:
    // Feed one input_event. Returned chord events are complete (modifiers resolved).
    QList<ChordEvent> feed(int type, int code, int value, qint64 usec);
    void reset();
    quint8 heldModifiers() const { return m_mods; }

private:
    quint8 m_mods = 0;
    QHash<int, KeyChord> m_down;
};

// Chord -> control event using a hardware map; nullopt for unmapped chords
// and for releases of knob turns (a detent is a single event).
std::optional<PadEvent> toPadEvent(const HardwareMap &map, const ChordEvent &e);

} // namespace cs
