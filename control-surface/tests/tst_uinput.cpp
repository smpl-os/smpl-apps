// SPDX-License-Identifier: GPL-2.0-or-later
// Creates the real uinput keyboard and reads it back. The test grabs the new
// event node BEFORE emitting anything, so no key ever reaches the desktop; if
// the grab fails it skips without emitting.
#include "uinputsink.h"

#include <QDir>
#include <QElapsedTimer>
#include <QFile>
#include <QTest>
#include <cerrno>
#include <fcntl.h>
#include <linux/input.h>
#include <poll.h>
#include <sys/ioctl.h>
#include <unistd.h>

using namespace cs;

class TestUinput : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void tapReadsBackThroughGrabbedNode()
    {
        if (::access("/dev/uinput", W_OK) != 0) {
            QSKIP("/dev/uinput not writable");
        }
        UinputKeySink sink;
        QString err;
        QVERIFY2(sink.open(&err), qPrintable(err));
        const QString sys = QStringLiteral("/sys/devices/virtual/input/") + sink.sysName();
        QString node;
        QElapsedTimer t;
        t.start();
        while (node.isEmpty() && t.elapsed() < 3000) {
            const auto ev = QDir(sys).entryList({QStringLiteral("event*")}, QDir::Dirs);
            if (!ev.isEmpty()) {
                node = QStringLiteral("/dev/input/") + ev.first();
            }
            QTest::qWait(20);
        }
        QVERIFY2(!node.isEmpty(), qPrintable(sys));
        int fd = -1;
        while (t.elapsed() < 3000 && (fd = ::open(QFile::encodeName(node).constData(), O_RDONLY | O_NONBLOCK)) < 0) {
            QTest::qWait(20);  // udev applies the input-group permission
        }
        if (fd < 0) {
            QSKIP("cannot open the virtual keyboard's event node");
        }
        input_id id{};
        QVERIFY(::ioctl(fd, EVIOCGID, &id) == 0);
        QCOMPARE(id.vendor, kUinputVendor);
        QCOMPARE(id.product, kUinputProduct);
        QVERIFY(id.vendor != 0x1189);  // the pad discovery can never match it
        if (::ioctl(fd, EVIOCGRAB, 1) != 0) {
            ::close(fd);
            QSKIP("cannot grab the virtual keyboard; not emitting keys into the session");
        }
        sink.tap(KeyChord{Mod::Ctrl, KEY_F14});
        QList<QPair<int, int>> seen;
        while (seen.size() < 4 && t.elapsed() < 5000) {
            pollfd p{fd, POLLIN, 0};
            if (::poll(&p, 1, 200) <= 0) {
                continue;
            }
            input_event ev;
            while (::read(fd, &ev, sizeof ev) == ssize_t(sizeof ev)) {
                if (ev.type == EV_KEY) {
                    seen << qMakePair(int(ev.code), ev.value);
                }
            }
        }
        ::ioctl(fd, EVIOCGRAB, 0);
        ::close(fd);
        const QList<QPair<int, int>> want{{KEY_LEFTCTRL, 1}, {KEY_F14, 1}, {KEY_F14, 0}, {KEY_LEFTCTRL, 0}};
        QCOMPARE(seen, want);
    }

    void mouseEventSequences()
    {
        QCOMPARE(mouseActionNames().size(), 9);
        for (const QString &n : mouseActionNames()) {
            QVERIFY2(mouseEvents(n), qPrintable(n));
            QCOMPARE(mouseEvents(n)->last(), (InputTriple{EV_SYN, SYN_REPORT, 0}));
        }
        QCOMPARE(*mouseEvents(QStringLiteral("left")),
                 (QList<InputTriple>{{EV_KEY, BTN_LEFT, 1}, {EV_SYN, SYN_REPORT, 0}, {EV_KEY, BTN_LEFT, 0}, {EV_SYN, SYN_REPORT, 0}}));
        QCOMPARE(mouseEvents(QStringLiteral("back"))->first().code, BTN_SIDE);
        QCOMPARE(mouseEvents(QStringLiteral("forward"))->first().code, BTN_EXTRA);
        QCOMPARE(mouseEvents(QStringLiteral("wheel-up"))->first(), (InputTriple{EV_REL, REL_WHEEL, 1}));
        QCOMPARE(mouseEvents(QStringLiteral("wheel-down"))->first(), (InputTriple{EV_REL, REL_WHEEL, -1}));
        QCOMPARE(mouseEvents(QStringLiteral("wheel-left"))->first(), (InputTriple{EV_REL, REL_HWHEEL, -1}));
        QCOMPARE(mouseEvents(QStringLiteral("wheel-right"))->first(), (InputTriple{EV_REL, REL_HWHEEL, 1}));
        QVERIFY(!mouseEvents(QStringLiteral("scroll")));
        RecordingKeySink rec;
        rec.mouse(QStringLiteral("middle"));
        QCOMPARE(rec.taps, QStringList{QStringLiteral("mouse:middle")});
    }

    void mouseReadsBackThroughGrabbedNode()
    {
        if (::access("/dev/uinput", W_OK) != 0) {
            QSKIP("/dev/uinput not writable");
        }
        UinputKeySink sink;
        QString err;
        QVERIFY2(sink.open(&err), qPrintable(err));
        QVERIFY(sink.openPointer());
        const QString sys = QStringLiteral("/sys/devices/virtual/input/") + sink.pointerSysName();
        QString node;
        QElapsedTimer t;
        t.start();
        while (node.isEmpty() && t.elapsed() < 3000) {
            const auto ev = QDir(sys).entryList({QStringLiteral("event*")}, QDir::Dirs);
            if (!ev.isEmpty()) {
                node = QStringLiteral("/dev/input/") + ev.first();
            }
            QTest::qWait(20);
        }
        QVERIFY2(!node.isEmpty(), qPrintable(sys));
        int fd = -1;
        while (t.elapsed() < 3000 && (fd = ::open(QFile::encodeName(node).constData(), O_RDONLY | O_NONBLOCK)) < 0) {
            QTest::qWait(20);
        }
        if (fd < 0) {
            QSKIP("cannot open the virtual pointer's event node");
        }
        input_id id{};
        QVERIFY(::ioctl(fd, EVIOCGID, &id) == 0);
        QCOMPARE(id.vendor, kUinputVendor);
        QCOMPARE(id.product, kUinputPointerProduct);
        if (::ioctl(fd, EVIOCGRAB, 1) != 0) {
            ::close(fd);
            QSKIP("cannot grab the virtual pointer; not clicking into the session");
        }
        sink.mouse(QStringLiteral("right"));
        sink.mouse(QStringLiteral("wheel-down"));
        QList<InputTriple> seen;
        while (seen.size() < 3 && t.elapsed() < 6000) {
            pollfd p{fd, POLLIN, 0};
            if (::poll(&p, 1, 200) <= 0) {
                continue;
            }
            input_event ev;
            while (::read(fd, &ev, sizeof ev) == ssize_t(sizeof ev)) {
                if (ev.type != EV_SYN) {
                    seen << InputTriple{ev.type, ev.code, ev.value};
                }
            }
        }
        ::ioctl(fd, EVIOCGRAB, 0);
        ::close(fd);
        QCOMPARE(seen, (QList<InputTriple>{{EV_KEY, BTN_RIGHT, 1}, {EV_KEY, BTN_RIGHT, 0}, {EV_REL, REL_WHEEL, -1}}));
    }
};

QTEST_GUILESS_MAIN(TestUinput)
#include "tst_uinput.moc"
