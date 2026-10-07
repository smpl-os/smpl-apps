// SPDX-License-Identifier: GPL-2.0-or-later
// Contract revision 2 over real D-Bus marshalling on a peer connection:
// envelopes, staged capabilities, validation, sequences, actions, gestures,
// and Engine -> KdenliveDBusClient -> MockKdenlive end to end.
#include "engine.h"
#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"
#include "mockkdenlive.h"
#include "rawclient.h"

#include <QDBusConnection>
#include <QDBusServer>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <limits>

using namespace cs;
using State = KdenliveClient::State;

// A server of an older, incompatible revision of the interface.
class OldAdaptor : public QDBusAbstractAdaptor
{
    Q_OBJECT
    Q_CLASSINFO("D-Bus Interface", "org.kde.kdenlive.ControlSurface1")
public:
    explicit OldAdaptor(QObject *parent)
        : QDBusAbstractAdaptor(parent)
    {
    }
public Q_SLOTS:
    QVariantMap Capabilities()
    {
        return {{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("version"), 1u}, {QStringLiteral("revision"), 1u}}}};
    }
    QVariantMap Subscribe()
    {
        ++subscribes;
        return {{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("session"), QStringLiteral("x")}}}};
    }

public:
    int subscribes = 0;
};

class TestKdenliveDBus : public QObject
{
    Q_OBJECT
    QDBusServer *m_server = nullptr;
    MockKdenlive *m_mock = nullptr;
    QStringList m_peers;
    bool m_register = true;
    QObject *m_old = nullptr;
    OldAdaptor *m_oldAdaptor = nullptr;
    Config m_cfg;
    int m_conn = 0;

