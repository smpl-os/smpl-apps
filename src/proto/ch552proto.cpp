// SPDX-License-Identifier: GPL-2.0-or-later
#include "ch552proto.h"

#include <cstdio>
#include <stdexcept>

namespace ch552 {

namespace {
Frame make(std::initializer_list<std::uint8_t> bytes)
{
    Frame f{};
    std::size_t i = 0;
    for (auto b : bytes) {
        f[i++] = b;
    }
    return f;
}

std::uint8_t typeByte(Generation g, std::uint8_t type, std::uint8_t layer)
{
    if (g == Generation::Rid0) {
        return type & 0x0f;
    }
    return static_cast<std::uint8_t>(((layer & 0x0f) << 4) | (type & 0x0f));
}

void checkSlot(std::uint8_t slot)
{
    if (slot < 1 || slot > kMaxSlot) {
        throw std::invalid_argument("slot out of range 1.." + std::to_string(kMaxSlot) + ": " + std::to_string(slot));
    }
}
} // namespace

std::uint8_t reportIdFor(Generation g)
{
    return g == Generation::Rid0 ? 0 : 3;
}

Frame pingFrame()
{
    return Frame{};
}

Frame commitFrame()
{
    return make({0xAA, 0xAA});
}

Frame layerSelectFrame(std::uint8_t layer)
{
    return make({0xA1, static_cast<std::uint8_t>(layer == 0 ? 1 : layer)});
}

std::vector<Frame> keyBinding(Generation g, std::uint8_t slot, const std::vector<Chord> &steps, std::uint8_t layer)
{
    checkSlot(slot);
    if (steps.empty() || steps.size() > kMaxSteps) {
        throw std::invalid_argument("macro must have 1..5 steps");
    }
    for (const auto &c : steps) {
        // Interface 0 declares keyboard usages 0x00..0x91 only.
        if (c.usage == 0 || c.usage > 0x91) {
            throw std::invalid_argument("usage outside keyboard descriptor range");
        }
    }
    const auto n = static_cast<std::uint8_t>(steps.size());
    const auto type = typeByte(g, 1, layer);
    std::vector<Frame> out;
    if (g == Generation::Rid3) {
        out.push_back(layerSelectFrame(layer));
    }
    out.push_back(make({slot, type, n, 0, steps[0].mods, 0}));
    for (std::uint8_t i = 1; i <= n; ++i) {
        out.push_back(make({slot, type, n, i, steps[i - 1].mods, steps[i - 1].usage}));
    }
    out.push_back(commitFrame());
    return out;
}

std::vector<Frame> emptyKey(Generation g, std::uint8_t slot, std::uint8_t layer)
{
    checkSlot(slot);
    std::vector<Frame> out;
    if (g == Generation::Rid3) {
        out.push_back(layerSelectFrame(layer));
    }
    out.push_back(make({slot, typeByte(g, 1, layer), 1, 0, 0, 0}));
    out.push_back(commitFrame());
    return out;
}

bool isForbiddenOpcode(std::uint8_t b)
{
    // 0xEF / 0x5A: bootloader/firmware commands in related protocols.
    // 0xFC: hardware-variant command. 0xFE: 884x/"FE preamble" dialect, not ours.
    // 0xB0: LED programming (not needed, keep out of this tool).
    return b == 0xEF || b == 0x5A || b == 0xFC || b == 0xFE || b == 0xB0 || b == 0xFD;
}

bool isAllowedFrame(const Frame &f)
{
    const auto b = f[0];
    if (isForbiddenOpcode(b)) {
        return false;
    }
    if (b == 0x00) {
        for (auto x : f) {
            if (x != 0) {
                return false;
            }
        }
        return true;
    }
    if (b == 0xAA) {
        return f[1] == 0xAA;
    }
    if (b == 0xA1) {
        return true;
    }
    return b >= 1 && b <= kMaxSlot && (f[1] & 0x0f) == 1;
}

std::string hex(const Frame &f, std::size_t bytes)
{
    std::string s;
    char buf[4];
    for (std::size_t i = 0; i < bytes && i < f.size(); ++i) {
        std::snprintf(buf, sizeof buf, i ? " %02x" : "%02x", f[i]);
        s += buf;
    }
    return s;
}

namespace blob {

Frame openFrame()
{
    Frame f{};
    f[0] = kMarker;
    f[1] = 0xA1;
    f[2] = 0x01;
    return f;
}

Frame closeFrame()
{
    Frame f{};
    f[0] = kMarker;
    f[1] = 0xAA;
    f[2] = 0xAA;
    return f;
}

Frame record(std::uint8_t keyId, const Chord &chord)
{
    if (keyId < 1 || keyId > kMaxSlot) {
        throw std::invalid_argument("key id out of range");
    }
    if (chord.usage > 0x91) {
        throw std::invalid_argument("usage outside the keyboard page range");
    }
    Frame f{};
    f[0] = kMarker;
    f[1] = keyId;
    f[2] = chord.mods;
    f[3] = 0;  // reserved byte of the boot keyboard report
    f[4] = chord.usage;
    return f;
}

bool isAllowedFrame(const Frame &f)
{
    if (f[0] != kMarker || isForbiddenOpcode(f[1])) {
        return false;
    }
    auto zeroFrom = [&](std::size_t i) {
        for (; i < f.size(); ++i) {
            if (f[i] != 0) {
                return false;
            }
        }
        return true;
    };
    if (f[1] == 0xA1) {
        return f[2] == 0x01 && zeroFrom(3);
    }
    if (f[1] == 0xAA) {
        return f[2] == 0xAA && zeroFrom(3);
    }
    return f[1] >= 1 && f[1] <= kMaxSlot && f[3] == 0 && f[4] <= 0x91 && zeroFrom(5);
}

} // namespace blob

namespace keyid {

Frame record(std::uint8_t keyId, const Chord &chord)
{
    if (keyId < 1 || keyId > kMaxSlot) {
        throw std::invalid_argument("key id out of range");
    }
    if (chord.usage > 0x91) {
        throw std::invalid_argument("usage outside the keyboard page range");
    }
    Frame f{};
    f[0] = keyId;
    f[1] = chord.mods;
    f[2] = 0;  // reserved byte of the boot keyboard report
    f[3] = chord.usage;
    return f;
}

bool isAllowedFrame(const Frame &f)
{
    if (f[0] < 1 || f[0] > kMaxSlot || f[2] != 0 || f[3] > 0x91) {
        return false;
    }
    for (std::size_t i = 4; i < f.size(); ++i) {
        if (f[i] != 0) {
            return false;
        }
    }
    return true;
}

} // namespace keyid

} // namespace ch552
