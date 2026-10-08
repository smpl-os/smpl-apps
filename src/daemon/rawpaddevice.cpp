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
    connect(m_heartbeat, &QTimer::timeout, this, &RawPadDevice::onHeartbeat);
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
    if (m_firmware) {
        m_firmware.reset();  // a new device (or the same one replugged): ask again
        Q_EMIT firmwareInfoChanged();
    }
    m_lastSeq = -1;
    m_epoch = -1;
    m_held = 0;
    m_sinceReply.invalidate();
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
    releaseHeld();  // nothing the pad held stays down in the engine
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
        ++m_diag.modeFlips;
        Q_EMIT activeChanged(on);
    }
}

QJsonObject RawDiagnostics::toJson() const
{
    return QJsonObject{{QStringLiteral("events"), qint64(events)},
                       {QStringLiteral("seqGaps"), qint64(seqGaps)},
                       {QStringLiteral("lostEvents"), qint64(lostEvents)},
                       {QStringLiteral("heartbeats"), qint64(heartbeats)},
                       {QStringLiteral("replies"), qint64(replies)},
                       {QStringLiteral("heartbeatMisses"), qint64(heartbeatMisses)},
                       {QStringLiteral("rawDrops"), qint64(rawDrops)},
                       {QStringLiteral("modeFlips"), qint64(modeFlips)},
                       {QStringLiteral("snapshotRequests"), qint64(snapshotRequests)},
                       {QStringLiteral("reconciledDowns"), qint64(reconciledDowns)},
                       {QStringLiteral("reconciledUps"), qint64(reconciledUps)},
                       {QStringLiteral("reconciledDetents"), qint64(reconciledDetents)}};
}

void RawPadDevice::onHeartbeat()
{
    if (!m_info) {
        return;
    }
    if (m_sinceReply.isValid()) {
        const qint64 quiet = m_sinceReply.elapsed();
        if (quiet > 2 * m_heartbeat->interval() + 100) {
            ++m_diag.heartbeatMisses;
        }
        if (quiet > m_rawTimeoutMs && m_active) {
            // The pad has given up raw mode by now and types its keymap, which
            // reaches the daemon through evdev. Release what raw mode held; the
            // next answer starts a new session.
            ++m_diag.rawDrops;
            Q_EMIT message(QStringLiteral("raw input: no answer for %1 ms; the pad uses its keymap until it answers").arg(quiet));
            releaseHeld();
            m_epoch = -1;
            setActive(false);
        }
    }
    send(padfw::rawMode(quint16(m_rawTimeoutMs)));
    ++m_diag.heartbeats;
}

void RawPadDevice::releaseHeld()
{
    const quint32 held = m_held;
    m_held = 0;
    for (int slot = 0; slot < 32; ++slot) {
        if (held & (1u << slot)) {
            ++m_diag.reconciledUps;
            emitSlot(slot, padfw::Up, 1, true);
        }
    }
}

void RawPadDevice::emitSlot(int slot, int event, int count, bool synthetic)
{
    const auto it = m_slots.constFind(slot);
    if (it == m_slots.cend()) {
        return;
    }
    const SlotTarget &t = it.value();
    switch (t.kind) {
    case SlotTarget::Key:
        if (event == padfw::Tap) {
            Q_EMIT padEvent(PadEvent{t.control, PadEvent::KeyDown, 0, 0});
            Q_EMIT padEvent(PadEvent{t.control, PadEvent::KeyUp, 0, 0});
        } else {
            Q_EMIT padEvent(PadEvent{t.control, event == padfw::Down ? PadEvent::KeyDown : PadEvent::KeyUp, 0, 0, synthetic});
        }
        break;
    case SlotTarget::KnobPress:
        if (event != padfw::Tap) {
            Q_EMIT padEvent(PadEvent{t.control, event == padfw::Down ? PadEvent::PressDown : PadEvent::PressUp, 0, 0, synthetic});
        }
        break;
    case SlotTarget::Ccw:
    case SlotTarget::Cw:
        if (event == padfw::Tap) {
            // One event per detent: the engine's coalescing and pacing apply as usual.
            for (int i = 0; i < count; ++i) {
                Q_EMIT padEvent(PadEvent{t.control, PadEvent::Turn, t.kind == SlotTarget::Cw ? 1 : -1, 0});
            }
        }
        break;
    }
}

namespace {
// Firmware slots of the knobs and their detent-total index (k cw = 2k, k ccw = 2k + 1).
constexpr int kFirstKnobSlot = 15;
int detentIndex(int slot)
{
    const int k = (slot - kFirstKnobSlot) / 3, role = (slot - kFirstKnobSlot) % 3;
    return role == 1 ? -1 : 2 * k + (role == 2 ? 0 : 1);
}
int detentSlot(int index)
{
    return kFirstKnobSlot + 3 * (index / 2) + (index % 2 == 0 ? 2 : 0);
}
} // namespace

