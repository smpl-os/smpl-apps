// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include "boardprofile.h"
#include "config.h"
#include "decoder.h"
#include "padfwproto.h"

#include <QHash>
#include <QObject>
#include <optional>

class QSocketNotifier;
class QTimer;

namespace cs {

// Raw input from the control-surface firmware (report 5) over hidraw. The
// daemon keeps raw mode alive with heartbeats; if it stops (crash, hang,
// unplug), the firmware falls back to its own keymap by itself. The evdev
// grab stays in place as a second line of defence.
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
    std::optional<padfw::Info> info() const { return m_info; }
    // The firmware's GET_INFO answer, kept even when the layout does not fit
    // (raw mode off): the full version (bcdDevice has only major.minor).
    std::optional<padfw::Info> firmwareInfo() const { return m_firmware; }
    quint64 sequenceGaps() const { return m_gaps; }

Q_SIGNALS:
    void padEvent(const cs::PadEvent &e);
    void activeChanged(bool active);
    void firmwareInfoChanged();
    void message(const QString &text);

private:
    void onReadable();
    void handleReport(const quint8 *data, int len);
    void send(const padfw::Request &r);
    void setActive(bool on);
    void close(const QString &why);
    void rebuildSlots();

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
    quint64 m_gaps = 0;
};

} // namespace cs
