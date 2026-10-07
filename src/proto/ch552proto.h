// SPDX-License-Identifier: GPL-2.0-or-later
// Encoder for the configuration protocol of WCH CH552 "MINI KeyBoard" macro pads
// (USB 1189:8890). Pure functions: no device access, no Qt.
//
// Wire format (interface 1, 64-byte output report, sent on the interrupt-OUT
// endpoint exactly like the vendor app's WriteFile path):
//   [reportId] + 64 data bytes, only the first 8 data bytes are meaningful.
//   Firmware generations differ by report id; pads whose interface-1 report
//   descriptor declares no report id use reportId 0 and no layer nibble.
//
// Key binding (type 1 = keyboard), N chords -> N+1 frames, then a commit:
//   [slot][type][N][0][mods0][0]      header, keycode always 0
//   [slot][type][N][i][modsI][usageI] i = 1..N
//   [0xAA][0xAA]                      write flash
// Empty key: header with N = 1 and no step frames, then commit.
#pragma once

#include <array>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace ch552 {

constexpr std::size_t kFrameSize = 64;
using Frame = std::array<std::uint8_t, kFrameSize>;

enum class Generation {
    Rid0,  // descriptor has no report id: type byte has no layer nibble, no 0xA1 layer select
    Rid3,  // report id 3: (layer << 4) | type and 0xA1 layer select before each binding
};

namespace mod {
constexpr std::uint8_t LCtrl = 0x01;
constexpr std::uint8_t LShift = 0x02;
constexpr std::uint8_t LAlt = 0x04;
constexpr std::uint8_t LGui = 0x08;
}

struct Chord {
    std::uint8_t mods = 0;   // HID modifier bitmask
    std::uint8_t usage = 0;  // HID keyboard-page usage
    bool operator==(const Chord &) const = default;
};

struct Frames {
    std::uint8_t reportId = 0;
    std::vector<Frame> frames;
};

constexpr std::uint8_t kMaxSlot = 24;  // 15 keys + 3 knobs x 3 actions
constexpr std::uint8_t kMaxSteps = 5;

std::uint8_t reportIdFor(Generation g);

// Vendor-app "version check" ping: data [00 00 ...].
Frame pingFrame();
Frame commitFrame();
Frame layerSelectFrame(std::uint8_t layer);

// Throws std::invalid_argument on out-of-range slot, empty/oversized macro or a
// usage outside the keyboard page range the device descriptor declares.
std::vector<Frame> keyBinding(Generation g, std::uint8_t slot, const std::vector<Chord> &steps, std::uint8_t layer = 1);
std::vector<Frame> emptyKey(Generation g, std::uint8_t slot, std::uint8_t layer = 1);

// Opcodes this encoder must never produce (bootloader / variant / unknown).
bool isForbiddenOpcode(std::uint8_t firstByte);
// Every frame we emit must start with a slot 1..kMaxSlot, 0xAA, 0xA1 or 0x00 (ping).
bool isAllowedFrame(const Frame &f);

std::string hex(const Frame &f, std::size_t bytes = 8);

// "blob03" dialect, measured on a low-speed 1189:8890 with this descriptor
// family (barkleesanders/padclaude, padflash.swift): the firmware takes frames
// whose first wire byte is 0x03 (even though the descriptor declares no report
// ids) and stores the bytes after [0x03][keyId] verbatim as that key's 8-byte
// boot-keyboard report. A session is bracketed once:
//   open   [03 A1 01]
//   record [03 keyId mods 00 usage 00 00 00 00]   (usage 0 = blank)
//   close  [03 AA AA]
// Frames are written to hidraw as exactly 64 raw bytes: byte 0 is 0x03, not
// a report number, so the kernel sends all 64 on the interrupt-OUT endpoint.
namespace blob {
constexpr std::uint8_t kMarker = 0x03;
Frame openFrame();
Frame closeFrame();
// Throws std::invalid_argument on a key id outside 1..kMaxSlot or a usage
// outside the keyboard page range.
Frame record(std::uint8_t keyId, const Chord &chord);
bool isAllowedFrame(const Frame &f);
} // namespace blob

} // namespace ch552