void RawPadDevice::handleSnapshot(const padfw::Snapshot &s)
{
    ++m_diag.replies;
    m_sinceReply.restart();
    if (m_epoch < 0 || s.epoch != quint8(m_epoch)) {
        if (m_epoch >= 0) {
            // A new raw session: the pad was on its keymap in between (a lost
            // heartbeat). What the old session held is gone.
            ++m_diag.rawDrops;
            Q_EMIT message(QStringLiteral("raw input: the pad left raw mode in between (session %1 -> %2)").arg(m_epoch).arg(s.epoch));
            releaseHeld();
        }
        // Start of a session: take the pad's view as it is. Keys it reports
        // down went down before this daemon followed it; their UP will come.
        m_epoch = s.epoch;
        m_held = s.held;
        m_detentBase = s.detents;
        m_detentRecv.fill(0);
        m_lastSeq = s.seq;
        return;
    }
    // Same session: everything sent before this reply has arrived.
    if (quint8(m_lastSeq) != s.seq) {
        const quint8 lost = quint8(s.seq - quint8(m_lastSeq));
        ++m_diag.seqGaps;
        m_diag.lostEvents += lost;
        m_lastSeq = s.seq;
        Q_EMIT message(QStringLiteral("raw input: %1 event(s) lost before the pad's last answer").arg(lost));
    }
    // Keys and knob presses: the pad's DOWN-without-UP set is the truth.
    const quint32 lostDown = s.held & ~m_held, lostUp = m_held & ~s.held;
    for (int slot = 0; slot < 24; ++slot) {
        if (lostDown & (1u << slot)) {
            ++m_diag.reconciledDowns;
            m_held |= 1u << slot;
            emitSlot(slot, padfw::Down, 1);
        }
        if (lostUp & (1u << slot)) {
            ++m_diag.reconciledUps;
            m_held &= ~(1u << slot);
            emitSlot(slot, padfw::Up, 1);
        }
    }
    if (lostDown || lostUp) {
        Q_EMIT message(QStringLiteral("raw input: restored %1 lost DOWN and %2 lost UP").arg(qPopulationCount(lostDown)).arg(qPopulationCount(lostUp)));
    }
    // Detents: the pad's running totals against what arrived.
    for (std::size_t i = 0; i < s.detents.size(); ++i) {
        const quint8 missing = quint8(s.detents[i] - m_detentBase[i] - m_detentRecv[i]);
        if (missing == 0) {
            continue;
        }
        if (missing >= 128) {
            // More arrived than the pad says it sent: the totals are not this
            // session's. Start counting afresh.
            m_detentBase[i] = s.detents[i];
            m_detentRecv[i] = 0;
            continue;
        }
        m_detentRecv[i] += missing;
        m_diag.reconciledDetents += missing;
        emitSlot(detentSlot(int(i)), padfw::Tap, missing);
        Q_EMIT message(QStringLiteral("raw input: restored %1 lost detent(s)").arg(missing));
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
        m_firmware = info;
        Q_EMIT firmwareInfoChanged();
        if (!info->reliableRaw()) {
            close(QStringLiteral("firmware %1 cannot keep raw mode (2.0.1 left it every 256 ms; fixed in 2.0.2); using evdev chords")
                      .arg(QString::fromStdString(info->version())));
            return;
        }
        if (int(info->slotCount) != m_layout.slotCount()) {
            close(QStringLiteral("firmware reports %1 slots, layout %2 has %3; using evdev").arg(info->slotCount).arg(m_layout.id).arg(m_layout.slotCount()));
            return;
        }
        m_info = info;
        Q_EMIT message(QStringLiteral("raw input: firmware %1, %2 slots, %3 layers").arg(QString::fromStdString(info->version())).arg(info->slotCount).arg(info->layers));
        send(padfw::rawMode(quint16(m_rawTimeoutMs)));
        ++m_diag.heartbeats;
        m_sinceReply.restart();
        m_heartbeat->start();
        return;
    }
    if (m_info && padfw::isReplyTo(data, std::size_t(len), padfw::RawMode)) {
        const bool ok = padfw::replyStatus(data, std::size_t(len)) == padfw::Ok && len > 8 && data[8] == 1;
        if (!ok && m_heartbeat->isActive()) {
            close(QStringLiteral("the firmware refused raw mode"));
            return;
        }
        if (const auto snap = padfw::parseSnapshot(data, std::size_t(len))) {
            handleSnapshot(*snap);
        }
        setActive(true);
        return;
    }
    if (m_info && padfw::isReplyTo(data, std::size_t(len), padfw::GetKeys)) {
        if (const auto snap = padfw::parseSnapshot(data, std::size_t(len)); snap && snap->rawActive) {
            handleSnapshot(*snap);
        }
        return;
    }
    const auto ev = padfw::parseRaw(data, std::size_t(len));
    if (!ev || !m_info) {
        return;  // replies to other tools, keyboard reports before raw mode
    }
    ++m_diag.events;
    if (m_epoch >= 0 && quint8(m_lastSeq + 1) != ev->seq) {
        const quint8 lost = quint8(ev->seq - m_lastSeq - 1);
        ++m_diag.seqGaps;
        m_diag.lostEvents += lost;
        Q_EMIT message(QStringLiteral("raw input: %1 event(s) lost; asking the pad what is down").arg(lost));
        send(padfw::getKeys());  // its answer restores them
        ++m_diag.snapshotRequests;
    }
    m_lastSeq = ev->seq;
    if (ev->slot < 24) {
        const quint32 bit = 1u << ev->slot;
        if (ev->event == padfw::Down) {
            m_held |= bit;
        } else if (ev->event == padfw::Up) {
            m_held &= ~bit;
        } else if (ev->event == padfw::Tap) {
            if (const int i = detentIndex(ev->slot); ev->slot >= kFirstKnobSlot && i >= 0) {
                m_detentRecv[std::size_t(i)] += ev->count;
            }
        }
    }
    emitSlot(ev->slot, ev->event, ev->count);
}

} // namespace cs
