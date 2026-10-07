// SPDX-License-Identifier: GPL-2.0-or-later
// Contract revision 2 on a PRIVATE real session bus (run via dbus-run-session;
// see CMakeLists.txt). Covers what a peer connection cannot: several
// subscribers with distinct unique names, destination-addressed signals,
// sender-bound leases and ownership loss.
#include "engine.h"
#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"
#include "mockkdenlive.h"
#include "rawclient.h"

#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusContext>
#include <QSignalSpy>
#include <QTest>

using namespace cs;
using State = KdenliveClient::State;

namespace {
const QString kService = QStringLiteral("org.kde.kdenlive-test");
const QString kScripted = QStringLiteral("org.kde.kdenlive-scripted");
QDBusConnection bus(const QString &name)
{
    return QDBusConnection::connectToBus(QDBusConnection::SessionBus, name);
}
} // namespace

// Minimal scripted server: records Control calls and lets the test choose acks.
class ScriptedObject : public QObject, public QDBusContext
{
    Q_OBJECT
public:
    using QObject::QObject;
    void delay()
    {
        setDelayedReply(true);  // and never answer: a hung editor
    }
};

class ScriptedAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.kdenlive.ControlSurface1")
public:
    explicit ScriptedAdaptor(ScriptedObject *parent)
        : QDBusAbstractAdaptor(parent)
        , m(parent)
    {
    }
    bool silent = false;
    int subscribes = 0;
    QList<QVariantMap> received;

public Q_SLOTS:
    QVariantMap Capabilities()
    {
        if (silent) {
            m->delay();
            return {};
        }
        return {{QStringLiteral("ok"), true},
                {QStringLiteral("result"), QVariantMap{{QStringLiteral("version"), 1u}, {QStringLiteral("revision"), 2u},
                                                       {QStringLiteral("controls"), QStringList{contract::kJog, contract::kColorWheel}},
                                                       {QStringLiteral("commands"), QStringList{}}}}};
    }
    QVariantMap Subscribe()
    {
        return {{QStringLiteral("ok"), true},
                {QStringLiteral("result"), QVariantMap{{QStringLiteral("session"), QStringLiteral("sess-%1").arg(++subscribes)},
                                                       {QStringLiteral("context"), QVariantMap{{QStringLiteral("serial"), QVariant::fromValue<qulonglong>(1)},
                                                                                               {QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)}}}}}};
    }
    QVariantMap Unsubscribe(const QString &)
    {
        return {{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("unsubscribed"), true}}}};
    }
    QVariantMap ListActions()
    {
        return {{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("actions"), QVariantList{}}}}};
    }
    Q_NOREPLY void Control(const QString &control, double delta, const QVariantMap &options, qulonglong seq)
    {
        received << QVariantMap{{QStringLiteral("control"), control},
                                {QStringLiteral("delta"), delta},
                                {QStringLiteral("options"), options},
                                {QStringLiteral("seq"), QVariant::fromValue<qulonglong>(seq)},
                                {QStringLiteral("sender"), m->message().service()}};
    }

private:
    ScriptedObject *m;
};

class TestKdenliveBus : public QObject
{
    Q_OBJECT
    MockKdenlive *m_mock = nullptr;
    QStringList m_conns;

    QDBusConnection client(const QString &name)
    {
        m_conns << name;
        return bus(name);
    }

private Q_SLOTS:
    void initTestCase()
    {
        if (qEnvironmentVariable("CS_PRIVATE_BUS") != QLatin1String("1")) {
            QSKIP("runs only on a private bus: ctest starts it under dbus-run-session");
        }
    }
    void init()
    {
        m_mock = new MockKdenlive(this);
        QDBusConnection m = bus(QStringLiteral("mock"));
        QVERIFY(m.isConnected());
        QVERIFY(m_mock->registerOn(m));
        QVERIFY(m.registerService(kService));
    }
    void cleanup()
    {
        for (const auto &c : std::as_const(m_conns)) {
            QDBusConnection::disconnectFromBus(c);
        }
        m_conns.clear();
        QDBusConnection m(QStringLiteral("mock"));
        m.unregisterService(kService);
        m.unregisterObject(contract::kPath);
        QDBusConnection::disconnectFromBus(QStringLiteral("mock"));
        delete m_mock;
        m_mock = nullptr;
    }

