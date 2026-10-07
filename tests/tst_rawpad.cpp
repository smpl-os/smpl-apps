// SPDX-License-Identifier: GPL-2.0-or-later
// Raw input backend against a simulated control-surface firmware on a
// socketpair (SOCK_SEQPACKET keeps report boundaries, like hidraw).
#include "padfwproto.h"
#include "hidrawdev.h"
#include "rawpaddevice.h"

#include <QDir>
#include <QFile>
#include <QRegularExpression>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QSocketNotifier>
#include <QTest>
#include <sys/socket.h>
#include <thread>
#include <unistd.h>

// Protocol constants must match the firmware's own headers.
extern "C" {
#include "padstore.h"
}
static_assert(padfw::kCfgReport == CFG_REPORT_ID);
static_assert(padfw::kRawReport == RAW_REPORT_ID);
static_assert(padfw::GetInfo == CMD_GET_INFO && padfw::GetAction == CMD_GET_ACTION && padfw::SetAction == CMD_SET_ACTION);
static_assert(padfw::Reset == CMD_RESET && padfw::Bootloader == CMD_BOOTLOADER && padfw::Dump == CMD_DUMP);
static_assert(padfw::RawMode == CMD_RAW_MODE && padfw::SetLayer == CMD_SET_LAYER && padfw::GetStats == CMD_GET_STATS);
static_assert(padfw::Ok == ST_OK && padfw::BadArg == ST_BAD_ARG && padfw::Unknown == ST_UNKNOWN);
static_assert(padfw::Down == RAW_EVT_DOWN && padfw::Up == RAW_EVT_UP && padfw::Tap == RAW_EVT_TAP);
static_assert(padfw::kRawTimeoutMaxMs == RAW_TIMEOUT_MAX_MS);
static_assert(SLOT_KNOB(0, KNOB_CCW) == 15 && SLOT_KNOB(2, KNOB_CW) == 23 && SLOT_COUNT == 24);

using namespace cs;

// The pad's side of the socketpair.
class FakeFirmware : public QObject
{
    Q_OBJECT
public:
    explicit FakeFirmware(int fd) : m_fd(fd), m_n(new QSocketNotifier(fd, QSocketNotifier::Read, this))
    {
        connect(m_n, &QSocketNotifier::activated, this, &FakeFirmware::onRequest);
    }
    ~FakeFirmware() override { closeLink(); }
    void closeLink()
    {
        if (m_fd >= 0) {
            m_n->setEnabled(false);
            ::close(m_fd);
            m_fd = -1;
        }
    }
    bool send(std::initializer_list<int> bytes)
    {
        QByteArray b;
        for (int v : bytes) {
            b.append(char(v));
        }
        // A reply can race the daemon closing its end: no SIGPIPE, just false.
        return m_fd >= 0 && ::send(m_fd, b.constData(), size_t(b.size()), MSG_NOSIGNAL) == b.size();
    }
    void raw(int seq, int slot, int event, int layer = 0, int count = 1) { QVERIFY(send({5, seq, slot, event, layer, count})); }

    bool answerInfo = true;
    QByteArray magic = "CS";
    int slotCount = 24;
    int rawStatus = 1;
    QList<QByteArray> requests;
    QList<int> rawTimeouts;

private Q_SLOTS:
    void onRequest()
    {
        char buf[64];
        const ssize_t n = ::read(m_fd, buf, sizeof buf);
        if (n <= 0) {
            return;
        }
        const QByteArray req(buf, int(n));
        requests << req;
        QCOMPARE(req.size(), 16);
        QCOMPARE(int(quint8(req[0])), 3);
        const int cmd = quint8(req[1]);
        if (cmd == 1 && answerInfo) {
            send({3, 1, magic[0], magic[1], 3, slotCount, 128, 1, 2, 0, 0, 2, 0, 0, 0, 0});
        } else if (cmd == 8) {
            const int t = quint8(req[2]) | (quint8(req[3]) << 8);
            rawTimeouts << t;
            send({3, 8, 0, 0, 0, 0, 0, rawStatus, (rawStatus == 1 && t) ? 1 : 0, 0, 0, 0, 0, 0, 0, 0});
        }
    }

private:
    int m_fd;
    QSocketNotifier *m_n;
};

class TestRawPad : public QObject
{
    Q_OBJECT
    int m_pair[2] = {-1, -1};

