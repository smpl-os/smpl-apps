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
    // A release the daemon infers (device gone, raw session lost), not one the
    // pad reported: it ends holds but never fires a tap deferred to release.
    bool synthetic = false;
    QString describe() const;
};

struct ChordEvent {
    KeyChord chord;
    bool down = true;
    qint64 usec = 0;
    // Modifiers of the other chords down when this one went down: the pad
    // keeps them in its report, so a chord pressed meanwhile carries them too.
    quint8 otherMods = 0;
    bool synthetic = false;  // a release inferred by releaseAll()
};

class Decoder
{
public:
    // Feed one input_event. Returned chord events are complete (modifiers resolved).
    QList<ChordEvent> feed(int type, int code, int value, qint64 usec);
    void reset();
    // Releases (synthetic) for every chord still down, then reset(): the node
    // closed or events were dropped, so nothing stays held in the engine.
    QList<ChordEvent> releaseAll();
    quint8 heldModifiers() const { return m_mods; }

private:
    struct Down {
        KeyChord chord;
        quint8 otherMods = 0;
    };
    quint8 m_mods = 0;
    QHash<int, Down> m_down;
};

// Chord -> control event using a hardware map; nullopt for unmapped chords
// and for releases of knob turns (a detent is a single event). A chord that
// maps only without the modifiers other held chords add (key8 = Shift+F15
// held while knob2 turns Alt+F14: Shift+Alt+F14) maps without them.
std::optional<PadEvent> toPadEvent(const HardwareMap &map, const ChordEvent &e);

} // namespace cs
