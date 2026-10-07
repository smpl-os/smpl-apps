// SPDX-License-Identifier: GPL-2.0-or-later
// End to end over real D-Bus marshalling: Engine -> KdenliveDBusClient -> peer
// connection -> MockKdenlive implementing docs/kdenlive-api-contract.md.
#include "engine.h"
#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"
#include "mockkdenlive.h"

#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusServer>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

class TestKdenliveDBus : public QObject
{
    Q_OBJECT
    QDBusServer *m_server = nullptr;
    MockKdenlive *m_mock = nullptr;
    QString m_peerName;
    bool m_register = true;
    Config m_cfg;

    QDBusConnection connectClient(const QString &name)
    {
        QDBusConnection c = QDBusConnection::connectToPeer(m_server->address(), name);
        return c;
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
            if (m_register) {
                QVERIFY(m_mock->registerOn(conn));
            }
            m_peerName = conn.name();
        });
    }

    void cleanup()
    {
        if (!m_peerName.isEmpty()) {
            QDBusConnection::disconnectFromPeer(m_peerName);
        }
        m_peerName.clear();
        delete m_mock;
        delete m_server;
    }

    void subscribeAndContext()
    {
        auto conn = connectClient(QStringLiteral("c1"));
        QVERIFY(conn.isConnected());
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        QSignalSpy avail(&client, &KdenliveClient::availabilityChanged);
        client.attachToPid(1);
        QTRY_VERIFY(client.isAvailable());
        QCOMPARE(m_mock->subscriberCount(), 1);
        QCOMPARE(client.context().value(QStringLiteral("focus")).toString(), QStringLiteral("timeline"));

        // Nested maps survive the wire and the change is pushed.
        m_mock->setContextValue(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}, {QStringLiteral("index"), 2}});
        QTRY_COMPARE(client.context().value(QStringLiteral("effect")).toMap().value(QStringLiteral("id")).toString(), QStringLiteral("lift_gamma_gain"));
        QCOMPARE(client.context().value(QStringLiteral("effect")).toMap().value(QStringLiteral("index")).toInt(), 2);

        // A burst of context changes is rate limited, the last state still arrives.
        const int before = m_mock->contextSignals();
        for (int i = 0; i < 50; ++i) {
            m_mock->setContextValue(QStringLiteral("position"), i);
        }
        QTRY_COMPARE(client.context().value(QStringLiteral("position")).toInt(), 49);
        QVERIFY(m_mock->contextSignals() - before <= 3);

        client.attachToPid(0);
        QTRY_COMPARE(m_mock->subscriberCount(), 0);
        QVERIFY(!client.isAvailable());
        const int quiet = m_mock->contextSignals();
        m_mock->setContextValue(QStringLiteral("position"), 1);
        QTest::qWait(60);
        QCOMPARE(m_mock->contextSignals(), quiet);  // no subscribers: no signal traffic
        QDBusConnection::disconnectFromPeer(QStringLiteral("c1"));
    }

    void engineDrivesMockWithCoalescing()
    {
        auto conn = connectClient(QStringLiteral("c2"));
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_VERIFY(e.kdenliveActive());

        m_mock->setApplyDelayMs(15);  // a busy GUI thread
        for (int i = 0; i < 200; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0});
            if (i % 10 == 0) {
                QTest::qWait(1);
            }
        }
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->state().value(QStringLiteral("position")).toInt(), 200, 3000);
        // 200 detents, far fewer messages, and every detent accounted for.
        QVERIFY2(m_mock->controlMessages() < 40, qPrintable(QString::number(m_mock->controlMessages())));
        QVERIFY(m_mock->applyBatches() <= m_mock->controlMessages());

        // Shuttle uses Kdenlive's speed table, clamped at 60x.
        for (int i = 0; i < 10; ++i) {
            e.handle(PadEvent{QStringLiteral("knob2"), PadEvent::Turn, 1, 0});
        }
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("speed")).toDouble(), 60.0);

        // Actions over D-Bus; unknown ones fall back to keys.
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_VERIFY(m_mock->state().value(QStringLiteral("triggered")).toStringList().contains(QStringLiteral("mark_in")));
        QVERIFY(keys.taps.isEmpty());
        QSignalSpy failed(&client, &KdenliveClient::actionFailed);
        client.triggerAction(QStringLiteral("no_such_action"));
        QTRY_COMPARE(failed.size(), 1);

        // Context-driven layer: lift wheel, press cycles to the red channel.
        m_mock->setContextValue(QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}});
        QTRY_COMPARE(e.resolve(QStringLiteral("knob1.turn"))->layer, QStringLiteral("color-wheels"));
        for (int i = 0; i < 5; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0});
        }
        e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::PressDown, 0, 0});
        for (int i = 0; i < 3; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, -1, 0});
        }
        QTRY_VERIFY(qAbs(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("r")].toDouble() - 0.02) < 1e-9);
        const QVariantMap lift = m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap();
        QVERIFY(qAbs(lift[QStringLiteral("g")].toDouble() - 0.05) < 1e-9);
        QVERIFY(m_mock->log.join(QLatin1Char('\n')).contains(QStringLiteral("Notify \"Lift: r\"")));
        // Gamma wheel untouched.
        QCOMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("gamma")].toMap()[QStringLiteral("r")].toDouble(), 1.0);
        e.handle(PadEvent{QStringLiteral("key6"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("g")].toDouble(), 0.0);

        e.setActiveWindow(WindowInfo{QStringLiteral("firefox"), {}, 5, {}});
        QTRY_COMPARE(m_mock->subscriberCount(), 0);
        QDBusConnection::disconnectFromPeer(QStringLiteral("c2"));
    }

    void staleActionReplyIsIgnored()
    {
        auto conn = connectClient(QStringLiteral("c5"));
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        client.attachToPid(1);
        QTRY_VERIFY(client.isAvailable());
        QSignalSpy failed(&client, &KdenliveClient::actionFailed);
        client.triggerAction(QStringLiteral("no_such_action"));  // answers false...
        client.attachToPid(0);                                    // ...but focus left first
        QTest::qWait(100);
        QCOMPARE(failed.size(), 0);
        QDBusConnection::disconnectFromPeer(QStringLiteral("c5"));
    }

    void listActionsAndCapabilitiesOnTheWire()
    {
        auto conn = connectClient(QStringLiteral("c3"));
        auto msg = QDBusMessage::createMethodCall(QString(), contract::kPath, contract::kInterface, QStringLiteral("ListActions"));
        QDBusMessage reply = conn.call(msg, QDBus::BlockWithGui, 2000);
        QCOMPARE(reply.type(), QDBusMessage::ReplyMessage);
        QCOMPARE(reply.signature(), QStringLiteral("aa{sv}"));
        const QVariantList actions = KdenliveDBusClient::normalize(reply.arguments().constFirst()).toList();
        QVERIFY(actions.size() > 20);
        QCOMPARE(actions.first().toMap().value(QStringLiteral("id")).toString(), QStringLiteral("monitor_play"));

        msg = QDBusMessage::createMethodCall(QString(), contract::kPath, contract::kInterface, QStringLiteral("Capabilities"));
        reply = conn.call(msg, QDBus::BlockWithGui, 2000);
        const QVariantMap caps = KdenliveDBusClient::normalize(reply.arguments().constFirst()).toMap();
        QCOMPARE(caps.value(QStringLiteral("version")).toInt(), 1);
        QVERIFY(caps.value(QStringLiteral("controls")).toStringList().contains(contract::kColorWheel));
        QDBusConnection::disconnectFromPeer(QStringLiteral("c3"));
    }

    void stockKdenliveWithoutInterface()
    {
        // A Kdenlive without the interface: nothing registered at the path.
        m_register = false;
        auto conn = connectClient(QStringLiteral("c4"));
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        QSignalSpy msgs(&client, &KdenliveClient::message);
        client.attachToPid(1);
        QTRY_VERIFY(!msgs.isEmpty());
        QVERIFY2(msgs.first().first().toString().contains(QStringLiteral("key fallbacks")), qPrintable(msgs.first().first().toString()));
        QVERIFY(!client.isAvailable());
        // The engine then types the configured fallback keys.
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("I")});
        m_register = true;
        QDBusConnection::disconnectFromPeer(QStringLiteral("c4"));
    }
};

QTEST_GUILESS_MAIN(TestKdenliveDBus)
#include "tst_kdenlive_dbus.moc"
