// SPDX-License-Identifier: GPL-2.0-or-later
#include "rawpaddevice.h"
#include "hidrawdev.h"

#include <QFile>
#include <QSocketNotifier>
#include <QTimer>
#include <algorithm>
#include <cerrno>
#include <cstring>
#include <fcntl.h>
#include <sys/socket.h>
#include <unistd.h>

namespace cs {

RawPadDevice::RawPadDevice(const DeviceMatch &match, QObject *parent)
    : QObject(parent)
    , m_match(match)
    , m_heartbeat(new QTimer(this))
    , m_infoTimer(new QTimer(this))
{
    m_heartbeat->setInterval(500);
    connect(m_heartbeat, &QTimer::timeout, this, [this] { send(padfw::rawMode(quint16(m_rawTimeoutMs))); });
    m_infoTimer->setSingleShot(true);
    m_infoTimer->setInterval(500);
    connect(m_infoTimer, &QTimer::timeout, this, [this] { close(QStringLiteral("no answer to GET_INFO: not the control-surface firmware")); });
    m_layout = *builtinBoardProfile(QStringLiteral("sy181-15k3e"));
    rebuildSlots();
}

RawPadDevice::~RawPadDevice()
{
    stop();
}

void RawPadDevice::setLayout(const BoardProfile &layout)
{
    m_layout = layout;
    rebuildSlots();
    // The same check as at GET_INFO: a slot map that does not fit the firmware
    // would send inputs to the wrong controls. Back to evdev instead.
    if (m_info && int(m_info->slotCount) != m_layout.slotCount()) {
        if (m_active) {
            send(padfw::rawMode(0));
        }
        close(QStringLiteral("firmware reports %1 slots, layout %2 has %3; using evdev").arg(m_info->slotCount).arg(m_layout.id).arg(m_layout.slotCount()));
    }
}

void RawPadDevice::setTiming(int heartbeatMs, int rawTimeoutMs, int infoTimeoutMs)
{
    m_heartbeat->setInterval(heartbeatMs);
    m_rawTimeoutMs = std::clamp(rawTimeoutMs, 1, int(padfw::kRawTimeoutMaxMs));
    m_infoTimer->setInterval(infoTimeoutMs);
}

void RawPadDevice::rebuildSlots()
{
    m_slots.clear();
    for (const BoardKey &k : m_layout.keys) {
        m_slots.insert(k.slot, SlotTarget{k.control, SlotTarget::Key});
    }
    for (const BoardKnob &k : m_layout.knobs) {
        m_slots.insert(k.ccw, SlotTarget{k.control, SlotTarget::Ccw});
        m_slots.insert(k.press, SlotTarget{k.control, SlotTarget::KnobPress});
        m_slots.insert(k.cw, SlotTarget{k.control, SlotTarget::Cw});
    }
}

QString findControlSurfaceHidraw(const DeviceMatch &match, const QString &usbPath, const QString &sysRoot)
{
    const auto nodes = ch552::findPadHidraw(match.vendor.toStdString(), match.product.toStdString(), match.serial.toStdString(), sysRoot.toStdString());
    const QString port = usbPath.section(QLatin1Char('/'), -1);  // "1-5": sysfs paths differ, the port name does not
    for (const auto &n : nodes) {
        if (!port.isEmpty() && QString::fromStdString(n.usbPath).section(QLatin1Char('/'), -1) != port) {
            continue;
        }
        const auto d = ch552::parseDescriptor(n.descriptor);
        const bool hasCfg = std::find(d.reportIds.begin(), d.reportIds.end(), int(padfw::kCfgReport)) != d.reportIds.end();
        const bool hasRaw = std::find(d.reportIds.begin(), d.reportIds.end(), int(padfw::kRawReport)) != d.reportIds.end();
        if (d.ok && hasCfg && hasRaw) {
            return QString::fromStdString(n.devnode);  // stock or older firmware lacks report 5
        }
    }
    return {};
}

bool RawPadDevice::start(const QString &usbPath)
{
    if (m_fd >= 0) {
        return true;
    }
    const QString node = findControlSurfaceHidraw(m_match, usbPath);
    if (node.isEmpty()) {
        return false;
    }
    const int fd = ::open(QFile::encodeName(node).constData(), O_RDWR | O_NONBLOCK | O_CLOEXEC);
    if (fd < 0) {
        Q_EMIT message(QStringLiteral("raw input: cannot open %1: %2").arg(node, QString::fromLocal8Bit(std::strerror(errno))));
        return false;
    }
    return startOnFd(fd);
}

bool RawPadDevice::startOnFd(int fd)
{
    stop();
    m_fd = fd;
    m_info.reset();
    m_lastSeq = -1;
    m_notifier = new QSocketNotifier(m_fd, QSocketNotifier::Read, this);
    connect(m_notifier, &QSocketNotifier::activated, this, &RawPadDevice::onReadable);
    send(padfw::getInfo());
    m_infoTimer->start();
    return true;
}

void RawPadDevice::stop()
{
    if (m_fd < 0) {
        return;
    }
    if (m_active) {
        send(padfw::rawMode(0));  // back to the keymap now rather than at the timeout
    }
    close(QString());
}

void RawPadDevice::close(const QString &why)
{
    m_heartbeat->stop();
    m_infoTimer->stop();
    if (m_notifier) {
        m_notifier->setEnabled(false);
        m_notifier->deleteLater();
        m_notifier = nullptr;
    }
    if (m_fd >= 0) {
        ::close(m_fd);
        m_fd = -1;
    }
    if (!why.isEmpty()) {
        Q_EMIT message(QStringLiteral("raw input: %1").arg(why));
    }
    setActive(false);
}

void RawPadDevice::setActive(bool on)
{
    if (on != m_active) {
        m_active = on;
        Q_EMIT activeChanged(on);
    }
}

void RawPadDevice::send(const padfw::Request &r)
{
    if (m_fd < 0) {
        return;
    }
    // hidraw is a character device; tests use a socket, where a closed peer
    // must not raise SIGPIPE. ENODEV after an unplug: the read side closes.
    ssize_t n = ::send(m_fd, r.data(), r.size(), MSG_NOSIGNAL);
    if (n < 0 && errno == ENOTSOCK) {
        n = ::write(m_fd, r.data(), r.size());
    }
    Q_UNUSED(n)
}

void RawPadDevice::onReadable()
{
    quint8 buf[64];
    for (;;) {
        const ssize_t n = ::read(m_fd, buf, sizeof buf);
        if (n > 0) {
            handleReport(buf, int(n));
            if (m_fd < 0) {
                return;  // closed while handling
            }
            continue;
        }
        if (n < 0 && (errno == EAGAIN || errno == EINTR)) {
            return;
        }
        close(n == 0 ? QStringLiteral("device closed") : QStringLiteral("read: %1").arg(QString::fromLocal8Bit(std::strerror(errno))));
        return;
    }
}

void RawPadDevice::handleReport(const quint8 *data, int len)
{
    if (!m_info && padfw::isReplyTo(data, std::size_t(len), padfw::GetInfo)) {
        m_infoTimer->stop();
        const auto info = padfw::parseInfo(data, std::size_t(len));
        if (!info) {
            close(QStringLiteral("GET_INFO answer is not protocol v3"));
            return;
        }
        if (int(info->slotCount) != m_layout.slotCount()) {
            close(QStringLiteral("firmware reports %1 slots, layout %2 has %3; using evdev").arg(info->slotCount).arg(m_layout.id).arg(m_layout.slotCount()));
            return;
        }
        m_info = info;
        Q_EMIT message(QStringLiteral("raw input: firmware %1, %2 slots, %3 layers").arg(QString::fromStdString(info->version())).arg(info->slotCount).arg(info->layers));
        send(padfw::rawMode(quint16(m_rawTimeoutMs)));
        m_heartbeat->start();
        return;
    }
    if (m_info && padfw::isReplyTo(data, std::size_t(len), padfw::RawMode)) {
        const bool ok = padfw::replyStatus(data, std::size_t(len)) == padfw::Ok && len > 8 && data[8] == 1;
        if (!ok && m_heartbeat->isActive()) {
            close(QStringLiteral("the firmware refused raw mode"));
            return;
        }
        setActive(true);
        return;
    }
    const auto ev = padfw::parseRaw(data, std::size_t(len));
    if (!ev || !m_active) {
        return;  // replies to other tools, keyboard reports before raw mode
    }
    if (m_lastSeq >= 0 && quint8(m_lastSeq + 1) != ev->seq) {
        ++m_gaps;
        Q_EMIT message(QStringLiteral("raw input: %1 event(s) lost").arg(quint8(ev->seq - m_lastSeq - 1)));
    }
    m_lastSeq = ev->seq;
    const auto it = m_slots.constFind(ev->slot);
    if (it == m_slots.cend()) {
        return;
    }
    const SlotTarget &t = it.value();
    switch (t.kind) {
    case SlotTarget::Key:
        if (ev->event == padfw::Tap) {
            Q_EMIT padEvent(PadEvent{t.control, PadEvent::KeyDown, 0, 0});
            Q_EMIT padEvent(PadEvent{t.control, PadEvent::KeyUp, 0, 0});
        } else {
            Q_EMIT padEvent(PadEvent{t.control, ev->event == padfw::Down ? PadEvent::KeyDown : PadEvent::KeyUp, 0, 0});
        }
        break;
    case SlotTarget::KnobPress:
        if (ev->event != padfw::Tap) {
            Q_EMIT padEvent(PadEvent{t.control, ev->event == padfw::Down ? PadEvent::PressDown : PadEvent::PressUp, 0, 0});
        }
        break;
    case SlotTarget::Ccw:
    case SlotTarget::Cw:
        if (ev->event == padfw::Tap) {
            Q_EMIT padEvent(PadEvent{t.control, PadEvent::Turn, t.kind == SlotTarget::Cw ? 1 : -1, 0});
        }
        break;
    }
}

} // namespace cs