    std::unique_ptr<FakeFirmware> link(RawPadDevice &dev)
    {
        if (::socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_NONBLOCK | SOCK_CLOEXEC, 0, m_pair) != 0) {
            return {};
        }
        auto fw = std::make_unique<FakeFirmware>(m_pair[1]);
        dev.setTiming(20, 1500, 200);
        return fw;
    }

private Q_SLOTS:
    void protocolCodec()
    {
        const auto r = padfw::rawMode(1500);
        QCOMPARE(int(r[0]), 3);
        QCOMPARE(int(r[1]), 8);
        QCOMPARE(int(r[2] | (r[3] << 8)), 1500);
        QCOMPARE(int(padfw::bootloader()[2]), int('B'));
        QCOMPARE(int(padfw::bootloader()[3]), int('L'));
        const quint8 info[] = {3, 1, 'C', 'S', 3, 24, 128, 1, 2, 0, 0, 2, 1, 0, 1, 0};
        const auto i = padfw::parseInfo(info, sizeof info);
        QVERIFY(i);
        QCOMPARE(i->version(), std::string("2.0.0"));
        QCOMPARE(int(i->slotCount), 24);
        QCOMPARE(int(i->activeLayer), 1);
        QCOMPARE(int(i->startLayer), 1);
        quint8 bad[sizeof info];
        memcpy(bad, info, sizeof info);
        bad[3] = 'M';  // EpicLPer's v2 magic
        QVERIFY(!padfw::parseInfo(bad, sizeof bad));
        QVERIFY(!padfw::parseInfo(info, 10));
        const quint8 ev[] = {5, 9, 16, 1, 0};
        QCOMPARE(int(padfw::parseRaw(ev, 5)->slot), 16);
        const quint8 tap6[] = {5, 9, 17, 3, 0, 12};
        QCOMPARE(int(padfw::parseRaw(tap6, 6)->count), 12);
        const quint8 down6[] = {5, 9, 16, 1, 0, 9};  // count only means something for taps
        QCOMPARE(int(padfw::parseRaw(down6, 6)->count), 1);
        const quint8 tap0[] = {5, 9, 17, 3, 0, 0};
        QCOMPARE(int(padfw::parseRaw(tap0, 6)->count), 1);
        const quint8 stats[] = {3, 0x0A, 1, 5, 0, 0, 0, 1, 2, 0, 0, 0, 0, 0, 0x10, 0x01};
        const auto w = padfw::parseStatsPage(stats, sizeof stats);
        QVERIFY(w);
        QCOMPARE(int((*w)[0]), 5);
        QCOMPARE(int((*w)[2]), 2);
        QCOMPARE(int((*w)[5]), 0x110);
        quint8 refused[sizeof stats];
        memcpy(refused, stats, sizeof stats);
        refused[7] = 5;
        QVERIFY(!padfw::parseStatsPage(refused, sizeof refused));
        const quint8 junk[] = {5, 9, 16, 7, 0};
        QVERIFY(!padfw::parseRaw(junk, 5));
        const quint8 kbd[] = {1, 0, 0, 0x69, 0, 0, 0, 0, 0};
        QVERIFY(!padfw::parseRaw(kbd, sizeof kbd));
    }

    void entersRawModeAndMapsSlots()
    {
        RawPadDevice dev{DeviceMatch{}};
        auto fw = link(dev);
        QVERIFY(fw);
        QSignalSpy active(&dev, &RawPadDevice::activeChanged);
        QSignalSpy events(&dev, &RawPadDevice::padEvent);
        fw->raw(1, 0, 1);  // before raw mode: ignored
        QVERIFY(dev.startOnFd(m_pair[0]));
        QTRY_COMPARE(active.count(), 1);
        QVERIFY(dev.isActive());
        QCOMPARE(dev.info()->version(), std::string("2.0.0"));
        QCOMPARE(fw->rawTimeouts.first(), 1500);
        QCOMPARE(events.count(), 0);

        fw->raw(1, 6, 1);    // key7 down
        fw->raw(2, 6, 2);    // key7 up
        fw->raw(3, 15, 3);   // knob1 ccw
        fw->raw(4, 17, 3);   // knob1 cw
        fw->raw(5, 19, 1);   // knob2 press
        fw->raw(6, 19, 2);
        fw->raw(7, 30, 1);   // no such slot
        fw->raw(8, 22, 3);   // a tap on a press slot means nothing
        QTRY_COMPARE(events.count(), 6);
        fw->raw(9, 20, 3, 0, 4);   // 2.0.1: four knob2 cw detents in one report
        QTRY_COMPARE(events.count(), 10);
        for (int i = 6; i < 10; ++i) {
            QCOMPARE(events.at(i).at(0).value<PadEvent>().control, QStringLiteral("knob2"));
            QCOMPARE(events.at(i).at(0).value<PadEvent>().delta, 1);
        }
        QVERIFY(fw->send({5, 10, 18, 3, 0}));   // 2.0.0 report without the count byte
        QTRY_COMPARE(events.count(), 11);
        QCOMPARE(events.last().at(0).value<PadEvent>().delta, -1);
        auto at = [&](int i) { return events.at(i).at(0).value<PadEvent>(); };
        QCOMPARE(at(0).control, QStringLiteral("key7"));
        QCOMPARE(at(0).type, PadEvent::KeyDown);
        QCOMPARE(at(1).type, PadEvent::KeyUp);
        QCOMPARE(at(2).control, QStringLiteral("knob1"));
        QCOMPARE(at(2).type, PadEvent::Turn);
        QCOMPARE(at(2).delta, -1);
        QCOMPARE(at(3).delta, 1);
        QCOMPARE(at(4).control, QStringLiteral("knob2"));
        QCOMPARE(at(4).type, PadEvent::PressDown);
        QCOMPARE(at(5).type, PadEvent::PressUp);
        QCOMPARE(dev.sequenceGaps(), 0u);

        // Heartbeats keep coming.
        const int before = fw->rawTimeouts.size();
        QTRY_VERIFY(fw->rawTimeouts.size() >= before + 3);

        // A lost report is noticed.
        QSignalSpy msgs(&dev, &RawPadDevice::message);
        fw->raw(13, 0, 1);
        QTRY_COMPARE(dev.sequenceGaps(), 1u);
        QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("2 event(s) lost")));

        // stop() switches raw mode off right away.
        dev.stop();
        QTRY_COMPARE(fw->rawTimeouts.last(), 0);
        QVERIFY(!dev.isActive());
        QCOMPARE(active.count(), 2);
    }

    void refusesOtherFirmware()
    {
        {  // EpicLPer's protocol v2 answers with another magic
            RawPadDevice dev{DeviceMatch{}};
            auto fw = link(dev);
            fw->magic = "OM";
            QSignalSpy msgs(&dev, &RawPadDevice::message);
            dev.startOnFd(m_pair[0]);
            QTRY_VERIFY(!msgs.isEmpty());
            QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("not protocol v3")));
            QVERIFY(!dev.isActive());
            QVERIFY(fw->rawTimeouts.isEmpty());  // never asked for raw mode
        }
        {  // no answer at all (stock firmware)
            RawPadDevice dev{DeviceMatch{}};
            auto fw = link(dev);
            fw->answerInfo = false;
            QSignalSpy msgs(&dev, &RawPadDevice::message);
            dev.startOnFd(m_pair[0]);
            QTRY_VERIFY(!msgs.isEmpty());
            QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("no answer")));
            QVERIFY(fw->rawTimeouts.isEmpty());
        }
        {  // a board with another slot count than the layout
            RawPadDevice dev{DeviceMatch{}};
            auto fw = link(dev);
            fw->slotCount = 6;
            QSignalSpy msgs(&dev, &RawPadDevice::message);
            QSignalSpy fwInfo(&dev, &RawPadDevice::firmwareInfoChanged);
            dev.startOnFd(m_pair[0]);
            QTRY_VERIFY(!msgs.isEmpty());
            QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("6 slots")));
            QVERIFY(fw->rawTimeouts.isEmpty());
            // Raw mode is off, but the firmware's answer stays for the device
            // report (the full version; bcdDevice has only major.minor).
            QVERIFY(!dev.info());
            QCOMPARE(fwInfo.count(), 1);
            QVERIFY(dev.firmwareInfo());
            QCOMPARE(dev.firmwareInfo()->version(), std::string("2.0.0"));
            QCOMPARE(int(dev.firmwareInfo()->slotCount), 6);
        }
        {  // ... unless the layout says so
            RawPadDevice dev{DeviceMatch{}};
            dev.setLayout(*builtinBoardProfile(QStringLiteral("generic-3k1e")));
            auto fw = link(dev);
            fw->slotCount = 6;
            QSignalSpy active(&dev, &RawPadDevice::activeChanged);
            QSignalSpy events(&dev, &RawPadDevice::padEvent);
            dev.startOnFd(m_pair[0]);
            QTRY_COMPARE(active.count(), 1);
            fw->raw(1, 5, 3);  // knob1 cw on a 3+1 pad
            QTRY_COMPARE(events.count(), 1);
            QCOMPARE(events.first().at(0).value<PadEvent>().control, QStringLiteral("knob1"));
            QCOMPARE(events.first().at(0).value<PadEvent>().delta, 1);
            dev.stop();
        }
        {  // raw mode refused
            RawPadDevice dev{DeviceMatch{}};
            auto fw = link(dev);
            fw->rawStatus = 5;
            QSignalSpy msgs(&dev, &RawPadDevice::message);
            dev.startOnFd(m_pair[0]);
            QTRY_VERIFY(msgs.size() >= 2);
            QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("refused raw mode")));
            QVERIFY(!dev.isActive());
        }
    }

    void layoutChangeThatDoesNotFitEndsRawMode()
    {
        RawPadDevice dev{DeviceMatch{}};
        auto fw = link(dev);
        QSignalSpy events(&dev, &RawPadDevice::padEvent);
        QSignalSpy msgs(&dev, &RawPadDevice::message);
        dev.startOnFd(m_pair[0]);
        QTRY_VERIFY(dev.isActive());
        // A hot-reloaded config with a 12+3 layout on this 24-slot firmware.
        dev.setLayout(*builtinBoardProfile(QStringLiteral("generic-12k3e")));
        QVERIFY(!dev.isActive());
        QVERIFY(msgs.last().at(0).toString().contains(QStringLiteral("using evdev")));
        QTRY_COMPARE(fw->rawTimeouts.last(), 0);  // the firmware is told at once
        fw->send({5, 1, 12, 1, 0});  // the daemon's end is closed: this goes nowhere
        QTest::qWait(50);
        QCOMPARE(events.count(), 0);  // nothing goes to the wrong control
        // A layout that fits keeps raw mode.
        RawPadDevice dev2{DeviceMatch{}};
        auto fw2 = link(dev2);
        dev2.startOnFd(m_pair[0]);
        QTRY_VERIFY(dev2.isActive());
        dev2.setLayout(*builtinBoardProfile(QStringLiteral("sy181-15k3e")));
        QVERIFY(dev2.isActive());
        dev2.stop();
    }

    void unplugEndsRawMode()
    {
        RawPadDevice dev{DeviceMatch{}};
        auto fw = link(dev);
        QSignalSpy active(&dev, &RawPadDevice::activeChanged);
        dev.startOnFd(m_pair[0]);
        QTRY_VERIFY(dev.isActive());
        fw->closeLink();
        QTRY_VERIFY(!dev.isActive());
        QCOMPARE(active.count(), 2);
    }

    void findsOnlyTheControlSurfaceFirmware()
    {
        // The firmware's real report descriptor, read from usb_descr.c.
        QFile src(QStringLiteral(CS_SOURCE_DIR "/firmware/src/usb_descr.c"));
        QVERIFY(src.open(QIODevice::ReadOnly));
        const QString text = QString::fromUtf8(src.readAll());
        const int from = text.indexOf(QStringLiteral("ReportDescr[] ="));
        const int to = text.indexOf(QStringLiteral("};"), from);
        QVERIFY(from > 0 && to > from);
        QByteArray fwDesc;
        const QRegularExpression hex(QStringLiteral("0x([0-9a-fA-F]{2})"));
        // Comments carry no 0x.. bytes; take each line's code part only.
        for (const QString &line : text.mid(from, to - from).split(QLatin1Char('\n'))) {
            auto it = hex.globalMatch(line.section(QStringLiteral("//"), 0, 0));
            while (it.hasNext()) {
                fwDesc.append(char(it.next().captured(1).toInt(nullptr, 16)));
            }
        }
        const auto info = ch552::parseDescriptor(std::vector<std::uint8_t>(fwDesc.begin(), fwDesc.end()));
        QVERIFY(info.ok);
        QCOMPARE(info.reportIds, (std::vector<int>{1, 2, 4, 3, 5}));
        // Stock-like: a vendor report 3 but no raw report 5.
        const QByteArray stockDesc = QByteArray::fromHex("0600ff0901a101850375089510090281029510090391020c0");

        QTemporaryDir root;
        auto pad = [&](const QString &port, const QByteArray &serial, const QByteArray &desc, int n) {
            const QString usb = root.path() + QStringLiteral("/devices/usb1/") + port;
            const QString hid = usb + QLatin1Char('/') + port + QStringLiteral(":1.0/0003:1189:8890.000%1").arg(n);
            QDir().mkpath(hid);
            auto put = [](const QString &f, const QByteArray &d) {
                QFile o(f);
                QVERIFY(o.open(QIODevice::WriteOnly));
                o.write(d);
            };
            put(usb + QStringLiteral("/idVendor"), "1189\n");
            put(usb + QStringLiteral("/idProduct"), "8890\n");
            put(usb + QStringLiteral("/serial"), serial + "\n");
            put(usb + QLatin1Char('/') + port + QStringLiteral(":1.0/bInterfaceNumber"), "00\n");
            put(hid + QStringLiteral("/report_descriptor"), desc);
            const QString cls = root.path() + QStringLiteral("/sys/class/hidraw/hidraw%1").arg(n);
            QDir().mkpath(cls);
            QVERIFY(QFile::link(hid, cls + QStringLiteral("/device")));
        };
        pad(QStringLiteral("1-7"), "key153", stockDesc, 3);
        pad(QStringLiteral("1-5"), "key153", fwDesc, 4);
        QCOMPARE(findControlSurfaceHidraw(DeviceMatch{}, {}, root.path()), QStringLiteral("/dev/hidraw4"));
        QCOMPARE(findControlSurfaceHidraw(DeviceMatch{}, QStringLiteral("/sys/devices/pci0000:00/usb1/1-5"), root.path()), QStringLiteral("/dev/hidraw4"));
        QVERIFY(findControlSurfaceHidraw(DeviceMatch{}, QStringLiteral("/sys/bus/usb/devices/1-7"), root.path()).isEmpty());  // stock: never
        DeviceMatch other;
        other.serial = QStringLiteral("CH552GPAD");
        QVERIFY(findControlSurfaceHidraw(other, {}, root.path()).isEmpty());
        QVERIFY(findControlSurfaceHidraw(DeviceMatch{}, {}, QStringLiteral("/nonexistent")).isEmpty());
    }

    void blockingExchange()
    {
        int sv[2];
        QVERIFY(::socketpair(AF_UNIX, SOCK_SEQPACKET | SOCK_CLOEXEC, 0, sv) == 0);
        std::thread pad([fd = sv[1]] {
            char req[64];
            if (::read(fd, req, sizeof req) != 16) {
                return;
            }
            const char kbd[] = {1, 0, 0, 0x69, 0, 0, 0, 0, 0};  // a key report arrives first
            ::send(fd, kbd, sizeof kbd, MSG_NOSIGNAL);
            const char other[] = {3, 8, 0, 0, 0, 0, 0, 1};       // a reply to another command
            ::send(fd, other, sizeof other, MSG_NOSIGNAL);
            const char info[] = {3, 1, 'C', 'S', 3, 24, char(128), 1, 2, 0, 0, 2, 0, 0, 0, 0};
            ::send(fd, info, sizeof info, MSG_NOSIGNAL);
        });
        std::string err;
        const auto reply = padfw::exchange(sv[0], padfw::getInfo(), 2000, &err);
        pad.join();
        QVERIFY2(reply, err.c_str());
        QVERIFY(padfw::parseInfo(reply->data(), reply->size()));
        // Nobody answers: a timeout, not a hang.
        const auto none = padfw::exchange(sv[0], padfw::bootloader(), 150, &err);
        QVERIFY(!none);
        QCOMPARE(err, std::string("no reply"));
        ::close(sv[0]);
        ::close(sv[1]);
        QVERIFY(!padfw::queryInfo("/nonexistent/hidraw9", &err));
        QVERIFY(!padfw::requestBootloader("/nonexistent/hidraw9", &err));
    }
};

QTEST_GUILESS_MAIN(TestRawPad)
#include "tst_rawpad.moc"