    // Item 3: pending keys include the sender; acks go only to their sender.
    void directedAcksAndSenderSeparation()
    {
        RawClient a(client(QStringLiteral("A")), kService);
        RawClient b(client(QStringLiteral("B")), kService);
        QVERIFY(a.subscribe().value(QStringLiteral("ok")).toBool());
        QVERIFY(b.subscribe().value(QStringLiteral("ok")).toBool());
        QVERIFY(a.session != b.session);
        m_mock->setApplyDelayMs(30);  // both land in one apply batch
        a.control(contract::kJog, 2, {}, 1);
        b.control(contract::kJog, 3, {}, 1);  // same control, options and seq number, other lease
        QTRY_COMPARE(a.acks.size(), 1);
        QTRY_COMPARE(b.acks.size(), 1);
        QTest::qWait(50);
        QCOMPARE(a.acks.size(), 1);  // never sees B's acknowledgement
        QCOMPARE(b.acks.size(), 1);
        QCOMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 5);
        const QVariantMap ra = a.acks[0].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap();
        const QVariantMap rb = b.acks[0].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap();
        QCOMPARE(ra.value(QStringLiteral("lastSeq")).toULongLong(), 1ull);
        QCOMPARE(rb.value(QStringLiteral("lastSeq")).toULongLong(), 1ull);
        QCOMPARE(a.acks[0].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("session")).toString(), a.session);
        QCOMPARE(b.acks[0].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("session")).toString(), b.session);
        QCOMPARE(m_mock->log.filter(QStringLiteral("Control playhead.jog")).size(), 2);  // two keys, not merged
        // Merging within one lease reports the batch range.
        a.control(contract::kJog, 1, {}, 2);
        a.control(contract::kJog, 1, {}, 3);
        a.control(contract::kJog, 1, {}, 4);
        QTRY_COMPARE(a.acks.size(), 2);
        const QVariantMap batch = a.acks[1].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap();
        QCOMPARE(a.acks[1].value(QStringLiteral("seq")).toULongLong(), 4ull);
        QCOMPARE(batch.value(QStringLiteral("firstSeq")).toULongLong(), 2ull);
        QCOMPARE(batch.value(QStringLiteral("lastSeq")).toULongLong(), 4ull);
    }

    void leaseIsSenderBound()
    {
        RawClient a(client(QStringLiteral("A")), kService);
        RawClient b(client(QStringLiteral("B")), kService);
        a.subscribe();
        b.subscribe();
        QVariantMap stolen{{QStringLiteral("session"), a.session}};
        auto msg = b.message(QStringLiteral("Control"));
        msg << contract::kJog << 1.0 << QVariant::fromValue(QVariantMap{{QStringLiteral("session"), a.session}, {QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(a.epoch())}})
            << QVariant::fromValue<qulonglong>(1);
        QDBusConnection(QStringLiteral("B")).send(msg);
        QTRY_COMPARE(b.acks.size(), 1);
        QCOMPARE(RawClient::code(b.acks[0].value(QStringLiteral("outcome")).toMap()), contract::err::NotSubscribed);
        QCOMPARE(a.acks.size(), 0);
        QCOMPARE(RawClient::code(b.call(QStringLiteral("Unsubscribe"), {a.session})), contract::err::NotSubscribed);
        QCOMPARE(m_mock->leaseCount(), 2);
    }

    void contextOnlyToCurrentSubscribers()
    {
        RawClient a(client(QStringLiteral("A")), kService);
        RawClient b(client(QStringLiteral("B")), kService);
        RawClient c(client(QStringLiteral("C")), kService);  // listens but never subscribes
        a.subscribe();
        b.subscribe();
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("clipMonitor"));
        QTRY_COMPARE(a.contexts, 1);
        QTRY_COMPARE(b.contexts, 1);
        QVERIFY(b.call(QStringLiteral("Unsubscribe"), {b.session}).value(QStringLiteral("ok")).toBool());
        QTest::qWait(40);
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("timeline"));
        QTRY_COMPARE(a.contexts, 2);
        QTest::qWait(80);
        QCOMPARE(b.contexts, 1);  // unsubscribed while A stays subscribed: nothing more
        QCOMPARE(c.contexts, 0);
        QCOMPARE(a.context.value(QStringLiteral("focus")).toString(), QStringLiteral("timeline"));
    }

    void contextRateIsAtMost30Hz()
    {
        RawClient a(client(QStringLiteral("A")), kService);
        a.subscribe();
        QElapsedTimer t;
        t.start();
        int frame = 0;
        while (t.elapsed() < 1000) {
            m_mock->setPosition(++frame);
            QTest::qWait(2);
        }
        QTest::qWait(60);
        const auto times = m_mock->contextEmitTimesMs();
        QVERIFY2(times.size() <= 31, qPrintable(QString::number(times.size())));
        QVERIFY(times.size() >= 20);
        for (int i = 1; i < times.size(); ++i) {
            QVERIFY2(times[i] - times[i - 1] >= contract::kMinContextIntervalMs, qPrintable(QString::number(times[i] - times[i - 1])));
        }
        QTRY_COMPARE(a.context.value(QStringLiteral("position")).toInt(), frame);  // the last state still arrives
        // No subscribers: no context work at all.
        a.call(QStringLiteral("Unsubscribe"), {a.session});
        const int before = m_mock->contextSignalsTotal();
        for (int i = 0; i < 50; ++i) {
            m_mock->setPosition(i);
        }
        QTest::qWait(80);
        QCOMPARE(m_mock->contextSignalsTotal(), before);
    }

    void ownerLossDropsQueuedWorkAndEndsGesture()
    {
        m_mock->setContextValue(QStringLiteral("colorWheel"), QVariantMap{{QStringLiteral("target"), QStringLiteral("cw-1")}});
        {
            RawClient b(client(QStringLiteral("B")), kService);
            b.subscribe();
            const QVariantMap g{{QStringLiteral("target"), QStringLiteral("cw-1")}, {QStringLiteral("gesture"), QStringLiteral("b1")}, {QStringLiteral("wheel"), QStringLiteral("gain")}};
            b.control(contract::kColorWheel, 10, g, 1);
            QTRY_COMPARE(b.acks.size(), 1);  // applied, gesture still open
            m_mock->setApplyDelayMs(300);
            b.control(contract::kJog, 50, {}, 2);  // queued, not yet applied
        }
        QDBusConnection::disconnectFromBus(QStringLiteral("B"));
        m_conns.removeAll(QStringLiteral("B"));
        QTRY_COMPARE(m_mock->leaseCount(), 0);
        QCOMPARE(m_mock->history().size(), 1);  // applied gesture ends as one owned entry
        QTest::qWait(350);
        QCOMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 0);  // queued work dropped
    }

    void leaseLimit()
    {
        QList<RawClient *> clients;
        for (int i = 0; i < contract::kMaxLeases; ++i) {
            auto *r = new RawClient(client(QStringLiteral("L%1").arg(i)), kService, this);
            QVERIFY(r->subscribe().value(QStringLiteral("ok")).toBool());
            clients << r;
        }
        RawClient extra(client(QStringLiteral("L9")), kService);
        QCOMPARE(RawClient::code(extra.subscribe()), contract::err::ResourceLimit);
        qDeleteAll(clients);
    }

    // Items 1 and 2: the client correlates by sequence and target; late, foreign,
    // duplicate and wrong-target acks release nothing.
    void clientAckCorrelation()
    {
        auto *obj = new ScriptedObject(this);
        auto *adaptor = new ScriptedAdaptor(obj);
        QDBusConnection srv = bus(QStringLiteral("scripted"));
        m_conns << QStringLiteral("scripted");
        QVERIFY(srv.registerObject(contract::kPath, obj, QDBusConnection::ExportAdaptors));
        QVERIFY(srv.registerService(kScripted));
        QDBusConnection cc = client(QStringLiteral("daemon"));
        KdenliveDBusClient kd(cc);
        kd.setServiceOverride(kScripted);
        kd.attachToPid(1);
        QTRY_VERIFY(kd.isAvailable());
        QSignalSpy acked(&kd, &KdenliveClient::controlAcked);
        QVERIFY(kd.control(QStringLiteral("K1"), contract::kJog, 1, {}));
        QVERIFY(kd.control(QStringLiteral("K1"), contract::kJog, 1, {}));  // ack timeout elapsed: newer batch
        QVERIFY(kd.control(QStringLiteral("K2"), contract::kColorWheel, 1, {{QStringLiteral("target"), QStringLiteral("t2")}, {QStringLiteral("gesture"), QStringLiteral("g")}}));
        QVERIFY(!kd.control(QStringLiteral("K3"), contract::kZoom, 1, {}));  // not advertised: never sent
        QTRY_COMPARE(adaptor->received.size(), 3);
        const quint64 s1 = adaptor->received[0].value(QStringLiteral("seq")).toULongLong();
        const quint64 s2 = adaptor->received[1].value(QStringLiteral("seq")).toULongLong();
        const quint64 s3 = adaptor->received[2].value(QStringLiteral("seq")).toULongLong();
        QVERIFY(s1 < s2 && s2 < s3);
        QCOMPARE(adaptor->received[0].value(QStringLiteral("options")).toMap().value(QStringLiteral("session")).toString(), kd.session());
        const QString me = cc.baseService();
        const QString lease = kd.session();
        QCOMPARE(lease, QStringLiteral("sess-1"));
        auto sendAck = [&](const QDBusConnection &from, quint64 seq, const QString &control, const QVariantMap &result) {
            QDBusMessage m = QDBusMessage::createTargetedSignal(me, contract::kPath, contract::kInterface, QStringLiteral("ControlAck"));
            m << QVariant::fromValue<qulonglong>(seq) << control
              << QVariant::fromValue(QVariantMap{{QStringLiteral("ok"), true}, {QStringLiteral("session"), lease}, {QStringLiteral("result"), result}});
            QVERIFY(QDBusConnection(from).send(m));
        };
        sendAck(srv, s1, contract::kJog, {{QStringLiteral("state"), QStringLiteral("applied")}});  // late: s2 still in flight
        sendAck(srv, s3, contract::kColorWheel, {{QStringLiteral("target"), QStringLiteral("other")}});  // wrong target
        sendAck(srv, 999999, contract::kJog, {});  // unknown sequence
        QDBusConnection evil = client(QStringLiteral("evil"));
        sendAck(evil, s2, contract::kJog, {});  // not from the service owner
        QTest::qWait(100);
        QCOMPARE(acked.size(), 0);
        sendAck(srv, s3, contract::kColorWheel, {{QStringLiteral("target"), QStringLiteral("t2")}});
        QTRY_COMPARE(acked.size(), 1);
        QCOMPARE(acked[0][0].toString(), QStringLiteral("K2"));  // only its own key
        sendAck(srv, s2, contract::kJog, {});
        QTRY_COMPARE(acked.size(), 2);
        QCOMPARE(acked[1][0].toString(), QStringLiteral("K1"));
        sendAck(srv, s2, contract::kJog, {});  // duplicate
        QTest::qWait(50);
        QCOMPARE(acked.size(), 2);
        QCOMPARE(kd.inFlightMessages(), 0);
        // A new lease never reuses sequence numbers, so old acks cannot collide.
        kd.attachToPid(0);
        kd.attachToPid(1);
        QTRY_VERIFY(kd.isAvailable());
        QVERIFY(kd.control(QStringLiteral("K1"), contract::kJog, 1, {}));
        QTRY_COMPARE(adaptor->received.size(), 4);
        QVERIFY(adaptor->received[3].value(QStringLiteral("seq")).toULongLong() > s3);
    }

    // MAIN clarification: acks are matched on (session, seq). A late ack of an
    // earlier lease never releases a batch of the current lease, even when it
    // names a sequence number that is pending now.
    void lateAckFromOldLeaseIsIgnored()
    {
        auto *obj = new ScriptedObject(this);
        auto *adaptor = new ScriptedAdaptor(obj);
        QDBusConnection srv = bus(QStringLiteral("scripted2"));
        m_conns << QStringLiteral("scripted2");
        QVERIFY(srv.registerObject(contract::kPath, obj, QDBusConnection::ExportAdaptors));
        QVERIFY(srv.registerService(QStringLiteral("org.kde.kdenlive-scripted2")));
        QDBusConnection cc = client(QStringLiteral("daemon3"));
        KdenliveDBusClient kd(cc);
        kd.setServiceOverride(QStringLiteral("org.kde.kdenlive-scripted2"));
        kd.attachToPid(1);
        QTRY_VERIFY(kd.isAvailable());
        const QString oldLease = kd.session();
        QVERIFY(kd.control(QStringLiteral("K"), contract::kJog, 1, {}));
        QTRY_COMPARE(adaptor->received.size(), 1);
        kd.attachToPid(0);  // lease ends
        kd.attachToPid(1);  // new lease
        QTRY_VERIFY(kd.isAvailable());
        const QString newLease = kd.session();
        QVERIFY(newLease != oldLease);
        QVERIFY(kd.control(QStringLiteral("K"), contract::kJog, 1, {}));
        QTRY_COMPARE(adaptor->received.size(), 2);
        const quint64 current = adaptor->received[1].value(QStringLiteral("seq")).toULongLong();
        QCOMPARE(adaptor->received[1].value(QStringLiteral("options")).toMap().value(QStringLiteral("session")).toString(), newLease);
        QSignalSpy acked(&kd, &KdenliveClient::controlAcked);
        QSignalSpy refused(&kd, &KdenliveClient::refused);
        auto send = [&](quint64 seq, const QString &session, bool ok) {
            QDBusMessage m = QDBusMessage::createTargetedSignal(cc.baseService(), contract::kPath, contract::kInterface, QStringLiteral("ControlAck"));
            QVariantMap outcome{{QStringLiteral("ok"), ok}, {QStringLiteral("session"), session}};
            if (ok) {
                outcome.insert(QStringLiteral("result"), QVariantMap{{QStringLiteral("state"), QStringLiteral("applied")}});
            } else {
                outcome.insert(QStringLiteral("error"), QVariantMap{{QStringLiteral("code"), contract::err::StaleContext}});
            }
            m << QVariant::fromValue<qulonglong>(seq) << contract::kJog << QVariant::fromValue(outcome);
            QVERIFY(srv.send(m));
        };
        send(current, oldLease, true);   // old lease, colliding sequence number
        send(current, oldLease, false);  // old lease error
        send(current, QString(), true);  // no session at all
        QTest::qWait(100);
        QCOMPARE(acked.size(), 0);
        QCOMPARE(refused.size(), 0);
        QCOMPARE(kd.inFlightMessages(), 1);
        send(current, newLease, false);  // the real answer, an error: still correlated and released
        QTRY_COMPARE(acked.size(), 1);
        QCOMPARE(refused.size(), 1);
        QCOMPARE(kd.inFlightMessages(), 0);
    }

    // Item 4 support: absent vs. unanswered are different states.
    void absentVersusPending()
    {
        QDBusConnection cc = client(QStringLiteral("daemon2"));
        {
            KdenliveDBusClient kd(cc);
            kd.setServiceOverride(QStringLiteral("org.kde.kdenlive-nothere"));
            kd.attachToPid(1);
            QTRY_COMPARE(kd.state(), State::Absent);  // no such service
        }
        QDBusConnection off = bus(QStringLiteral("off"));
        m_conns << QStringLiteral("off");
        QVERIFY(off.registerService(QStringLiteral("org.kde.kdenlive-off")));  // Kdenlive running, interface off
        {
            KdenliveDBusClient kd(cc);
            kd.setServiceOverride(QStringLiteral("org.kde.kdenlive-off"));
            kd.attachToPid(1);
            QTRY_COMPARE(kd.state(), State::Absent);
        }
        auto *obj = new ScriptedObject(this);
        auto *adaptor = new ScriptedAdaptor(obj);
        adaptor->silent = true;  // hung editor
        QDBusConnection srv = bus(QStringLiteral("hung"));
        m_conns << QStringLiteral("hung");
        QVERIFY(srv.registerObject(contract::kPath, obj, QDBusConnection::ExportAdaptors));
        QVERIFY(srv.registerService(QStringLiteral("org.kde.kdenlive-hung")));
        KdenliveDBusClient kd(cc);
        kd.setServiceOverride(QStringLiteral("org.kde.kdenlive-hung"));
        RecordingKeySink keys;
        Engine e(&keys, &kd);
        Config cfg;
        QString err;
        cfg = *parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{\"key1\":{\"action\":\"mark_in\",\"fallback\":\"i\"}}}]}", {}, &err);
        e.setConfig(cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 3, QStringLiteral("0x3")});
        QTest::qWait(2300);  // past the client's call timeout
        QCOMPARE(kd.state(), State::Pending);  // a timeout proves nothing
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTest::qWait(50);
        QVERIFY(keys.taps.isEmpty());  // never typed into a busy editor
    }
};

QTEST_GUILESS_MAIN(TestKdenliveBus)
#include "tst_kdenlive_bus.moc"
