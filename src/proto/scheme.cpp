// SPDX-License-Identifier: GPL-2.0-or-later
#include "scheme.h"

#include <stdexcept>

namespace ch552 {

std::vector<SlotCode> defaultScheme()
{
    static const std::uint8_t mods[4] = {0, mod::LShift, mod::LCtrl, mod::LAlt};
    static const char *prefixes[4] = {"", "shift+", "ctrl+", "alt+"};
    std::vector<SlotCode> out;
    for (std::uint8_t slot = 1; slot <= kMaxSlot; ++slot) {
        const int i = slot - 1;
        const auto usage = static_cast<std::uint8_t>(kUsageF14 + i % 6);
        out.push_back({slot, Chord{mods[i / 6], usage}, std::string(prefixes[i / 6]) + "F" + std::to_string(14 + i % 6)});
    }
    return out;
}

SlotTarget slotTarget(Numbering n, std::uint8_t slot)
{
    if (slot < 1 || slot > kMaxSlot) {
        throw std::invalid_argument("slot out of range");
    }
    static const char *roles[3] = {"ccw", "press", "cw"};
    if (n == Numbering::KeysThenKnobs) {
        if (slot <= 15) {
            return {"key" + std::to_string(slot), "key"};
        }
        const int k = slot - 16;
        return {"knob" + std::to_string(k / 3 + 1), roles[k % 3]};
    }
    if (slot <= 12) {
        return {"key" + std::to_string(slot), "key"};
    }
    if (slot <= 21) {
        const int k = slot - 13;
        return {"knob" + std::to_string(k / 3 + 1), roles[k % 3]};
    }
    return {"key" + std::to_string(slot - 9), "key"};
}

std::string numberingName(Numbering n)
{
    return n == Numbering::KeysThenKnobs ? "keys-then-knobs" : "vendor-twelve";
}

} // namespace ch552
