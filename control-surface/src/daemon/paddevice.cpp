// SPDX-License-Identifier: GPL-2.0-or-later
#include "paddevice.h"

#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QSocketNotifier>
#include <QTimer>

#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <libudev.h>
#include <linux/input.h>
#include <sys/ioctl.h>
#include <unistd.h>

namespace cs {

namespace {
QString readTrim(const QString &path)
{
    QFile f(path);
    if (!f.open(QIODevice::ReadOnly)) {
        return {};
    }
    return QString::fromLatin1(f.readAll()).trimmed();
}
} // namespace

QList<InputNodeInfo> findPadInputNodes(const DeviceMatch &m, const QString &sysRoot, const QString &devRoot)
{
    QList<InputNodeInfo> out;
    const QDir cls(sysRoot + QStringLiteral("/sys/class/input"));
    const QStringList entries = cls.entryList({QStringLiteral("event*")}, QDir::Dirs | QDir::System | QDir::NoDotAndDotDot, QDir::Name);
    for (const QString &name : entries) {
        QString dir = QFileInfo(cls.filePath(name) + QStringLiteral("/device")).canonicalFilePath();
        InputNodeInfo info;
        info.sysName = name;
        info.devnode = (devRoot.isEmpty() ? QStringLiteral("/dev/input/") : devRoot + QLatin1Char('/')) + name;
        // Walk up: inputN -> hid -> interface (bInterfaceNumber) -> usb device (idVendor)
        for (int depth = 0; depth < 8 && !dir.isEmpty() && dir != QLatin1String("/"); ++depth) {
            if (info.interfaceNumber < 0 && QFile::exists(dir + QStringLiteral("/bInterfaceNumber"))) {
                info.interfaceNumber = readTrim(dir + QStringLiteral("/bInterfaceNumber")).toInt(nullptr, 16);
            }
            if (QFile::exists(dir + QStringLiteral("/idVendor"))) {
                info.usbPath = dir;
                info.vendor = readTrim(dir + QStringLiteral("/idVendor")).toLower();
                info.product = readTrim(dir + QStringLiteral("/idProduct")).toLower();
                info.serial = readTrim(dir + QStringLiteral("/serial"));
                break;
            }
            dir = QFileInfo(dir).path();
        }
        if (info.usbPath.isEmpty() || info.vendor != m.vendor || info.product != m.product
            || info.vendor != QStringLiteral("%1").arg(kPadVendor, 4, 16, QLatin1Char('0'))
            || info.product != QStringLiteral("%1").arg(kPadProduct, 4, 16, QLatin1Char('0'))) {
            continue;
        }
        if (!m.serial.isEmpty() && info.serial != m.serial) {
            continue;
        }
        out << info;
    }
    return out;
}

PadDevice::PadDevice(const DeviceMatch &match, QObject *parent)
    : QObject(parent)
    , m_match(match)
{
    m_rescan = new QTimer(this);
    m_rescan->setSingleShot(true);
    connect(m_rescan, &QTimer::timeout, this, &PadDevice::rescan);
    m_poll = new QTimer(this);
    m_poll->setInterval(5000);
    connect(m_poll, &QTimer::timeout, this, [this] {
        if (m_nodes.isEmpty()) {
            rescan();
        }
    });
}

PadDevice::~PadDevice()
{
    stop();
}

QStringList PadDevice::devnodes() const
{
    QStringList l;
    for (const auto *n : m_nodes) {
        l << n->devnode;
    }
    return l;
}

void PadDevice::start()
{
    if (!m_udev) {
        m_udev = udev_new();
        if (m_udev) {
            m_monitor = udev_monitor_new_from_netlink(m_udev, "udev");
            if (m_monitor) {
                udev_monitor_filter_add_match_subsystem_devtype(m_monitor, "input", nullptr);
                udev_monitor_enable_receiving(m_monitor);
                m_udevNotifier = new QSocketNotifier(udev_monitor_get_fd(m_monitor), QSocketNotifier::Read, this);
                connect(m_udevNotifier, &QSocketNotifier::activated, this, &PadDevice::onUdev);
            }
        }
        if (!m_monitor) {
            Q_EMIT message(QStringLiteral("udev monitor unavailable, polling for the pad every 5 s"));
        }
    }
    m_poll->start();
    rescan();
}

void PadDevice::stop()
{
    closeAll();
    m_poll->stop();
    delete m_udevNotifier;
    m_udevNotifier = nullptr;
    if (m_monitor) {
        udev_monitor_unref(m_monitor);
        m_monitor = nullptr;
    }
    if (m_udev) {
        udev_unref(m_udev);
        m_udev = nullptr;
    }
}

void PadDevice::onUdev()
{
    udev_device *dev = udev_monitor_receive_device(m_monitor);
    if (!dev) {
        return;
    }
    udev_device_unref(dev);
    scheduleRescan(250);  // let all four interfaces settle and permissions apply
}

void PadDevice::scheduleRescan(int ms)
{
    m_rescan->start(ms);
}

QList<InputNodeInfo> onePad(const QList<InputNodeInfo> &nodes, const QString &preferUsbPath)
{
    QString pick;
    for (const auto &n : nodes) {
        if (!preferUsbPath.isEmpty() && n.usbPath == preferUsbPath) {
            pick = preferUsbPath;
            break;
        }
        if (pick.isEmpty() || n.usbPath < pick) {
            pick = n.usbPath;
        }
    }
    QList<InputNodeInfo> out;
    for (const auto &n : nodes) {
        if (n.usbPath == pick) {
            out << n;
        }
    }
    return out;
}

void PadDevice::rescan()
{
    // A second pad plugged in later never takes over the one in use.
    const auto found = onePad(findPadInputNodes(m_match), m_usbPath);
    m_usbPath = found.isEmpty() ? QString() : found.first().usbPath;
    QStringList want;
    for (const auto &f : found) {
        want << f.devnode;
    }
    // Drop nodes that vanished.
    for (int i = m_nodes.size() - 1; i >= 0; --i) {
        if (!want.contains(m_nodes.at(i)->devnode)) {
            closeNode(i);
        }
    }
    const bool wasConnected = !m_nodes.isEmpty();
    for (const auto &f : found) {
        bool have = false;
        for (const auto *n : m_nodes) {
            have = have || n->devnode == f.devnode;
        }
        if (!have) {
            openNode(f);
        }
    }
    if (!wasConnected && !m_nodes.isEmpty()) {
        Q_EMIT connected(devnodes());
    } else if (wasConnected && m_nodes.isEmpty()) {
        Q_EMIT disconnected();
    }
}

void PadDevice::openNode(const InputNodeInfo &info)
{
    const QByteArray path = QFile::encodeName(info.devnode);
    const int fd = ::open(path.constData(), O_RDONLY | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        Q_EMIT message(QStringLiteral("cannot open %1: %2").arg(info.devnode, QString::fromLocal8Bit(std::strerror(errno))));
        if (errno == EACCES || errno == ENOENT) {
            scheduleRescan(1000);  // udev may still be applying permissions
        }
        return;
    }
    // Second, independent identity check right before grabbing.
    input_id id{};
    if (::ioctl(fd, EVIOCGID, &id) < 0 || id.vendor != kPadVendor || id.product != kPadProduct) {
        Q_EMIT message(QStringLiteral("refusing %1: evdev id %2:%3 does not match").arg(info.devnode).arg(id.vendor, 4, 16, QLatin1Char('0')).arg(id.product, 4, 16, QLatin1Char('0')));
        ::close(fd);
        return;
    }
    if (m_grab && ::ioctl(fd, EVIOCGRAB, 1) < 0) {
        Q_EMIT message(QStringLiteral("grab %1 failed: %2").arg(info.devnode, QString::fromLocal8Bit(std::strerror(errno))));
        ::close(fd);
        scheduleRescan(2000);
        return;
    }
    auto *n = new Node;
    n->fd = fd;
    n->devnode = info.devnode;
    n->notifier = new QSocketNotifier(fd, QSocketNotifier::Read, this);
    connect(n->notifier, &QSocketNotifier::activated, this, [this, fd] { readNode(fd); });
    m_nodes << n;
    Q_EMIT message(QStringLiteral("%1 %2 (interface %3, serial %4)").arg(m_grab ? QStringLiteral("grabbed") : QStringLiteral("reading"), info.devnode).arg(info.interfaceNumber).arg(info.serial));
}

void PadDevice::emitChords(const QList<ChordEvent> &events)
{
    for (const auto &ce : events) {
        Q_EMIT chordEvent(ce);
        if (auto pe = toPadEvent(m_map, ce)) {
            Q_EMIT padEvent(*pe);
        } else if (ce.down && !m_map.lookup(ce.chord)) {
            Q_EMIT unmappedChord(ce.chord);
        }
    }
}

void PadDevice::closeNode(int index)
{
    Node *n = m_nodes.takeAt(index);
    emitChords(n->decoder.releaseAll());  // unplugged or released: nothing stays held
    n->notifier->setEnabled(false);
    n->notifier->deleteLater();  // may be called from its own activated() signal
    if (m_grab) {
        ::ioctl(n->fd, EVIOCGRAB, 0);
    }
    ::close(n->fd);
    Q_EMIT message(QStringLiteral("released %1").arg(n->devnode));
    delete n;
}

void PadDevice::closeAll()
{
    const bool was = !m_nodes.isEmpty();
    while (!m_nodes.isEmpty()) {
        closeNode(m_nodes.size() - 1);
    }
    if (was) {
        Q_EMIT disconnected();
    }
}

void PadDevice::readNode(int fd)
{
    int index = -1;
    for (int i = 0; i < m_nodes.size(); ++i) {
        if (m_nodes.at(i)->fd == fd) {
            index = i;
        }
    }
    if (index < 0) {
        return;
    }
    Node *n = m_nodes.at(index);
    input_event evs[64];
    while (true) {
        const ssize_t r = ::read(fd, evs, sizeof evs);
        if (r < 0) {
            if (errno == EAGAIN || errno == EINTR) {
                return;
            }
            // ENODEV on unplug: drop the node, udev/poll will bring it back.
            const bool last = m_nodes.size() == 1;
            closeNode(index);
            if (last) {
                Q_EMIT disconnected();
            }
            scheduleRescan(500);
            return;
        }
        if (r == 0) {
            return;
        }
        const int count = int(r / sizeof(input_event));
        for (int i = 0; i < count; ++i) {
            const auto &ev = evs[i];
            if (ev.type == EV_SYN && ev.code == SYN_DROPPED) {
                emitChords(n->decoder.releaseAll());  // events were lost: nothing stays held
                continue;
            }
            const qint64 usec = qint64(ev.input_event_sec) * 1000000 + ev.input_event_usec;
            emitChords(n->decoder.feed(ev.type, ev.code, ev.value, usec));
        }
    }
}

} // namespace cs
