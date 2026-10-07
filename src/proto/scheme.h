// SPDX-License-Identifier: GPL-2.0-or-later
// The 24-slot code scheme written into the pad and the slot -> physical control
// hypotheses. Shared by the flasher, the verifier and the daemon.
#pragma once

#include "ch552proto.h"

#include <string>
#include <vector>

namespace ch552 {

// HID keyboard usages 0x69..0x6E are F14..F19 (Linux KEY_F14..KEY_F19 = 184..189).
constexpr std::uint8_t kUsageF14 = 0x69;
constexpr int kEvdevF14 = 184;

struct SlotCode {
    std::uint8_t slot;
    Chord chord;
    std::string name;  // e.g. "shift+F16"
};

// slot n (1-based): base key F14 + (n-1) % 6, modifier group (n-1) / 6 in
// {none, LShift, LCtrl, LAlt}. 6 x 4 = 24 distinct chords, none of which xkb
// or the desktop binds (see docs/hardware-ch552.md).
std::vector<SlotCode> defaultScheme();

enum class Numbering {
    KeysThenKnobs,  // "A": keys 1..15, knob n (0-based) ccw/press/cw = 16+3n+{0,1,2}
    VendorTwelve,   // "B": keys 1..12, knobs 13+3n+{0,1,2}, keys 13..15 at 22..24
};

struct SlotTarget {
    std::string control;  // key1..key15, knob1..knob3
    std::string role;     // "key", "ccw", "press", "cw"
};

SlotTarget slotTarget(Numbering n, std::uint8_t slot);
std::string numberingName(Numbering n);

} // namespace ch552
