// SPDX-License-Identifier: GPL-2.0-or-later
#include "coalescer.h"

#include <QSignalSpy>
#include <QTest>

using namespace cs;

class TestCoalescer : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void firstDetentIsImmediate()
    {
        DeltaCoalescer c;
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("jog"), 1);
        QCOMPARE(spy.size(), 1);  // no added latency for a single detent
        QCOMPARE(spy[0][1].toDouble(), 1.0);
    }
    void mergesWhileInFlight()
    {
        DeltaCoalescer c;
        c.setMinIntervalMs(0);
        c.setAckTimeoutMs(10000);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        for (int i = 0; i < 50; ++i) {
            c.add(QStringLiteral("jog"), 1);
        }
        QCOMPARE(spy.size(), 1);
        QCOMPARE(c.pending(QStringLiteral("jog")), 49.0);
        c.ack(QStringLiteral("jog"));
        QCOMPARE(spy.size(), 2);
        QCOMPARE(spy[1][1].toDouble(), 49.0);
        QCOMPARE(spy[1][2].toInt(), 49);
    }
    void minIntervalWithoutAcks()
    {
        DeltaCoalescer c;
        c.setRequireAck(false);
        c.setMinIntervalMs(20);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        QElapsedTimer t;
        t.start();
        while (t.elapsed() < 100) {
            c.add(QStringLiteral("zoom"), 1);
            QTest::qWait(1);
        }
        QTRY_VERIFY(c.pending(QStringLiteral("zoom")) == 0.0);
        // ~100 detents in 100 ms must not produce more than ~1 flush per 20 ms.
        QVERIFY2(spy.size() <= 8, qPrintable(QString::number(spy.size())));
        double total = 0;
        for (const auto &s : spy) {
            total += s[1].toDouble();
        }
        int detents = 0;
        for (const auto &s : spy) {
            detents += s[2].toInt();
        }
        QCOMPARE(total, double(detents));
    }
    void lostAckDoesNotStall()
    {
        DeltaCoalescer c;
        c.setMinIntervalMs(0);
        c.setAckTimeoutMs(30);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("jog"), 1);
        c.add(QStringLiteral("jog"), 1);
        QCOMPARE(spy.size(), 1);
        QTRY_COMPARE_WITH_TIMEOUT(spy.size(), 2, 500);
    }
    void oppositeDetentsCancel()
    {
        DeltaCoalescer c;
        c.setMinIntervalMs(0);
        c.setAckTimeoutMs(10000);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("jog"), 1);
        c.add(QStringLiteral("jog"), 1);
        c.add(QStringLiteral("jog"), -1);
        c.ack(QStringLiteral("jog"));
        QCOMPARE(spy.size(), 1);  // +1 -1 merged to nothing: no message at all
    }
    void keysAreIndependent()
    {
        DeltaCoalescer c;
        c.setAckTimeoutMs(10000);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("a"), 1);
        c.add(QStringLiteral("b"), -2, {{QStringLiteral("x"), 1}});
        QCOMPARE(spy.size(), 2);
        QCOMPARE(spy[1][3].toMap().value(QStringLiteral("x")).toInt(), 1);
    }
    void zeroDeltaEndIsStillDelivered()
    {
        DeltaCoalescer c;
        c.setMinIntervalMs(1000);
        c.setAckTimeoutMs(10000);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("g"), 1);  // in flight
        c.end(QStringLiteral("g"), {{QStringLiteral("phase"), QStringLiteral("end")}}, false);
        QCOMPARE(spy.size(), 2);        // not held back by the in-flight update or the interval
        QCOMPARE(spy[1][1].toDouble(), 0.0);
        QVERIFY(spy[1][4].toBool());    // isEnd
        QCOMPARE(spy[1][3].toMap().value(QStringLiteral("phase")).toString(), QStringLiteral("end"));
        QVERIFY(!c.contains(QStringLiteral("g")));  // key retired after its end
        c.ack(QStringLiteral("g"));     // a late ack for a retired key is harmless
        QCOMPARE(spy.size(), 2);
    }
    void endCarriesPendingDeltaUnlessDropped()
    {
        DeltaCoalescer c;
        c.setMinIntervalMs(0);
        c.setAckTimeoutMs(10000);
        QSignalSpy spy(&c, &DeltaCoalescer::flushed);
        c.add(QStringLiteral("a"), 1);
        c.add(QStringLiteral("a"), 2);  // merged behind the in-flight 1
        c.end(QStringLiteral("a"), {}, false);
        QCOMPARE(spy.last()[1].toDouble(), 2.0);  // final update rides on the end
        QVERIFY(spy.last()[4].toBool());
        c.add(QStringLiteral("b"), 1);
        c.add(QStringLiteral("b"), 5);
        c.end(QStringLiteral("b"), {}, true);  // target changed: unsent motion is dropped
        QCOMPARE(spy.last()[1].toDouble(), 0.0);
        QVERIFY(spy.last()[4].toBool());
        c.end(QStringLiteral("never-used"), {}, false);  // unknown key: nothing to end
        QCOMPARE(spy.size(), 4);
    }
    void boundedKeys()
    {
        DeltaCoalescer c;
        c.setMaxKeys(3);
        QVERIFY(c.add(QStringLiteral("1"), 1));
        QVERIFY(c.add(QStringLiteral("2"), 1));
        QVERIFY(c.add(QStringLiteral("3"), 1));
        QVERIFY(!c.add(QStringLiteral("4"), 1));
        QVERIFY(c.add(QStringLiteral("1"), 1));  // existing keys still accept motion
        QCOMPARE(c.keyCount(), 3);
    }
};

QTEST_GUILESS_MAIN(TestCoalescer)
#include "tst_coalescer.moc"
