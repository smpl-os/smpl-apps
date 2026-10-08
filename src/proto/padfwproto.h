// SPDX-License-Identifier: GPL-2.0-or-later
// Host side of the control-surface pad firmware's protocol v3
// (firmware/src/padstore.h): requests, replies and raw input events.
#pragma once

#include <array>
#include <cstdint>
#include <optional>
#include <string>
#include <vector>

namespace padfw {

constexpr std::uint8_t kCfgReport = 3;  // vendor page 0xFF00, 15 bytes each way
constexpr std::uint8_t kRawReport = 5;  // [seq][slot][event][layer][count] (2.0.0: no count)
constexpr std::size_t kRequestSize = 16;

enum Cmd : std::uint8_t {
    GetInfo = 0x01,
    GetAction = 0x02,
    SetAction = 0x03,
    Reset = 0x04,
    Bootloader = 0x05,
    Dump = 0x06,
    RawMode = 0x08,
    SetLayer = 0x09,
    GetStats = 0x0A,  // 2.0.1+
    GetKeys = 0x0B,   // 2.0.2+
};

enum Status : std::uint8_t { Ok = 1, BadIndex = 2, BadAction = 3, WriteFail = 4, BadArg = 5, Unknown = 6 };
enum RawEventKind : std::uint8_t { Down = 1, Up = 2, Tap = 3 };
constexpr std::uint16_t kRawTimeoutMaxMs = 10000;

using Request = std::array<std::uint8_t, kRequestSize>;

Request getInfo();
Request rawMode(std::uint16_t timeoutMs);  // 0 = off; repeat as heartbeat
Request bootloader();                      // carries the 'B','L' guard
Request getAction(std::uint8_t slot, std::uint8_t layer);
Request setLayer(std::uint8_t layer, bool persist);
Request getStats(std::uint8_t page, bool clear);
Request getKeys();

// Firmware whose raw mode the daemon relies on: 2.0.1 left it at every wrap of
// its 8-bit millisecond clock (an SDCC miscompile), 2.0.0 had no snapshot.
constexpr std::uint8_t kReliableRawMajor = 2, kReliableRawMinor = 0, kReliableRawPatch = 2;

struct Info {
    std::uint8_t format = 0, slotCount = 0, eepromBytes = 0, status = 0;
    std::uint8_t fwMajor = 0, fwMinor = 0, fwPatch = 0;  // not major/minor: glibc macros
    std::uint8_t layers = 0, activeLayer = 0, rawActive = 0, startLayer = 0;
    std::string version() const;  // "2.0.0"
    bool reliableRaw() const;     // >= 2.0.2: raw mode holds, replies carry a Snapshot
};

// The raw session in a CMD_RAW_MODE or CMD_GET_KEYS reply (2.0.2+). Raw events
// sent before the reply reach the host first (same endpoint, in order), so it
// can tell exactly what it lost.
struct Snapshot {
    std::uint8_t epoch = 0;      // counts raw-mode starts; a change: raw was off meanwhile
    std::uint8_t seq = 0;        // sequence number of the last raw event sent
    std::uint32_t held = 0;      // bit s: slot s went DOWN in this session and is not UP yet
    bool rawActive = false;
    std::array<std::uint8_t, 6> detents{};  // raw detents sent, mod 256: k1 cw, k1 ccw, k2 cw, ...
};
std::optional<Snapshot> parseSnapshot(const std::uint8_t *data, std::size_t len);

// A reply is the report as hidraw returns it: report ID first.
bool isReplyTo(const std::uint8_t *data, std::size_t len, Cmd cmd);
std::optional<Info> parseInfo(const std::uint8_t *data, std::size_t len);  // magic "CS", format 3
std::uint8_t replyStatus(const std::uint8_t *data, std::size_t len);       // byte 7, 0 if short

struct RawEvent {
    std::uint8_t seq = 0, slot = 0, event = 0, layer = 0;
    std::uint8_t count = 1;  // detents in a Tap (firmware 2.0.1 coalesces); 1 otherwise
};
std::optional<RawEvent> parseRaw(const std::uint8_t *data, std::size_t len);

// Encoder diagnostics (GET_STATS pages 0 and 1).
struct Stats {
    std::uint16_t illegal[3] = {0, 0, 0};  // two-line jumps per knob
    std::uint16_t overruns = 0, queueDrops = 0, maxQueue = 0;
    std::uint16_t cw[3] = {0, 0, 0}, ccw[3] = {0, 0, 0};
    bool hasRaw = false;                   // page 2 (2.0.2+)
    std::uint16_t rawEntries = 0, rawExpiries = 0, rawStops = 0;
};
// The six words of one GET_STATS reply, or nothing if it is not one.
std::optional<std::array<std::uint16_t, 6>> parseStatsPage(const std::uint8_t *data, std::size_t len);

// Blocking exchange on an open hidraw fd: write one request, read until the
// matching reply (other reports, e.g. keyboard input, are skipped).
std::optional<std::vector<std::uint8_t>> exchange(int fd, const Request &req, int timeoutMs, std::string *error);

// One-shot helpers on a hidraw node (opened and closed here).
std::optional<Info> queryInfo(const std::string &devnode, std::string *error, int timeoutMs = 1000);
// GET_STATS pages 0 and 1, and 2 where the firmware has it; nothing on
// firmware without GET_STATS (2.0.0).
std::optional<Stats> queryStats(const std::string &devnode, bool clear, std::string *error, int timeoutMs = 1000);
// Asks the firmware to jump to the CH552 ROM bootloader (nothing is written).
bool requestBootloader(const std::string &devnode, std::string *error, int timeoutMs = 1000);

} // namespace padfw
