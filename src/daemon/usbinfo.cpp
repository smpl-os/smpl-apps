// SPDX-License-Identifier: GPL-2.0-or-later
#include "usbinfo.h"

#include <QDir>
#include <QFile>

namespace cs {

namespace {
QString readTrim(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        return {};
    }
    return QString::fromUtf8(f.readAll()).trimmed();
}
} // namespace

std::optional<UsbDeviceInfo> usbDeviceAt(const QString &dir)
{
    if (!QFile::exists(dir + QStringLiteral("/idVendor"))) {
        return std::nullopt;
    }
    UsbDeviceInfo d;
    d.sysPath = dir;
    d.vendor = readTrim(dir + QStringLiteral("/idVendor")).toLower();
    d.product = readTrim(dir + QStringLiteral("/idProduct")).toLower();
    d.serial = readTrim(dir + QStringLiteral("/serial"));
    d.manufacturer = readTrim(dir + QStringLiteral("/manufacturer"));
    d.productName = readTrim(dir + QStringLiteral("/product"));
    d.bcdDevice = readTrim(dir + QStringLiteral("/bcdDevice")).toLower();
    d.busnum = readTrim(dir + QStringLiteral("/busnum")).toInt();
    d.devnum = readTrim(dir + QStringLiteral("/devnum")).toInt();
    d.interfaces = readTrim(dir + QStringLiteral("/bNumInterfaces")).toInt();
    return d;
}

QList<UsbDeviceInfo> listUsbDevices(const QString &sysRoot)
{
    const QString root = (sysRoot.isEmpty() ? QStringLiteral("/sys") : sysRoot) + QStringLiteral("/bus/usb/devices");
    QList<UsbDeviceInfo> out;
    const QStringList names = QDir(root).entryList(QDir::Dirs | QDir::NoDotAndDotDot | QDir::System, QDir::Name);
    for (const QString &name : names) {
        if (name.contains(QLatin1Char(':'))) {
            continue;  // interfaces have a colon in their name
        }
        if (auto d = usbDeviceAt(root + QLatin1Char('/') + name)) {
            out << *d;
        }
    }
    return out;
}

QJsonObject FirmwareInfo::toJson() const
{
    return QJsonObject{{QStringLiteral("type"), type},
                       {QStringLiteral("version"), version},
                       {QStringLiteral("versionSource"), versionSource},
                       {QStringLiteral("board"), board},
                       {QStringLiteral("slots"), slotCount > 0 ? QJsonValue(slotCount) : QJsonValue()}};
}

bool isWchIsp(const UsbDeviceInfo &d)
{
    const QString id = d.vendor + QLatin1Char(':') + d.product;
    return id == QLatin1String(kBootloaderId) || id == QLatin1String(kBootloaderIdAlt);
}

FirmwareInfo classifyFirmware(const UsbDeviceInfo &d)
{
    FirmwareInfo f;
    const QString id = d.vendor + QLatin1Char(':') + d.product;
    if (isWchIsp(d)) {
        f.type = QStringLiteral("bootloader");
        return f;
    }
    if (id != QLatin1String("1189:8890")) {
        f.type = QStringLiteral("unknown");
        return f;
    }
    if (d.manufacturer == QLatin1String("OpenMacroPad") && d.productName.startsWith(QLatin1String("Control Surface"))) {
        // bcdDevice MMmm: major in the high byte, minor in the low byte (BCD).
        f.type = QStringLiteral("control-surface");
        const QString bcd = d.bcdDevice.rightJustified(4, QLatin1Char('0'));
        f.version = QStringLiteral("%1.%2").arg(bcd.left(2).toInt()).arg(bcd.mid(2).toInt());
        f.versionSource = QStringLiteral("bcdDevice");
        if (d.productName == QLatin1String("Control Surface 15+3")) {
            f.board = QStringLiteral("sy181-15k3e");
        }
        return f;
    }
    if (d.manufacturer == QLatin1String("SY181")) {
        f.type = QStringLiteral("openmacropad");  // EpicLPer's firmware or its discovery build
        return f;
    }
    if (d.manufacturer == QLatin1String("wch.cn") || d.interfaces >= 3) {
        f.type = QStringLiteral("stock");
        return f;
    }
    f.type = QStringLiteral("unknown");
    return f;
}

} // namespace cs
