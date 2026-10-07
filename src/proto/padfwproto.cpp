// SPDX-License-Identifier: GPL-2.0-or-later
#include "padfwproto.h"

#include <cerrno>
#include <chrono>
#include <cstring>
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

std::string Info::version() const
{
    return std::to_string(fwMajor) + '.' + std::to_string(fwMinor) + '.' + std::to_string(fwPatch);
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
    return RawEvent{data[1], data[2], data[3], data[4]};
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

} // namespace padfw
