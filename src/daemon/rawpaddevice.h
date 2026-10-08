// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "boardprofile.h"
#include "config.h"
#include "decoder.h"
#include "padfwproto.h"

#include <QElapsedTimer>
#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <array>
#include <optional>

class QSocketNotifier;
class QTimer;

namespace cs {

// Raw input from the control-surface firmware (report 5) over hidraw. The
// daemon keeps raw mode alive with heartbeats; if it stops (crash, hang,
// unplug), the firmware falls back to its own keymap by itself. The evdev
// grab stays in place as a second line of defence.
//
// Keyboard-reliable (firmware 2.0.2+): every heartbeat reply is a snapshot of
// the raw session (epoch, last sequence number, slots held down, detent
// totals). Raw events sent before it have all arrived, so anything missing
// is restored at once: a lost UP is sent, a lost DOWN too, lost detents are
// turned. A new epoch means the pad left raw mode meanwhile: whatever this
// session held is released. Older firmware is refused (2.0.1 left raw mode
// every 256 ms).

// Counters for status --json ("input") and the stress test.
struct RawDiagnostics {
    quint64 events = 0;            // raw reports received (DOWN, UP, TAP)
    quint64 seqGaps = 0;           // breaks in the sequence numbers
    quint64 lostEvents = 0;        // events those breaks skipped
    quint64 heartbeats = 0;        // RAW_MODE requests sent
    quint64 replies = 0;           // snapshots received (RAW_MODE and GET_KEYS replies)
    quint64 heartbeatMisses = 0;   // heartbeat ticks more than two intervals after the last reply
    quint64 rawDrops = 0;          // the pad left raw mode (new epoch, or silence past the timeout)
    quint64 modeFlips = 0;         // raw <-> keymap switches seen by the daemon
    quint64 snapshotRequests = 0;  // GET_KEYS sent after a gap
    quint64 reconciledDowns = 0, reconciledUps = 0, reconciledDetents = 0;
    QJsonObject toJson() const;
};
// The hidraw node of a pad running the control-surface firmware (report IDs 3
// and 5 in its descriptor), optionally only the pad at usbPath. Empty if none.
// Only sysfs is read; sysRoot lets tests point at a fake tree.
QString findControlSurfaceHidraw(const DeviceMatch &match, const QString &usbPath = {}, const QString &sysRoot = {});

class RawPadDevice : public QObject
{
    Q_OBJECT
public:
    explicit RawPadDevice(const DeviceMatch &match, QObject *parent = nullptr);
    ~RawPadDevice() override;

    void setLayout(const BoardProfile &layout);
    void setTiming(int heartbeatMs, int rawTimeoutMs, int infoTimeoutMs);
    // Find the hidraw node of the pad at usbPath (the one the evdev side holds;
    // any matching pad if empty), ask GET_INFO, then enter raw mode.
    bool start(const QString &usbPath = {});
    bool startOnFd(int fd);   // tests: an already open hidraw-like fd (taken over)
    void stop();              // raw mode off (best effort) and close
    bool isActive() const { return m_active; }
    bool isOpen() const { return m_fd >= 0; }  // started (asking for, or in, raw mode)
    std::optional<padfw::Info> info() const { return m_info; }
    // The firmware's GET_INFO answer, kept even when the layout does not fit
    // (raw mode off): the full version (bcdDevice has only major.minor).
    std::optional<padfw::Info> firmwareInfo() const { return m_firmware; }
    quint64 sequenceGaps() const { return m_diag.seqGaps; }
    const RawDiagnostics &diagnostics() const { return m_diag; }

Q_SIGNALS:
    void padEvent(const cs::PadEvent &e);
    void activeChanged(bool active);
    void firmwareInfoChanged();
    void message(const QString &text);

private:
    void onReadable();
    void handleReport(const quint8 *data, int len);
    void handleSnapshot(const padfw::Snapshot &s);
    void onHeartbeat();
    void send(const padfw::Request &r);
    void setActive(bool on);
    void close(const QString &why);
    void rebuildSlots();
    // DOWN/UP/TAP for a firmware slot as the engine's events; count for TAP.
    void emitSlot(int slot, int event, int count);
    void releaseHeld();  // UP for everything this session holds down

    struct SlotTarget {
        QString control;
        enum Kind { Key, KnobPress, Ccw, Cw } kind = Key;
    };
    DeviceMatch m_match;
    BoardProfile m_layout;
    QHash<int, SlotTarget> m_slots;
    int m_fd = -1;
    QSocketNotifier *m_notifier = nullptr;
    QTimer *m_heartbeat;
    QTimer *m_infoTimer;
    int m_rawTimeoutMs = 1500;
    bool m_active = false;
    std::optional<padfw::Info> m_info;
    std::optional<padfw::Info> m_firmware;
    int m_lastSeq = -1;
    int m_epoch = -1;                     // raw session the daemon follows; -1 before the first snapshot
    quint32 m_held = 0;                   // bit s: DOWN delivered for slot s, UP not yet
    std::array<quint8, 6> m_detentBase{}; // the snapshot's detent totals at the session's start
    std::array<quint8, 6> m_detentRecv{}; // detents received (or restored) since, mod 256
    QElapsedTimer m_sinceReply;
    RawDiagnostics m_diag;
};

} // namespace cs
