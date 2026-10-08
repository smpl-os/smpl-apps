// SPDX-License-Identifier: GPL-2.0-or-later
// Finds the pad's evdev nodes by USB VID:PID (and serial), grabs them
// exclusively and re-attaches after unplug/replug. Nothing else is ever opened.
#pragma once

#include "config.h"
#include "decoder.h"

#include <QList>
#include <QObject>
#include <QStringList>

class QSocketNotifier;
class QTimer;
struct udev;
struct udev_monitor;

namespace cs {

struct InputNodeInfo {
    QString devnode;  // /dev/input/eventN
    QString sysName;  // eventN
    QString usbPath;
    QString vendor, product, serial;
    int interfaceNumber = -1;
};

// sysRoot/devRoot let tests use a fake tree.
QList<InputNodeInfo> findPadInputNodes(const DeviceMatch &m, const QString &sysRoot = {}, const QString &devRoot = {});
// The nodes of one pad: the one at preferUsbPath if it is still there, else
// the first by USB path. With an empty serial several pads may match.
QList<InputNodeInfo> onePad(const QList<InputNodeInfo> &nodes, const QString &preferUsbPath = {});

class PadDevice : public QObject
{
    Q_OBJECT
public:
    explicit PadDevice(const DeviceMatch &match, QObject *parent = nullptr);
    ~PadDevice() override;

    void setHardwareMap(const HardwareMap &map) { m_map = map; }
    void setGrab(bool grab) { m_grab = grab; }
    void start();
    void stop();
    bool isConnected() const { return !m_nodes.isEmpty(); }
    QStringList devnodes() const;
    QString usbPath() const { return m_usbPath; }

Q_SIGNALS:
    void padEvent(const cs::PadEvent &e);
    void chordEvent(const cs::ChordEvent &e);
    void unmappedChord(const cs::KeyChord &c);
    void connected(const QStringList &devnodes);
    void disconnected();
    void message(const QString &text);

private:
    struct Node {
        int fd = -1;
        QString devnode;
        QSocketNotifier *notifier = nullptr;
        Decoder decoder;
    };
    void rescan();
    void scheduleRescan(int ms);
    void openNode(const InputNodeInfo &info);
    void closeNode(int index);
    void emitChords(const QList<ChordEvent> &events);
    void closeAll();
    void readNode(int fd);
    void onUdev();

    DeviceMatch m_match;
    QString m_usbPath;
    HardwareMap m_map = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
    bool m_grab = true;
    QList<Node *> m_nodes;
    QTimer *m_rescan = nullptr;
    QTimer *m_poll = nullptr;
    udev *m_udev = nullptr;
    udev_monitor *m_monitor = nullptr;
    QSocketNotifier *m_udevNotifier = nullptr;
};

} // namespace cs
