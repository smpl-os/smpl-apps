// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QList>
#include <QString>
#include <optional>

namespace cs {

// One USB device as sysfs describes it (/sys/bus/usb/devices/<bus>-<port>).
struct UsbDeviceInfo {
    QString sysPath;
    QString vendor, product;  // lower-case hex, 4 digits
    QString serial, manufacturer, productName;
    QString bcdDevice;        // 4 hex digits, e.g. "0200"
    int busnum = 0;
    int devnum = 0;
    int interfaces = 0;       // bNumInterfaces of the active configuration
};

// Every USB device (not interfaces) under sysRoot/bus/usb/devices.
QList<UsbDeviceInfo> listUsbDevices(const QString &sysRoot = {});
// The USB device at one sysfs directory (as found by findPadInputNodes).
std::optional<UsbDeviceInfo> usbDeviceAt(const QString &sysPath);

constexpr const char *kBootloaderId = "4348:55e0";  // WCH CH55x ROM bootloader
constexpr const char *kBootloaderIdAlt = "1a86:55e0";  // the same ISP protocol on other WCH chips (wchisp opens both)
// Any WCH ISP bootloader session wchisp could pick.
bool isWchIsp(const UsbDeviceInfo &d);

// What runs on a 1189:8890 pad (or the ROM bootloader), from descriptors only.
struct FirmwareInfo {
    QString type;     // control-surface | openmacropad | stock | bootloader | unknown
    QString version;  // control-surface: from bcdDevice, e.g. "2.0"
    QString board;    // board profile id when the descriptors name one
    QJsonObject toJson() const;
};

FirmwareInfo classifyFirmware(const UsbDeviceInfo &d);

} // namespace cs
