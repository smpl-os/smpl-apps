// SPDX-License-Identifier: GPL-2.0-or-later
// Locating the CH552 configuration interface (hidraw) and checking its report
// descriptor before anything is written to it.
#pragma once

#include "ch552proto.h"

#include <optional>
#include <string>
#include <vector>

namespace ch552 {

struct DescriptorInfo {
    bool hasReportIds = false;
    std::vector<int> reportIds;
    int outputBits = 0;  // total bits of output items (no report id: one report)
    int inputBits = 0;
    bool ok = false;     // parse succeeded
};

DescriptorInfo parseDescriptor(const std::vector<std::uint8_t> &desc);
// Rid0 if no report ids and a 512-bit output report; Rid3 if report id 3 only.
std::optional<Generation> generationFor(const DescriptorInfo &info);

struct HidrawNode {
    std::string devnode;      // /dev/hidrawN
    std::string sysPath;      // /sys/class/hidraw/hidrawN
    std::string usbPath;      // /sys/bus/usb/devices/1-5
    std::string vendor, product, serial;
    int interfaceNumber = -1;
    std::vector<std::uint8_t> descriptor;
};

// sysRoot lets tests point at a fake tree; "" means the real /sys.
std::vector<HidrawNode> findPadHidraw(const std::string &vendor, const std::string &product,
                                      const std::string &serial = {}, const std::string &sysRoot = {});

} // namespace ch552