    QDBusConnection connectClient()
    {
        return QDBusConnection::connectToPeer(m_server->address(), QStringLiteral("c%1").arg(++m_conn));
    }
    QVariantMap wheelCtx(const QString &target = QStringLiteral("cw-1"))
    {
        return {{QStringLiteral("target"), target}};
    }

private Q_SLOTS:
    void initTestCase()
    {
        QTemporaryDir home;
        qputenv("HOME", home.path().toLocal8Bit());
        QString err;
        auto c = loadConfig(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"), &err);
        QVERIFY2(c, qPrintable(err));
        m_cfg = *c;
        m_cfg.settings.accelFactor = 1.0;
    }
    void init()
    {
        m_server = new QDBusServer(QStringLiteral("unix:tmpdir=/tmp"), this);
        QVERIFY(m_server->isConnected());
        m_server->setAnonymousAuthenticationAllowed(true);
        m_mock = new MockKdenlive(this);
        connect(m_server, &QDBusServer::newConnection, this, [this](const QDBusConnection &conn) {
            if (m_old) {
                QDBusConnection(conn).registerObject(contract::kPath, m_old, QDBusConnection::ExportAdaptors);
            } else if (m_register) {
                QVERIFY(m_mock->registerOn(conn));
            }
            m_peers << conn.name();
        });
    }
    void cleanup()
    {
        for (const auto &p : std::as_const(m_peers)) {
            QDBusConnection::disconnectFromPeer(p);
        }
        m_peers.clear();
        for (int i = 1; i <= m_conn; ++i) {
            QDBusConnection::disconnectFromPeer(QStringLiteral("c%1").arg(i));
        }
        delete m_mock;
        delete m_server;
        delete m_old;
        m_old = nullptr;
        m_register = true;
    }

    void envelopesAndStagedCapabilities()
    {
        m_mock->setStage(1);
        RawClient raw(connectClient(), QString());
        QTRY_VERIFY(!m_peers.isEmpty());
        const QVariantMap caps = raw.call(QStringLiteral("Capabilities"));
        QVERIFY(caps.value(QStringLiteral("ok")).toBool());
        const QVariantMap r = caps.value(QStringLiteral("result")).toMap();
        QCOMPARE(r.value(QStringLiteral("version")).toUInt(), 1u);
        QCOMPARE(r.value(QStringLiteral("revision")).toUInt(), 2u);
        QCOMPARE(r.value(QStringLiteral("controls")).toStringList(), (QStringList{contract::kJog, contract::kShuttle, contract::kZoom}));
        QVERIFY(r.value(QStringLiteral("commands")).toStringList().isEmpty());
        QCOMPARE(r.value(QStringLiteral("limits")).toMap().value(QStringLiteral("maxLeases")).toInt(), 8);
        QCOMPARE(r.value(QStringLiteral("controlDescriptors")).toList().size(), 3);
        // Wire types: ListActions result carries aa{sv}.
        const QDBusMessage la = raw.rawCall(QStringLiteral("ListActions"));
        QCOMPARE(la.signature(), QStringLiteral("a{sv}"));
        const QVariantList actions = KdenliveDBusClient::normalize(la.arguments().value(0)).toMap().value(QStringLiteral("result")).toMap().value(QStringLiteral("actions")).toList();
        QVERIFY(actions.size() > 10);
        QCOMPARE(actions.first().toMap().value(QStringLiteral("id")).toString(), QStringLiteral("monitor_play"));
        // Subscribe/GetContext/Unsubscribe envelopes and required context keys.
        QVERIFY(raw.subscribe().value(QStringLiteral("ok")).toBool());
        QVERIFY(!raw.session.isEmpty());
        for (const char *k : {"serial", "epoch", "ready", "active", "dialog", "focus", "project", "sequence", "activeMonitor", "position", "fps", "playing", "speed", "tool"}) {
            QVERIFY2(raw.context.contains(QString::fromLatin1(k)), k);
        }
        QCOMPARE(raw.context.value(QStringLiteral("fps")).toMap().value(QStringLiteral("num")).toInt(), 25);
        QVERIFY(!raw.context.contains(QStringLiteral("timeline")));  // MR3 context absent at stage 1
        m_mock->setStage(3);
        const QVariantMap track = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap().value(QStringLiteral("timeline")).toMap().value(QStringLiteral("track")).toMap();
        QVERIFY(!track.value(QStringLiteral("target")).toString().isEmpty());
        QCOMPARE(track.value(QStringLiteral("id")).toInt(), 7);  // native id
        QCOMPARE(track.value(QStringLiteral("sequence")).toString(), QStringLiteral("seq-1"));
        m_mock->setStage(1);
        const QString again = raw.session;
        raw.subscribe();
        QCOMPARE(raw.session, again);  // idempotent per sender
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Unsubscribe"), {QStringLiteral("not-mine")})), contract::err::NotSubscribed);
        QVERIFY(raw.call(QStringLiteral("Unsubscribe"), {raw.session}).value(QStringLiteral("result")).toMap().value(QStringLiteral("unsubscribed")).toBool());
        // Introspection exposes exactly the contract members (explicit adaptor).
        const QDBusMessage intro = QDBusConnection(QStringLiteral("c%1").arg(m_conn))
                                       .call(QDBusMessage::createMethodCall(QString(), contract::kPath, QStringLiteral("org.freedesktop.DBus.Introspectable"), QStringLiteral("Introspect")),
                                             QDBus::BlockWithGui, 3000);
        const QString xml = intro.arguments().value(0).toString();
        for (const char *m : {"Capabilities", "Subscribe", "Unsubscribe", "GetContext", "ListActions", "TriggerAction", "Control", "SetControlValue", "Invoke", "Notify", "ContextChanged", "ControlAck", "ActionFinished", "ActionsChanged"}) {
            QVERIFY2(xml.contains(QStringLiteral("\"%1\"").arg(QString::fromLatin1(m))), m);
        }
        QVERIFY(!xml.contains(QStringLiteral("setContextValue")));
        QVERIFY(!xml.contains(QStringLiteral("registerOn")));
    }

    void validationAndSequences()
    {
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        quint64 seq = 10;
        raw.control(contract::kJog, 3, {}, seq);  // ok
        QTRY_VERIFY(!raw.ackFor(10).isEmpty());
        const QVariantMap okAck = raw.ackFor(10).value(QStringLiteral("outcome")).toMap();
        QVERIFY(okAck.value(QStringLiteral("ok")).toBool());
        QCOMPARE(okAck.value(QStringLiteral("session")).toString(), raw.session);
        const QVariantMap res = okAck.value(QStringLiteral("result")).toMap();
        QCOMPARE(res.value(QStringLiteral("state")).toString(), QStringLiteral("applied"));
        QCOMPARE(res.value(QStringLiteral("firstSeq")).toULongLong(), 10ull);
        QCOMPARE(res.value(QStringLiteral("lastSeq")).toULongLong(), 10ull);
        QCOMPARE(res.value(QStringLiteral("position")).toInt(), 3);
        QVERIFY(res.value(QStringLiteral("changed")).toBool());
        auto expectError = [&](quint64 s, const QString &code) {
            QTRY_VERIFY2(!raw.ackFor(s).isEmpty(), qPrintable(QString::number(s)));
            QCOMPARE(RawClient::code(raw.ackFor(s).value(QStringLiteral("outcome")).toMap()), code);
        };
        raw.control(contract::kJog, 1, {}, 10);  // duplicate sequence
        QTest::qWait(30);
        QCOMPARE(raw.acks.size(), 2);
        QCOMPARE(RawClient::code(raw.acks.last().value(QStringLiteral("outcome")).toMap()), contract::err::StaleSequence);
        QCOMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 3);  // not applied again
        raw.control(contract::kJog, std::numeric_limits<double>::quiet_NaN(), {}, 11);
        expectError(11, contract::err::InvalidArguments);
        QCOMPARE(raw.ackFor(11).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("session")).toString(), raw.session);  // errors carry it too
        raw.control(contract::kJog, std::numeric_limits<double>::infinity(), {}, 12);
        expectError(12, contract::err::InvalidArguments);
        raw.control(contract::kJog, 0.5, {}, 13);  // integral frames only
        expectError(13, contract::err::InvalidArguments);
        raw.control(contract::kJog, 1e9, {}, 14);  // out of range
        expectError(14, contract::err::InvalidArguments);
        raw.control(contract::kJog, 1, {{QStringLiteral("turbo"), true}}, 15);  // unknown behaviour-changing option
        expectError(15, contract::err::InvalidArguments);
        raw.control(contract::kJog, 1, {{QStringLiteral("gesture"), QStringLiteral("g")}}, 16);  // editing options on transport
        expectError(16, contract::err::InvalidArguments);
        raw.control(QStringLiteral("rocket.launch"), 1, {}, 17);
        expectError(17, contract::err::UnsupportedControl);
        raw.control(contract::kJog, 1, {{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(999)}}, 18);
        expectError(18, contract::err::StaleContext);
        QVariantMap big;
        for (int i = 0; i < 40; ++i) {
            big.insert(QStringLiteral("o%1").arg(i), i);
        }
        raw.control(contract::kJog, 1, big, 19);
        expectError(19, contract::err::InvalidArguments);
        raw.control(contract::kJog, 1, {{QStringLiteral("monitor"), QString(5000, QLatin1Char('x'))}}, 20);
        expectError(20, contract::err::InvalidArguments);
        // Wrong lease
        auto msg = raw.message(QStringLiteral("Control"));
        msg << contract::kJog << 1.0 << QVariant::fromValue(QVariantMap{{QStringLiteral("session"), QStringLiteral("forged")}, {QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(raw.epoch())}}) << QVariant::fromValue<qulonglong>(21);
        QDBusConnection(QStringLiteral("c%1").arg(m_conn)).send(msg);
        expectError(21, contract::err::NotSubscribed);
        QCOMPARE(raw.ackFor(21).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("session")).toString(), QStringLiteral("forged"));
        // Modal refusal is acked too.
        m_mock->setContextValue(QStringLiteral("dialog"), true);
        raw.control(contract::kJog, 1, {}, 22);
        expectError(22, contract::err::Modal);
        QCOMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 3);
    }

    void transportSemantics()
    {
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        m_mock->setContextValue(QStringLiteral("tool"), QStringLiteral("slip"));
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        const QStringList historyBefore = m_mock->history();
        raw.control(contract::kJog, 5, {{QStringLiteral("monitor"), QStringLiteral("project")}, {QStringLiteral("scrub"), true}}, 1);
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 5);
        QCOMPARE(m_mock->history(), historyBefore);  // pure transport even in Slip mode
        raw.control(contract::kJog, -50, {}, 2);       // net batch clamped once at 0
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 0);
        for (int i = 1; i <= 7; ++i) {
            const QVariantMap r = raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, double(i), QVariant::fromValue(raw.common())});
            QVERIFY(r.value(QStringLiteral("ok")).toBool());
        }
        QCOMPARE(m_mock->state().value(QStringLiteral("speed")).toDouble(), 60.0);
        QVERIFY(raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, -3.0, QVariant::fromValue(raw.common())}).value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->state().value(QStringLiteral("speed")).toDouble(), -4.0);
        QVERIFY(raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, 0.0, QVariant::fromValue(raw.common())}).value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->context().value(QStringLiteral("playing")).toBool(), false);  // zero pauses explicitly
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, 8.0, QVariant::fromValue(raw.common())})), contract::err::InvalidArguments);
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("SetControlValue"), {contract::kZoom, 3.0, QVariant::fromValue(raw.common())})), contract::err::UnsupportedControl);
        raw.control(contract::kZoom, 3, {{QStringLiteral("anchor"), QStringLiteral("playhead")}}, 3);
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("zoom")).toInt(), 13);
    }

    void actionsAcceptedThenFinished()
    {
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("TriggerAction"), {QStringLiteral("project_render"), QVariant::fromValue(raw.common())})), contract::err::UnknownAction);
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("TriggerAction"), {QStringLiteral("mark_in"), QVariant::fromValue(QVariantMap{})})), contract::err::NotSubscribed);
        const QVariantMap acc = raw.call(QStringLiteral("TriggerAction"), {QStringLiteral("mark_in"), QVariant::fromValue(raw.common())});
        QCOMPARE(acc.value(QStringLiteral("result")).toMap().value(QStringLiteral("state")).toString(), QStringLiteral("accepted"));
        const quint64 req = acc.value(QStringLiteral("result")).toMap().value(QStringLiteral("requestId")).toULongLong();
        QVERIFY(req > 0);
        QTRY_COMPARE(raw.finished.size(), 1);
        QCOMPARE(raw.finished[0].value(QStringLiteral("requestId")).toULongLong(), req);
        QCOMPARE(raw.finished[0].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap().value(QStringLiteral("state")).toString(), QStringLiteral("invoked"));
        // Accepted, then refused at dispatch (dialog opened in between): reported, not invoked.
        m_mock->setApplyDelayMs(50);
        const QVariantMap acc2 = raw.call(QStringLiteral("TriggerAction"), {QStringLiteral("mark_out"), QVariant::fromValue(raw.common())});
        QVERIFY(acc2.value(QStringLiteral("ok")).toBool());
        m_mock->setContextValue(QStringLiteral("dialog"), true);
        QTRY_COMPARE(raw.finished.size(), 2);
        QVERIFY(!raw.finished[1].value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(!m_mock->state().value(QStringLiteral("triggered")).toStringList().contains(QStringLiteral("mark_out")));
        // Notify bounds.
        m_mock->setContextValue(QStringLiteral("dialog"), false);
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        QVERIFY(raw.call(QStringLiteral("Notify"), {QStringLiteral("Lift: r"), 1500, QVariant::fromValue(raw.common())}).value(QStringLiteral("result")).toMap().value(QStringLiteral("shown")).toBool());
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Notify"), {QString(257, QLatin1Char('x')), 1500, QVariant::fromValue(raw.common())})), contract::err::InvalidArguments);
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Notify"), {QStringLiteral("x"), 100, QVariant::fromValue(raw.common())})), contract::err::InvalidArguments);
    }

    void gestureHistory()
    {
        RawClient raw(connectClient(), QString());
        m_mock->setContextValue(QStringLiteral("colorWheel"), wheelCtx());
        raw.subscribe();
        const QVariantMap g1{{QStringLiteral("target"), QStringLiteral("cw-1")}, {QStringLiteral("gesture"), QStringLiteral("g1")}, {QStringLiteral("wheel"), QStringLiteral("lift")}, {QStringLiteral("axis"), QStringLiteral("value")}};
        quint64 seq = 0;
        for (int i = 0; i < 20; ++i) {
            raw.control(contract::kColorWheel, 1, g1, ++seq);
        }
        QVariantMap end = g1;
        end.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kColorWheel, 0, end, ++seq);  // zero-delta end is a barrier
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(m_mock->history().size(), 1);  // 20 detents, one entry
        QVERIFY(qAbs(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("g")].toDouble() - 0.2) < 1e-9);
        const QVariantMap r = raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap();
        QCOMPARE(r.value(QStringLiteral("gesture")).toString(), QStringLiteral("g1"));
        QCOMPARE(r.value(QStringLiteral("target")).toString(), QStringLiteral("cw-1"));
        // A no-op gesture leaves no history.
        QVariantMap g2 = g1;
        g2.insert(QStringLiteral("gesture"), QStringLiteral("g2"));
        raw.control(contract::kColorWheel, 1, g2, ++seq);
        raw.control(contract::kColorWheel, -1, g2, ++seq);
        g2.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kColorWheel, 0, g2, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(m_mock->history().size(), 1);
        // Cancel restores while still owned.
        QVariantMap g3 = g1;
        g3.insert(QStringLiteral("gesture"), QStringLiteral("g3"));
        g3.insert(QStringLiteral("wheel"), QStringLiteral("gain"));
        raw.control(contract::kColorWheel, 50, g3, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        g3.insert(QStringLiteral("phase"), QStringLiteral("cancel"));
        raw.control(contract::kColorWheel, 0, g3, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("gain")].toMap()[QStringLiteral("r")].toDouble(), 1.0);
        QCOMPARE(m_mock->history().size(), 1);
        // Cancel after unrelated history: history_conflict, unrelated work untouched.
        QVariantMap g4 = g3;
        g4.insert(QStringLiteral("gesture"), QStringLiteral("g4"));
        g4.insert(QStringLiteral("phase"), QStringLiteral("update"));
        raw.control(contract::kColorWheel, 10, g4, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        m_mock->addUnrelatedHistory(QStringLiteral("user edit"));
        g4.insert(QStringLiteral("phase"), QStringLiteral("cancel"));
        raw.control(contract::kColorWheel, 0, g4, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::HistoryConflict);
        QVERIFY(m_mock->history().contains(QStringLiteral("user edit")));
        // Wrong or stale target and unadvertised axes.
        QVariantMap bad = g1;
        bad.insert(QStringLiteral("gesture"), QStringLiteral("g5"));
        bad.insert(QStringLiteral("target"), QStringLiteral("cw-404"));
        raw.control(contract::kColorWheel, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::TargetNotFound);
        bad.insert(QStringLiteral("target"), QStringLiteral("cw-1"));
        bad.insert(QStringLiteral("axis"), QStringLiteral("hue"));
        raw.control(contract::kColorWheel, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedMode);
        bad.insert(QStringLiteral("axis"), QStringLiteral("value"));
        bad.remove(QStringLiteral("gesture"));
        raw.control(contract::kColorWheel, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
    }

    void epochChangeInvalidatesQueuedWork()
    {
        RawClient raw(connectClient(), QString());
        m_mock->setContextValue(QStringLiteral("colorWheel"), wheelCtx());
        raw.subscribe();
        m_mock->setApplyDelayMs(40);
        const QVariantMap g{{QStringLiteral("target"), QStringLiteral("cw-1")}, {QStringLiteral("gesture"), QStringLiteral("q")}, {QStringLiteral("wheel"), QStringLiteral("lift")}};
        raw.control(contract::kColorWheel, 5, g, 1);
        QTest::qWait(5);
        QTRY_VERIFY(m_mock->controlMessages() >= 1);
        m_mock->setContextValue(QStringLiteral("colorWheel"), wheelCtx(QStringLiteral("cw-2")));  // focus moved
        QTRY_VERIFY(!raw.ackFor(1).isEmpty());
        QCOMPARE(RawClient::code(raw.ackFor(1).value(QStringLiteral("outcome")).toMap()), contract::err::StaleContext);
        QCOMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("r")].toDouble(), 0.0);
        // Playhead ticks change the serial but not the epoch.
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        const quint64 epoch = raw.epoch();
        m_mock->setPosition(77);
        QCOMPARE(m_mock->context().value(QStringLiteral("epoch")).toULongLong(), epoch);
    }

    void daemonEndToEnd()
    {
        auto conn = connectClient();
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_VERIFY(client.isAvailable());
        QCOMPARE(m_mock->leaseCount(), 1);
        QVERIFY(client.supportsAction(QStringLiteral("mark_in")));

        m_mock->setApplyDelayMs(15);  // a busy GUI thread
        for (int i = 0; i < 200; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0});
            if (i % 10 == 0) {
                QTest::qWait(1);
            }
        }
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->state().value(QStringLiteral("position")).toInt(), 200, 3000);
        QVERIFY2(m_mock->controlMessages() < 40, qPrintable(QString::number(m_mock->controlMessages())));
        QCOMPARE(client.inFlightMessages(), 0);

        // Actions over D-Bus; ActionFinished reports invocation.
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_VERIFY(m_mock->state().value(QStringLiteral("triggered")).toStringList().contains(QStringLiteral("mark_in")));
        // A refusal is reported and never typed.
        m_mock->setContextValue(QStringLiteral("dialog"), true);
        QSignalSpy refused(&client, &KdenliveClient::refused);
        e.handle(PadEvent{QStringLiteral("key2"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(refused.size(), 1);
        QCOMPARE(refused[0][1].toString(), contract::err::Modal);
        QTest::qWait(30);
        QVERIFY(keys.taps.isEmpty());
        m_mock->setContextValue(QStringLiteral("dialog"), false);

        // Colour wheel layer from pushed context; press cycles to the red channel.
        m_mock->setContextValue(QStringLiteral("colorWheel"), wheelCtx());
        QTRY_COMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob1"), 1))->layer, QStringLiteral("color-wheels"));
        for (int i = 0; i < 5; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0});
        }
        e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::PressDown, 0, 0});
        for (int i = 0; i < 3; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, -1, 0});
        }
        QTRY_VERIFY(qAbs(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("r")].toDouble() - 0.02) < 1e-9);
        QVERIFY(qAbs(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("g")].toDouble() - 0.05) < 1e-9);
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->history().size(), 2, 2000);  // two gestures, two entries
        QVERIFY(m_mock->log.join(QLatin1Char('\n')).contains(QStringLiteral("Notify \"Lift: r\"")));

        e.setActiveWindow(WindowInfo{QStringLiteral("firefox"), {}, 5, {}});
        QTRY_COMPARE(m_mock->leaseCount(), 0);
    }

    void stockKdenliveWithoutInterface()
    {
        m_register = false;  // Kdenlive with the interface off (its default)
        auto conn = connectClient();
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_COMPARE(client.state(), State::Absent);
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("I")});  // stock shortcut
    }

    void incompatibleRevisionIsTreatedAsAbsent()
    {
        m_old = new QObject;
        m_oldAdaptor = new OldAdaptor(m_old);
        auto conn = connectClient();
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        client.attachToPid(1);
        QTRY_COMPARE(client.state(), State::Absent);
        QCOMPARE(m_oldAdaptor->subscribes, 0);  // never subscribed to, never driven
    }
};

QTEST_GUILESS_MAIN(TestKdenliveDBus)
#include "tst_kdenlive_dbus.moc"
