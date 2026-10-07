// SPDX-License-Identifier: GPL-2.0-or-later
#include "paddevice.h"

#include <QDir>
#include <QFile>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

namespace {
void write(const QString &path, const QByteArray &data)
{
    QDir().mkpath(QFileInfo(path).path());
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(data);
}

// <root>/sys/devices/<usb>/<usb>:1.<ifn>/0003:<vid>:<pid>.000N/input/inputM/eventK + class symlink
void addNode(const QString &root, const QString &usb, const QByteArray &vid, const QByteArray &pid, const QByteArray &serial, int ifn, int event)
{
    const QString dev = root + QStringLiteral("/sys/devices/pci0/") + usb;
    write(dev + QStringLiteral("/idVendor"), vid + '\n');
    write(dev + QStringLiteral("/idProduct"), pid + '\n');
    write(dev + QStringLiteral("/serial"), serial + '\n');
    const QString intf = dev + QStringLiteral("/%1:1.%2").arg(usb).arg(ifn);
    write(intf + QStringLiteral("/bInterfaceNumber"), QByteArray::number(ifn).rightJustified(2, '0') + '\n');
    const QString input = intf + QStringLiteral("/0003:%1:%2.%3/input/input%4").arg(QString::fromLatin1(vid), QString::fromLatin1(pid)).arg(event).arg(event);
    const QString ev = input + QStringLiteral("/event%1").arg(event);
    QDir().mkpath(ev);
    QFile::link(input, ev + QStringLiteral("/device"));
    QDir().mkpath(root + QStringLiteral("/sys/class/input"));
    QFile::link(ev, root + QStringLiteral("/sys/class/input/event%1").arg(event));
}
} // namespace

class TestPadDevice : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void discoveryMatchesOnlyThePad()
    {
        QTemporaryDir root;
        addNode(root.path(), QStringLiteral("1-5"), "1189", "8890", "key153", 0, 18);
        addNode(root.path(), QStringLiteral("1-5"), "1189", "8890", "key153", 1, 19);
        addNode(root.path(), QStringLiteral("1-5"), "1189", "8890", "key153", 2, 20);
        addNode(root.path(), QStringLiteral("1-5"), "1189", "8890", "key153", 3, 21);
        addNode(root.path(), QStringLiteral("1-2"), "0c45", "760a", "", 0, 3);       // the user's main keyboard
        addNode(root.path(), QStringLiteral("1-7"), "1189", "8890", "other", 0, 30);  // a second pad
        DeviceMatch m;
        m.serial = QStringLiteral("key153");
        const auto nodes = findPadInputNodes(m, root.path(), QStringLiteral("/dev/input"));
        QStringList names;
        for (const auto &n : nodes) {
            names << n.sysName;
            QCOMPARE(n.vendor, QStringLiteral("1189"));
            QCOMPARE(n.serial, QStringLiteral("key153"));
        }
        QCOMPARE(names, (QStringList{QStringLiteral("event18"), QStringLiteral("event19"), QStringLiteral("event20"), QStringLiteral("event21")}));
        QCOMPARE(nodes[2].interfaceNumber, 2);
        m.serial.clear();
        QCOMPARE(findPadInputNodes(m, root.path()).size(), 5);  // no serial: both pads, never the keyboard
        m.vendor = QStringLiteral("0c45");
        m.product = QStringLiteral("760a");
        QCOMPARE(findPadInputNodes(DeviceMatch{}, QStringLiteral("/nonexistent")).size(), 0);
    }

    void realPadGrabAndRelease()
    {
        // Opt-in: touches the real pad (grab + release only; nothing is written).
        if (qEnvironmentVariable("CS_TEST_REAL_PAD") != QLatin1String("1")) {
            QSKIP("set CS_TEST_REAL_PAD=1 to grab the attached pad");
        }
        DeviceMatch m;
        m.serial = QStringLiteral("key153");
        const auto found = findPadInputNodes(m);
        if (found.isEmpty()) {
            QSKIP("pad not attached");
        }
        PadDevice dev(m);
        QSignalSpy conn(&dev, &PadDevice::connected);
        QStringList msgs;
        connect(&dev, &PadDevice::message, this, [&msgs](const QString &s) { msgs << s; });
        dev.start();
        QTRY_COMPARE(conn.size(), 1);
        QCOMPARE(dev.devnodes().size(), found.size());
        QCOMPARE(msgs.filter(QStringLiteral("grabbed")).size(), found.size());
        dev.stop();
        QVERIFY(!dev.isConnected());
        QCOMPARE(msgs.filter(QStringLiteral("released")).size(), found.size());
    }
};

QTEST_GUILESS_MAIN(TestPadDevice)
#include "tst_paddevice.moc"
