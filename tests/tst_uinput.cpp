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
};

QTEST_GUILESS_MAIN(TestUinput)
#include "tst_uinput.moc"
