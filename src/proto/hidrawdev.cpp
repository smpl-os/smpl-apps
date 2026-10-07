// SPDX-License-Identifier: GPL-2.0-or-later
#include "hidrawdev.h"

#include <algorithm>
#include <climits>
#include <cstdlib>
#include <dirent.h>
#include <fstream>
#include <iterator>
#include <sstream>
#include <unistd.h>

namespace ch552 {

DescriptorInfo parseDescriptor(const std::vector<std::uint8_t> &d)
{
    DescriptorInfo info;
    int reportSize = 0;
    int reportCount = 0;
    std::size_t i = 0;
    while (i < d.size()) {
        const std::uint8_t prefix = d[i];
        if (prefix == 0xFE) {  // long item: unsupported here
            return info;
        }
        int size = prefix & 0x03;
        if (size == 3) {
            size = 4;
        }
        if (i + 1 + size > d.size()) {
            return info;
        }
        std::uint32_t value = 0;
        for (int b = 0; b < size; ++b) {
            value |= std::uint32_t(d[i + 1 + b]) << (8 * b);
        }
        const std::uint8_t tag = prefix & 0xFC;
        switch (tag) {
        case 0x74:
            reportSize = int(value);
            break;
        case 0x94:
            reportCount = int(value);
            break;
        case 0x84:
            info.hasReportIds = true;
            info.reportIds.push_back(int(value));
            break;
        case 0x90:
            info.outputBits += reportSize * reportCount;
            break;
        case 0x80:
            info.inputBits += reportSize * reportCount;
            break;
        default:
            break;
        }
        i += 1 + size;
    }
    info.ok = true;
    return info;
}

std::optional<Generation> generationFor(const DescriptorInfo &info)
{
    if (!info.ok || info.outputBits != 512) {
        return std::nullopt;
    }
    if (!info.hasReportIds) {
        return Generation::Rid0;
    }
    if (info.reportIds.size() == 1 && info.reportIds[0] == 3) {
        return Generation::Rid3;
    }
    return std::nullopt;
}

namespace {
std::string readTrim(const std::string &path)
{
    std::ifstream f(path);
    std::string s;
    std::getline(f, s);
    while (!s.empty() && (s.back() == '\n' || s.back() == ' ' || s.back() == '\r')) {
        s.pop_back();
    }
    return s;
}

std::vector<std::uint8_t> readBytes(const std::string &path)
{
    std::ifstream f(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(f), std::istreambuf_iterator<char>()};
}

std::string canonical(const std::string &p)
{
    char buf[PATH_MAX];
    if (realpath(p.c_str(), buf)) {
        return buf;
    }
    return {};
}

std::string parentDir(const std::string &p)
{
    auto pos = p.find_last_of('/');
    return pos == std::string::npos || pos == 0 ? std::string("/") : p.substr(0, pos);
}
} // namespace

std::vector<HidrawNode> findPadHidraw(const std::string &vendor, const std::string &product, const std::string &serial,
                                      const std::string &sysRoot)
{
    std::vector<HidrawNode> out;
    const std::string cls = sysRoot + "/sys/class/hidraw";
    DIR *dir = opendir(cls.c_str());
    if (!dir) {
        return out;
    }
    std::vector<std::string> names;
    while (auto *e = readdir(dir)) {
        std::string n = e->d_name;
        if (n.rfind("hidraw", 0) == 0) {
            names.push_back(n);
        }
    }
    closedir(dir);
    std::sort(names.begin(), names.end());
    for (const auto &n : names) {
        HidrawNode node;
        node.sysPath = cls + "/" + n;
        node.devnode = "/dev/" + n;
        // .../<usbdev>/<usbdev>:1.1/0003:1189:8890.0007 is the hid device
        const std::string hid = canonical(node.sysPath + "/device");
        if (hid.empty()) {
            continue;
        }
        const std::string intf = parentDir(hid);
        const std::string usb = parentDir(intf);
        node.vendor = readTrim(usb + "/idVendor");
        node.product = readTrim(usb + "/idProduct");
        if (node.vendor != vendor || node.product != product) {
            continue;
        }
        node.serial = readTrim(usb + "/serial");
        if (!serial.empty() && node.serial != serial) {
            continue;
        }
        node.usbPath = usb;
        const std::string ifn = readTrim(intf + "/bInterfaceNumber");
        node.interfaceNumber = ifn.empty() ? -1 : int(std::strtol(ifn.c_str(), nullptr, 16));
        node.descriptor = readBytes(hid + "/report_descriptor");
        out.push_back(std::move(node));
    }
    return out;
}

} // namespace ch552
