// SPDX-License-Identifier: GPL-2.0-or-later
#include "padfwproto.h"

#include <cerrno>
#include <chrono>
#include <cstring>
#include <fcntl.h>
#include <poll.h>
#include <unistd.h>

namespace padfw {

namespace {
Request make(Cmd c)
{
    Request r{};
    r[0] = kCfgReport;
    r[1] = c;
    return r;
}
} // namespace

Request getInfo()
{
    return make(GetInfo);
}

Request rawMode(std::uint16_t timeoutMs)
{
    Request r = make(RawMode);
    r[2] = std::uint8_t(timeoutMs & 0xFF);
    r[3] = std::uint8_t(timeoutMs >> 8);
    return r;
}

Request bootloader()
{
    Request r = make(Bootloader);
    r[2] = 'B';
    r[3] = 'L';
    return r;
}

Request getAction(std::uint8_t slot, std::uint8_t layer)
{
    Request r = make(GetAction);
    r[2] = slot;
    r[7] = layer;
    return r;
}

Request setLayer(std::uint8_t layer, bool persist)
{
    Request r = make(SetLayer);
    r[2] = layer;
    r[3] = persist ? 1 : 0;
    return r;
}

Request getStats(std::uint8_t page, bool clear)
{
    Request r = make(GetStats);
    r[2] = page;
    r[3] = clear ? 1 : 0;
    return r;
}

Request getKeys()
{
    return make(GetKeys);
}

std::string Info::version() const
{
    return std::to_string(fwMajor) + '.' + std::to_string(fwMinor) + '.' + std::to_string(fwPatch);
}

bool Info::reliableRaw() const
{
    if (fwMajor != kReliableRawMajor) {
        return fwMajor > kReliableRawMajor;
    }
    if (fwMinor != kReliableRawMinor) {
        return fwMinor > kReliableRawMinor;
    }
    return fwPatch >= kReliableRawPatch;
}

std::optional<Snapshot> parseSnapshot(const std::uint8_t *data, std::size_t len)
{
    if (len < 15 || !(isReplyTo(data, len, RawMode) || isReplyTo(data, len, GetKeys)) || data[7] != Ok) {
        return std::nullopt;
    }
    Snapshot s;
    s.epoch = data[2];
    s.seq = data[3];
    s.held = std::uint32_t(data[4]) | (std::uint32_t(data[5]) << 8) | (std::uint32_t(data[6]) << 16);
    s.rawActive = data[8] != 0;
    for (std::size_t i = 0; i < s.detents.size(); ++i) {
        s.detents[i] = data[9 + i];
    }
    return s;
}

bool isReplyTo(const std::uint8_t *data, std::size_t len, Cmd cmd)
{
    return len >= 2 && data[0] == kCfgReport && data[1] == cmd;
}

std::uint8_t replyStatus(const std::uint8_t *data, std::size_t len)
{
    return len > 7 ? data[7] : 0;
}

std::optional<Info> parseInfo(const std::uint8_t *data, std::size_t len)
{
    if (len < 15 || !isReplyTo(data, len, GetInfo) || data[2] != 'C' || data[3] != 'S' || data[4] != 3) {
        return std::nullopt;
    }
    Info i;
    i.format = data[4];
    i.slotCount = data[5];
    i.eepromBytes = data[6];
    i.status = data[7];
    i.fwMajor = data[8];
    i.fwMinor = data[9];
    i.fwPatch = data[10];
    i.layers = data[11];
    i.activeLayer = data[12];
    i.rawActive = data[13];
    i.startLayer = data[14];
    if (i.status != Ok || i.slotCount == 0 || i.slotCount > 32 || i.layers == 0) {
        return std::nullopt;
    }
    return i;
}

std::optional<RawEvent> parseRaw(const std::uint8_t *data, std::size_t len)
{
    if (len < 5 || data[0] != kRawReport || data[3] < Down || data[3] > Tap) {
        return std::nullopt;
    }
    RawEvent e{data[1], data[2], data[3], data[4], 1};
    if (len >= 6 && data[3] == Tap && data[5] > 0) {
        e.count = data[5];
    }
    return e;
}

std::optional<std::array<std::uint16_t, 6>> parseStatsPage(const std::uint8_t *data, std::size_t len)
{
    if (len < 16 || !isReplyTo(data, len, GetStats) || data[7] != Ok) {
        return std::nullopt;
    }
    std::array<std::uint16_t, 6> w{};
    for (std::size_t j = 0; j < 6; ++j) {
        const std::size_t lo = j * 2 < 4 ? 3 + j * 2 : 4 + j * 2;  // byte 7 is the status
        w[j] = std::uint16_t(data[lo] | (data[lo + 1] << 8));
    }
    return w;
}

std::optional<std::vector<std::uint8_t>> exchange(int fd, const Request &req, int timeoutMs, std::string *error)
{
    if (::write(fd, req.data(), req.size()) != ssize_t(req.size())) {
        if (error) {
            *error = std::string("write: ") + std::strerror(errno);
        }
        return std::nullopt;
    }
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(timeoutMs);
    for (;;) {
        const auto left = std::chrono::duration_cast<std::chrono::milliseconds>(deadline - std::chrono::steady_clock::now()).count();
        if (left <= 0) {
            if (error) {
                *error = "no reply";
            }
            return std::nullopt;
        }
        pollfd p{fd, POLLIN, 0};
        if (::poll(&p, 1, int(left)) <= 0) {
            continue;
        }
        std::uint8_t buf[64];
        const ssize_t n = ::read(fd, buf, sizeof buf);
        if (n < 0) {
            if (errno == EAGAIN || errno == EINTR) {
                continue;
            }
            if (error) {
                *error = std::string("read: ") + std::strerror(errno);
            }
            return std::nullopt;
        }
        if (isReplyTo(buf, std::size_t(n), Cmd(req[1]))) {
            return std::vector<std::uint8_t>(buf, buf + n);
        }
    }
}

namespace {
int openNode(const std::string &devnode, std::string *error)
{
    const int fd = ::open(devnode.c_str(), O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0 && error) {
        *error = devnode + ": " + std::strerror(errno);
    }
    return fd;
}
} // namespace

std::optional<Info> queryInfo(const std::string &devnode, std::string *error, int timeoutMs)
{
    const int fd = openNode(devnode, error);
    if (fd < 0) {
        return std::nullopt;
    }
    const auto reply = exchange(fd, getInfo(), timeoutMs, error);
    ::close(fd);
    if (!reply) {
        return std::nullopt;
    }
    auto info = parseInfo(reply->data(), reply->size());
    if (!info && error) {
        *error = "not the control-surface firmware (protocol v3)";
    }
    return info;
}

std::optional<Stats> queryStats(const std::string &devnode, bool clear, std::string *error, int timeoutMs)
{
    const int fd = openNode(devnode, error);
    if (fd < 0) {
        return std::nullopt;
    }
    Stats st;
    // Page 2 first: clearing (with page 1) zeroes every counter. 2.0.1 has no
    // page 2 and answers BadArg.
    if (const auto reply = exchange(fd, getStats(2, false), timeoutMs, nullptr)) {
        if (const auto w = parseStatsPage(reply->data(), reply->size())) {
            st.hasRaw = true;
            st.rawEntries = (*w)[0];
            st.rawExpiries = (*w)[1];
            st.rawStops = (*w)[2];
        }
    }
    for (std::uint8_t page = 0; page < 2; ++page) {
        // Clear only with the last page, so both pages describe the same span.
        const auto reply = exchange(fd, getStats(page, clear && page == 1), timeoutMs, error);
        const auto w = reply ? parseStatsPage(reply->data(), reply->size()) : std::nullopt;
        if (!w) {
            ::close(fd);
            if (reply && error) {
                *error = "the firmware has no GET_STATS (2.0.0)";
            }
            return std::nullopt;
        }
        if (page == 0) {
            st.illegal[0] = (*w)[0];
            st.illegal[1] = (*w)[1];
            st.illegal[2] = (*w)[2];
            st.overruns = (*w)[3];
            st.queueDrops = (*w)[4];
            st.maxQueue = (*w)[5];
        } else {
            st.cw[0] = (*w)[0];
            st.ccw[0] = (*w)[1];
            st.cw[1] = (*w)[2];
            st.ccw[1] = (*w)[3];
            st.cw[2] = (*w)[4];
            st.ccw[2] = (*w)[5];
        }
    }
    ::close(fd);
    return st;
}

bool requestBootloader(const std::string &devnode, std::string *error, int timeoutMs)
{
    const int fd = openNode(devnode, error);
    if (fd < 0) {
        return false;
    }
    const auto reply = exchange(fd, bootloader(), timeoutMs, error);
    ::close(fd);
    if (!reply) {
        return false;
    }
    if (replyStatus(reply->data(), reply->size()) != Ok) {
        if (error) {
            *error = "the firmware refused";
        }
        return false;
    }
    return true;
}

} // namespace padfw
