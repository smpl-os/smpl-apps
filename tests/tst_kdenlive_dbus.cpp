// SPDX-License-Identifier: GPL-2.0-or-later
// Contract revision 2 over real D-Bus marshalling on a peer connection:
// envelopes, staged capabilities, validation, sequences, actions, gestures,
// and Engine -> KdenliveDBusClient -> MockKdenlive end to end.
#include "capabilities.h"
#include "engine.h"
#include "kdenlivecontract.h"
#include "kdenlivedbusclient.h"
#include "mockkdenlive.h"
#include "rawclient.h"

#include <QDBusConnection>
#include <QDBusServer>
#include <QJsonObject>
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
    static QString cw(const char *wheel) { return MockKdenlive::wheelTarget(QString::fromLatin1(wheel)); }
    static double wheelValue(MockKdenlive *m, const char *wheel, const char *channel)
    {
        return m->state()[QStringLiteral("wheels")].toMap()[QString::fromLatin1(wheel)].toMap()[QString::fromLatin1(channel)].toDouble();
    }

private Q_SLOTS:
    void initTestCase()
    {
        QTemporaryDir home;
        qputenv("HOME", home.path().toLocal8Bit());
        QString err;
        auto c = loadConfig(QStringLiteral(CS_SOURCE_DIR "/tests/data/engine-test-config.jsonc"), &err);
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
        // Limit names as published by Kdenlive's MR2 implementation.
        const QVariantMap limits = r.value(QStringLiteral("limits")).toMap();
        QCOMPARE(limits.value(QStringLiteral("subscriptions")).toInt(), 8);
        QCOMPARE(limits.value(QStringLiteral("pendingKeys")).toInt(), 64);
        QCOMPARE(limits.value(QStringLiteral("queuedActions")).toInt(), 32);
        QCOMPARE(limits.value(QStringLiteral("contextHz")).toInt(), 30);
        QCOMPARE(limits.value(QStringLiteral("maximumDelta")).toInt(), 10000);
        QVERIFY(r.value(QStringLiteral("contextKeys")).toStringList().contains(QStringLiteral("emittedAtMs")));
        QVERIFY(!r.value(QStringLiteral("contextKeys")).toStringList().contains(QStringLiteral("colorWheels")));  // stage 1
        QVERIFY(!r.contains(QStringLiteral("controlDescriptors")));  // optional; absent in the qualified host
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
        // Before MR3 the timeline is described (native track identity) but offers no handles.
        QCOMPARE(raw.context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("track")).toMap().value(QStringLiteral("id")).toInt(), 7);
        QVERIFY(!raw.context.value(QStringLiteral("timeline")).toMap().value(QStringLiteral("track")).toMap().contains(QStringLiteral("target")));
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
        raw.control(contract::kJog, 1, big, 19);  // oversized option count
        expectError(19, contract::err::ResourceLimit);
        raw.control(contract::kJog, 1, {{QStringLiteral("monitor"), QString(5000, QLatin1Char('x'))}}, 20);  // oversized string input
        expectError(20, contract::err::ResourceLimit);
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
        // MR1a: Slip's monitor trimming preview refuses playback (busy, not an
        // editing-writer conflict); zero still pauses. select_tool + fresh epoch.
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, 2.0, QVariant::fromValue(raw.common())})), contract::err::Busy);
        raw.control(contract::kShuttle, 1, {}, 3);
        QTRY_VERIFY(!raw.ackFor(3).isEmpty());
        QCOMPARE(RawClient::code(raw.ackFor(3).value(QStringLiteral("outcome")).toMap()), contract::err::Busy);
        QVERIFY(!m_mock->context().value(QStringLiteral("playing")).toBool());  // no fake playing state
        QVERIFY(raw.call(QStringLiteral("SetControlValue"), {contract::kShuttle, 0.0, QVariant::fromValue(raw.common())}).value(QStringLiteral("ok")).toBool());
        m_mock->setContextValue(QStringLiteral("tool"), QStringLiteral("select"));
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
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
        raw.control(contract::kZoom, 3, {{QStringLiteral("anchor"), QStringLiteral("playhead")}}, 4);
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
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Notify"), {QString(257, QLatin1Char('x')), 1500, QVariant::fromValue(raw.common())})), contract::err::ResourceLimit);  // oversized text
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Notify"), {QStringLiteral("x"), 100, QVariant::fromValue(raw.common())})), contract::err::InvalidArguments);
    }

    void gestureHistory()
    {
        RawClient raw(connectClient(), QString());
        m_mock->focusWheels(QStringLiteral("lift"));
        raw.subscribe();
        const QVariantMap g1{{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("g1")}, {QStringLiteral("wheel"), QStringLiteral("lift")}, {QStringLiteral("axis"), QStringLiteral("value")}};
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
        QCOMPARE(r.value(QStringLiteral("target")).toString(), cw("lift"));
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
        g3.insert(QStringLiteral("target"), cw("gain"));
        g3.insert(QStringLiteral("wheel"), QStringLiteral("gain"));
        raw.control(contract::kColorWheel, 50, g3, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        g3.insert(QStringLiteral("phase"), QStringLiteral("cancel"));
        raw.control(contract::kColorWheel, 0, g3, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("gain")].toMap()[QStringLiteral("r")].toDouble(), 1.0);
        QCOMPARE(m_mock->history().size(), 1);
        // The cancel ack and the published context show the restored values.
        const QVariantMap cancelled = raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("result")).toMap();
        QVERIFY(cancelled.value(QStringLiteral("changed")).toBool());  // something owned was reverted
        QCOMPARE(cancelled.value(QStringLiteral("values")).toMap().value(QStringLiteral("r")).toDouble(), 1.0);
        const QVariantMap published = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        QCOMPARE(published.value(QStringLiteral("colorWheels")).toList().at(2).toMap().value(QStringLiteral("values")).toMap().value(QStringLiteral("r")).toDouble(), 1.0);
        QTRY_COMPARE(raw.context.value(QStringLiteral("colorWheels")).toList().at(2).toMap().value(QStringLiteral("values")).toMap().value(QStringLiteral("r")).toDouble(), 1.0);
        // Cancel after unrelated history: history_conflict, unrelated work untouched.
        QVariantMap g4 = g3;
        g4.insert(QStringLiteral("gesture"), QStringLiteral("g4"));
        g4.insert(QStringLiteral("phase"), QStringLiteral("update"));
        raw.control(contract::kColorWheel, 10, g4, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        m_mock->addUnrelatedHistory(QStringLiteral("user edit"));  // ends the gesture, new epoch
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
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
        bad.insert(QStringLiteral("target"), cw("lift"));
        bad.insert(QStringLiteral("axis"), QStringLiteral("hue"));
        raw.control(contract::kColorWheel, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedMode);
        bad.insert(QStringLiteral("axis"), QStringLiteral("value"));
        bad.remove(QStringLiteral("gesture"));
        raw.control(contract::kColorWheel, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
    }

    // Review item 4: dispatch re-checks inactive/not_ready/closing/modal.
    void dispatchRechecksEditorState()
    {
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        m_mock->setApplyDelayMs(40);
        const struct {
            const char *key;
            QVariant value;
            QString code;
        } cases[] = {{"active", false, contract::err::Inactive}, {"ready", false, contract::err::NotReady}, {"closing", true, contract::err::Closing}};
        quint64 seq = 0;
        for (const auto &c : cases) {
            raw.control(contract::kJog, 4, {}, ++seq);  // admitted
            QTRY_VERIFY(m_mock->controlMessages() >= int(seq));
            m_mock->setContextValue(QString::fromLatin1(c.key), c.value);  // state changes before the apply
            QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
            QCOMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), c.code);
            QCOMPARE(m_mock->state().value(QStringLiteral("position")).toInt(), 0);
            m_mock->setContextValue(QString::fromLatin1(c.key), c.value.toBool() ? QVariant(false) : QVariant(true));
            raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        }
        // Queued actions too.
        QVERIFY(raw.call(QStringLiteral("TriggerAction"), {QStringLiteral("mark_in"), QVariant::fromValue(raw.common())}).value(QStringLiteral("ok")).toBool());
        m_mock->setContextValue(QStringLiteral("active"), false);
        QTRY_COMPARE(raw.finished.size(), 1);
        QCOMPARE(RawClient::code(raw.finished[0].value(QStringLiteral("outcome")).toMap()), contract::err::Inactive);
        QVERIFY(!m_mock->state().value(QStringLiteral("triggered")).toStringList().contains(QStringLiteral("mark_in")));
    }

    // Review item 5: a gesture is judged on its own parameter, not the newly focused one.
    void parameterGestureAcrossFocusChange()
    {
        m_mock->focusParam(QStringLiteral("level"));
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        const QVariantMap g{{QStringLiteral("target"), QStringLiteral("par-level")}, {QStringLiteral("gesture"), QStringLiteral("p1")}};
        raw.control(contract::kParamNudge, 1, g, 1);
        QTRY_VERIFY(!raw.ackFor(1).isEmpty());
        raw.control(contract::kParamNudge, -1, g, 2);
        QTRY_VERIFY(!raw.ackFor(2).isEmpty());
        // Focus moves to another parameter (opacity = 100): the level gesture ends as a no-op.
        m_mock->focusParam(QStringLiteral("opacity"));
        QVERIFY(m_mock->history().isEmpty());
        QCOMPARE(m_mock->state()[QStringLiteral("params")].toMap()[QStringLiteral("level")].toDouble(), 50.0);
        // A real edit ended by a focus change makes exactly one entry.
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        const QVariantMap g2{{QStringLiteral("target"), QStringLiteral("par-opacity")}, {QStringLiteral("gesture"), QStringLiteral("p2")}};
        raw.control(contract::kParamNudge, -5, g2, 3);
        QTRY_VERIFY(!raw.ackFor(3).isEmpty());
        m_mock->focusParam(QStringLiteral("level"));
        QCOMPARE(m_mock->history().size(), 1);
        QCOMPARE(m_mock->state()[QStringLiteral("params")].toMap()[QStringLiteral("opacity")].toDouble(), 95.0);
    }

    // Review item 6: a gesture the host already ended is never "cancelled"
    // successfully. MR2: a late update or end with that id starts a new gesture
    // (Kdenlive's behaviour); a late end applies its delta once and ends it.
    void lateCancelOfEndedGesture()
    {
        m_mock->focusWheels(QStringLiteral("gain"));
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        QVariantMap g{{QStringLiteral("target"), cw("gain")}, {QStringLiteral("gesture"), QStringLiteral("idle")}, {QStringLiteral("wheel"), QStringLiteral("gain")}};
        raw.control(contract::kColorWheel, -5, g, 1);
        QTRY_VERIFY(!raw.ackFor(1).isEmpty());
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->history().size(), 1, 2000);  // host idle end after 600 ms
        g.insert(QStringLiteral("phase"), QStringLiteral("cancel"));
        raw.control(contract::kColorWheel, 0, g, 2);
        QTRY_VERIFY(!raw.ackFor(2).isEmpty());
        QCOMPARE(RawClient::code(raw.ackFor(2).value(QStringLiteral("outcome")).toMap()), contract::err::HistoryConflict);
        QVERIFY(qAbs(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("gain")].toMap()[QStringLiteral("r")].toDouble() - 0.95) < 1e-9);
        g.insert(QStringLiteral("phase"), QStringLiteral("update"));
        raw.control(contract::kColorWheel, 1, g, 3);  // late update of the ended gesture: a new gesture
        QTRY_VERIFY(!raw.ackFor(3).isEmpty());
        QVERIFY(raw.ackFor(3).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(qAbs(wheelValue(m_mock, "gain", "r") - 0.96) < 1e-9);
        g.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kColorWheel, 0, g, 4);  // ends it: one more entry
        QTRY_VERIFY(!raw.ackFor(4).isEmpty());
        QVERIFY(raw.ackFor(4).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 2);
        raw.control(contract::kColorWheel, 0, g, 5);  // a repeated zero-delta end: no-op, no history
        QTRY_VERIFY(!raw.ackFor(5).isEmpty());
        QVERIFY(raw.ackFor(5).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 2);
        raw.control(contract::kColorWheel, 2, g, 6);  // a late end with a delta applies it once
        QTRY_VERIFY(!raw.ackFor(6).isEmpty());
        QVERIFY(qAbs(wheelValue(m_mock, "gain", "r") - 0.98) < 1e-9);
        QCOMPARE(m_mock->history().size(), 3);
    }

    void epochChangeInvalidatesQueuedWork()
    {
        RawClient raw(connectClient(), QString());
        m_mock->focusWheels(QStringLiteral("lift"));
        raw.subscribe();
        m_mock->setApplyDelayMs(40);
        const QVariantMap g{{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("q")}, {QStringLiteral("wheel"), QStringLiteral("lift")}};
        raw.control(contract::kColorWheel, 5, g, 1);
        QTest::qWait(5);
        QTRY_VERIFY(m_mock->controlMessages() >= 1);
        m_mock->focusWheels(QStringLiteral("gamma"));  // keyboard focus moved to another wheel: new epoch
        QTRY_VERIFY(!raw.ackFor(1).isEmpty());
        QCOMPARE(RawClient::code(raw.ackFor(1).value(QStringLiteral("outcome")).toMap()), contract::err::StaleContext);
        QCOMPARE(m_mock->state()[QStringLiteral("wheels")].toMap()[QStringLiteral("lift")].toMap()[QStringLiteral("r")].toDouble(), 0.0);
        // Playhead ticks change the serial but not the epoch.
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        const quint64 epoch = raw.epoch();
        m_mock->setPosition(77);
        QCOMPARE(m_mock->context().value(QStringLiteral("epoch")).toULongLong(), epoch);
    }

    // MR2 context: three per-wheel handles beside the focused wheel; values
    // update without a new epoch; validation as in Kdenlive's MR2.
    void mr2WheelHandlesAndValidation()
    {
        m_mock->setStage(2);
        m_mock->focusWheels(QStringLiteral("gamma"));
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        const QVariantList wheels = raw.context.value(QStringLiteral("colorWheels")).toList();
        QCOMPARE(wheels.size(), 3);
        QStringList kinds;
        for (const auto &w : wheels) {
            const QVariantMap d = w.toMap();
            kinds << d.value(QStringLiteral("wheel")).toString();
            QCOMPARE(d.value(QStringLiteral("target")).toString(), MockKdenlive::wheelTarget(kinds.last()));
            QCOMPARE(d.value(QStringLiteral("axes")).toStringList(), (QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}));
            for (const char *k : {"min", "max", "step", "fineStep", "frame", "keyframed", "enabled", "values"}) {
                QVERIFY2(d.contains(QString::fromLatin1(k)), k);
            }
        }
        QCOMPARE(kinds, (QStringList{QStringLiteral("lift"), QStringLiteral("gamma"), QStringLiteral("gain")}));
        QCOMPARE(raw.context.value(QStringLiteral("colorWheel")).toMap().value(QStringLiteral("wheel")).toString(), QStringLiteral("gamma"));
        QCOMPARE(raw.context.value(QStringLiteral("focus")).toString(), QStringLiteral("effectStack"));
        QVERIFY(!raw.context.value(QStringLiteral("effect")).toMap().contains(QStringLiteral("target")));  // effect is described, not targeted
        QCOMPARE(raw.context.value(QStringLiteral("colorWheels")).toList().at(0).toMap().value(QStringLiteral("values")).toMap().value(QStringLiteral("r")).toDouble(), 0.5);  // lift shown as (v+1)/2
        const quint64 epoch = raw.epoch();
        // Lift is not keyboard-focused, but its handle is addressable; fractional steps apply.
        QVariantMap g{{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("h1")}, {QStringLiteral("wheel"), QStringLiteral("lift")}, {QStringLiteral("axis"), QStringLiteral("r")}};
        raw.control(contract::kColorWheel, 2.5, g, 1);
        QTRY_VERIFY(!raw.ackFor(1).isEmpty());
        QVERIFY(raw.ackFor(1).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(qAbs(wheelValue(m_mock, "lift", "r") - 0.025) < 1e-9);
        QTRY_VERIFY(qAbs(raw.context.value(QStringLiteral("colorWheels")).toList().at(0).toMap().value(QStringLiteral("values")).toMap().value(QStringLiteral("r")).toDouble() - 0.5125) < 1e-9);
        QCOMPARE(raw.epoch(), epoch);  // a value change is not a retarget
        QVERIFY(raw.context.contains(QStringLiteral("emittedAtMs")));
        // A wheel option that contradicts the handle is unsupported_parameter.
        QVariantMap wrong = g;
        wrong.insert(QStringLiteral("wheel"), QStringLiteral("gain"));
        raw.control(contract::kColorWheel, 1, wrong, 2);
        QTRY_COMPARE(RawClient::code(raw.ackFor(2).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedParameter);
        // The wheel option is optional: the handle names the wheel.
        QVariantMap bare = g;
        bare.remove(QStringLiteral("wheel"));
        bare.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kColorWheel, 0, bare, 3);
        QTRY_VERIFY(raw.ackFor(3).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 1);
        // Delta bounds: |delta| <= 10000 for relative parameter controls.
        raw.control(contract::kColorWheel, 10001, g, 4);
        QTRY_COMPARE(RawClient::code(raw.ackFor(4).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        // Resets: per handle; a contradicting wheel option is refused.
        QVariantMap reset = raw.common();
        reset.insert(QStringLiteral("target"), cw("lift"));
        QVERIFY(raw.call(QStringLiteral("Invoke"), {contract::kCmdWheelReset, QVariant::fromValue(reset)}).value(QStringLiteral("ok")).toBool());
        QCOMPARE(wheelValue(m_mock, "lift", "r"), 0.0);
        reset.insert(QStringLiteral("wheel"), QStringLiteral("gamma"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdWheelReset, QVariant::fromValue(reset)})), contract::err::UnsupportedParameter);
        const QVariantMap unknown = raw.call(QStringLiteral("Invoke"), {QStringLiteral("project.render"), QVariant::fromValue(raw.common())});
        QCOMPARE(RawClient::code(unknown), contract::err::UnsupportedControl);
        QCOMPARE(unknown.value(QStringLiteral("error")).toMap().value(QStringLiteral("field")).toString(), QString());  // empty, as the host
        QVariantMap extra = raw.common();
        extra.insert(QStringLiteral("frobnicate"), true);  // options are validated before dispatch
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {QStringLiteral("project.render"), QVariant::fromValue(extra)})), contract::err::InvalidArguments);
        // Transport refuses parameter options; parameter controls share one option set.
        raw.control(contract::kJog, 1, {{QStringLiteral("wheel"), QStringLiteral("lift")}}, 5);
        QTRY_COMPARE(RawClient::code(raw.ackFor(5).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        // Numeric parameters: only the value axis; fractional steps; param.focus is integral and bounded.
        m_mock->focusParam(QStringLiteral("level"));
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        QCOMPARE(raw.context.value(QStringLiteral("param")).toMap().value(QStringLiteral("type")).toString(), QStringLiteral("number"));
        QVariantMap p{{QStringLiteral("target"), QStringLiteral("par-level")}, {QStringLiteral("gesture"), QStringLiteral("p1")}, {QStringLiteral("axis"), QStringLiteral("value")}};
        raw.control(contract::kParamNudge, 1, p, 6);  // param.nudge takes no axis/wheel options
        QTRY_COMPARE(RawClient::code(raw.ackFor(6).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        p.remove(QStringLiteral("axis"));
        p.insert(QStringLiteral("step"), QStringLiteral("fine"));
        raw.control(contract::kParamNudge, 2.5, p, 7);
        QTRY_VERIFY(raw.ackFor(7).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(qAbs(m_mock->state()[QStringLiteral("params")].toMap()[QStringLiteral("level")].toDouble() - 50.3) < 1e-9);  // 2.5 x 0.1, quantised
        raw.control(contract::kParamFocus, 1, {{QStringLiteral("step"), QStringLiteral("normal")}}, 8);  // param.focus takes no options
        QTRY_COMPARE(RawClient::code(raw.ackFor(8).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        raw.control(contract::kParamFocus, 1, {}, 11);
        QTRY_VERIFY(raw.ackFor(11).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        raw.control(contract::kParamFocus, 0.5, {}, 9);
        QTRY_COMPARE(RawClient::code(raw.ackFor(9).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        raw.control(contract::kParamFocus, 65, {}, 10);
        QTRY_COMPARE(RawClient::code(raw.ackFor(10).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
    }

    // One active editing gesture per host: another caller is busy; the same
    // caller's new gesture ends its previous one first.
    void singleActiveGestureAndBusy()
    {
        m_mock->focusWheels(QStringLiteral("lift"));
        RawClient a(connectClient(), QString());
        RawClient b(connectClient(), QString());
        a.subscribe();
        b.subscribe();
        QTRY_COMPARE(m_mock->leaseCount(), 2);
        const QVariantMap ga{{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("a1")}};
        a.control(contract::kColorWheel, 5, ga, 1);
        QTRY_VERIFY(!a.ackFor(1).isEmpty());
        const QVariantMap gb{{QStringLiteral("target"), cw("gamma")}, {QStringLiteral("gesture"), QStringLiteral("b1")}};
        b.control(contract::kColorWheel, 5, gb, 1);
        QTRY_VERIFY(!b.ackFor(1).isEmpty());
        QCOMPARE(RawClient::code(b.ackFor(1).value(QStringLiteral("outcome")).toMap()), contract::err::Busy);
        QCOMPARE(wheelValue(m_mock, "gamma", "r"), 1.0);
        QCOMPARE(a.acks.size(), 1);  // the refusal went to B only
        // A switches to another wheel: its lift gesture is committed first.
        const QVariantMap ga2{{QStringLiteral("target"), cw("gamma")}, {QStringLiteral("gesture"), QStringLiteral("a2")}};
        a.control(contract::kColorWheel, 3, ga2, 2);
        QTRY_VERIFY(!a.ackFor(2).isEmpty());
        QVERIFY(a.ackFor(2).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 1);
        QVariantMap end = ga2;
        end.insert(QStringLiteral("phase"), QStringLiteral("end"));
        a.control(contract::kColorWheel, 0, end, 3);
        QTRY_VERIFY(!a.ackFor(3).isEmpty());
        QCOMPARE(m_mock->history().size(), 2);
        // Free again: B may edit.
        b.control(contract::kColorWheel, 1, gb, 2);
        QTRY_VERIFY(!b.ackFor(2).isEmpty());
        QVERIFY(b.ackFor(2).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
    }

    // The playhead moving or another control applying ends an editing gesture
    // (its captured frame/scope changed); grouped propagation is refused.
    // Live grading: a whole-clip (static) gesture survives playback clock
    // ticks and stays one undo entry; applying any non-editing control (jog,
    // zoom) ends it, as in Kdenlive; grouped propagation is refused.
    void liveGradingGestureSurvivesPlaybackTicks()
    {
        m_mock->focusWheels(QStringLiteral("gain"));
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        QVERIFY(raw.context.value(QStringLiteral("colorWheels")).toList().at(2).toMap().value(QStringLiteral("liveGrading")).toBool());
        m_mock->setPlaying(true);
        QVariantMap g{{QStringLiteral("target"), cw("gain")}, {QStringLiteral("gesture"), QStringLiteral("p")}};
        raw.control(contract::kColorWheel, 4, g, 1);
        QTRY_VERIFY(raw.ackFor(1).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        for (int f = 1; f <= 5; ++f) {
            m_mock->setPosition(f);  // playback ticks
        }
        raw.control(contract::kColorWheel, 4, g, 2);
        QTRY_VERIFY(raw.ackFor(2).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(m_mock->history().isEmpty());  // still the one open gesture
        raw.control(contract::kZoom, 1, {}, 3);  // a view/transport control ends it
        QTRY_VERIFY(!raw.ackFor(3).isEmpty());
        QCOMPARE(m_mock->history().size(), 1);  // one entry for both updates
        QVERIFY(qAbs(wheelValue(m_mock, "gain", "r") - 1.08) < 1e-9);
        // The mock's wheels are static: explicit key creation is unsupported.
        QVariantMap create{{QStringLiteral("target"), cw("gain")}, {QStringLiteral("gesture"), QStringLiteral("c")}, {QStringLiteral("keyframe"), QStringLiteral("create")}};
        raw.control(contract::kColorWheel, 1, create, 4);
        QTRY_COMPARE(RawClient::code(raw.ackFor(4).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedParameter);
        m_mock->setPlaying(false);
        // Grouped propagation is not qualified: refused, nothing changes.
        m_mock->setGroupedPropagation(true);
        g.insert(QStringLiteral("gesture"), QStringLiteral("grp"));
        raw.control(contract::kColorWheel, 4, g, 7);
        QTRY_COMPARE(RawClient::code(raw.ackFor(7).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedGroup);
        QVariantMap reset = raw.common();
        reset.insert(QStringLiteral("target"), cw("gain"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdWheelReset, QVariant::fromValue(reset)})), contract::err::UnsupportedGroup);
        QVERIFY(qAbs(wheelValue(m_mock, "gain", "r") - 1.08) < 1e-9);
        QCOMPARE(m_mock->history().size(), 1);
    }

    // Multi-key parameters: an existing key at the captured frame or explicit
    // create, stopped playback only, and a seek ends the gesture; cancel
    // removes a created key; a cancel without a live gesture conflicts.
    void multiKeyEditsNeedStoppedPlayback()
    {
        m_mock->focusParam(QStringLiteral("level"));
        m_mock->setParamMultiKey(QStringLiteral("level"), true);  // keys at 0 and 100
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        const QVariantMap desc = raw.context.value(QStringLiteral("param")).toMap();
        QVERIFY(desc.value(QStringLiteral("keyframed")).toBool());
        QVERIFY(!desc.value(QStringLiteral("liveGrading")).toBool());
        QCOMPARE(desc.value(QStringLiteral("frame")).toInt(), 0);
        QVariantMap g{{QStringLiteral("target"), QStringLiteral("par-level")}, {QStringLiteral("gesture"), QStringLiteral("k1")}};
        m_mock->setPlaying(true);
        raw.control(contract::kParamNudge, 2, g, 1);
        QTRY_COMPARE(RawClient::code(raw.ackFor(1).value(QStringLiteral("outcome")).toMap()), contract::err::Busy);
        m_mock->setPlaying(false);
        raw.control(contract::kParamNudge, 2, g, 2);  // key at frame 0
        QTRY_VERIFY(raw.ackFor(2).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->state()[QStringLiteral("keys")].toMap()[QStringLiteral("level")].toMap()[QStringLiteral("0")].toDouble(), 52.0);
        m_mock->setPosition(50);  // seeking ends the multi-key gesture
        QCOMPARE(m_mock->history().size(), 1);
        raw.control(contract::kParamNudge, 1, g, 3);  // no key at 50, no create
        QTRY_COMPARE(RawClient::code(raw.ackFor(3).value(QStringLiteral("outcome")).toMap()), contract::err::KeyframeRequired);
        QVariantMap create = g;
        create.insert(QStringLiteral("gesture"), QStringLiteral("k2"));
        create.insert(QStringLiteral("keyframe"), QStringLiteral("create"));
        raw.control(contract::kParamNudge, 3, create, 4);
        QTRY_VERIFY(raw.ackFor(4).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->state()[QStringLiteral("keys")].toMap()[QStringLiteral("level")].toMap()[QStringLiteral("50")].toDouble(), 53.0);
        create.insert(QStringLiteral("phase"), QStringLiteral("cancel"));
        raw.control(contract::kParamNudge, 0, create, 5);  // still owned: the created key goes too
        QTRY_VERIFY(raw.ackFor(5).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVERIFY(!m_mock->state()[QStringLiteral("keys")].toMap()[QStringLiteral("level")].toMap().contains(QStringLiteral("50")));
        QCOMPARE(m_mock->history().size(), 1);
        raw.control(contract::kParamNudge, 0, create, 6);  // nothing live to cancel
        QTRY_COMPARE(RawClient::code(raw.ackFor(6).value(QStringLiteral("outcome")).toMap()), contract::err::HistoryConflict);
        // An oversized gesture id is a resource limit, an empty one malformed.
        QVariantMap big = g;
        big.insert(QStringLiteral("gesture"), QString(129, QLatin1Char('g')));
        raw.control(contract::kParamNudge, 1, big, 7);
        QTRY_COMPARE(RawClient::code(raw.ackFor(7).value(QStringLiteral("outcome")).toMap()), contract::err::ResourceLimit);
        big.insert(QStringLiteral("gesture"), QString());
        raw.control(contract::kParamNudge, 1, big, 8);
        QTRY_COMPARE(RawClient::code(raw.ackFor(8).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
    }

    // MR3 handles as the contract places them: track (and its mixer gain),
    // unique static clip gain, and the declared resize-only trim scope.
    void mr3TimelineHandles()
    {
        RawClient raw(connectClient(), QString());
        const QVariantMap caps = raw.call(QStringLiteral("Capabilities")).value(QStringLiteral("result")).toMap();
        QCOMPARE(caps.value(QStringLiteral("limits")).toMap().value(QStringLiteral("editingWriters")).toInt(), 1);
        QCOMPARE(caps.value(QStringLiteral("limits")).toMap().value(QStringLiteral("trimGestureSteps")).toInt(), 128);
        raw.subscribe();
        auto timeline = [&raw] { return raw.context.value(QStringLiteral("timeline")).toMap(); };
        auto refresh = [&raw] { raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap(); };
        QVariantMap track = timeline().value(QStringLiteral("track")).toMap();
        QCOMPARE(track.value(QStringLiteral("target")).toString(), QStringLiteral("trk-7"));
        QCOMPARE(track.value(QStringLiteral("id")).toInt(), 7);
        QCOMPARE(track.value(QStringLiteral("gain")).toMap().value(QStringLiteral("target")).toString(), QStringLiteral("trk-7"));  // same handle
        QVERIFY(!timeline().contains(QStringLiteral("clipGain")));
        QVERIFY(!timeline().contains(QStringLiteral("trim")));
        // track.set: explicit handle, typed value, applicability by track type.
        QVariantMap set = raw.common();
        set.insert(QStringLiteral("target"), QStringLiteral("trk-7"));
        set.insert(QStringLiteral("what"), QStringLiteral("mute"));
        set.insert(QStringLiteral("value"), true);
        QVERIFY(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)}).value(QStringLiteral("result")).toMap().value(QStringLiteral("changed")).toBool());
        QTRY_VERIFY(timeline().value(QStringLiteral("track")).toMap().value(QStringLiteral("muted")).toBool());
        set.insert(QStringLiteral("value"), QStringLiteral("yes"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::InvalidArguments);
        set.insert(QStringLiteral("value"), true);
        set.insert(QStringLiteral("what"), QStringLiteral("hide"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::UnsupportedMode);
        set.insert(QStringLiteral("what"), QStringLiteral("lock"));
        set.insert(QStringLiteral("soloMode"), QStringLiteral("exclusive"));  // soloMode only with solo
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::InvalidArguments);
        set.insert(QStringLiteral("what"), QStringLiteral("solo"));
        set.insert(QStringLiteral("soloMode"), QStringLiteral("loud"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::UnsupportedMode);
        set.insert(QStringLiteral("what"), QStringLiteral("blink"));
        set.remove(QStringLiteral("soloMode"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::UnsupportedMode);
        set.insert(QStringLiteral("target"), QStringLiteral("trk-404"));  // never issued
        set.insert(QStringLiteral("what"), QStringLiteral("lock"));
        QCOMPARE(RawClient::code(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)})), contract::err::TargetNotFound);
        // Exclusive solo clears other solos and keeps manual mute states.
        m_mock->focusTrack(QStringLiteral("trk-8"));
        refresh();
        set = raw.common();
        set.insert(QStringLiteral("target"), QStringLiteral("trk-8"));
        set.insert(QStringLiteral("what"), QStringLiteral("solo"));
        set.insert(QStringLiteral("value"), true);
        QVERIFY(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)}).value(QStringLiteral("ok")).toBool());
        m_mock->focusTrack(QStringLiteral("trk-7"));
        refresh();
        set = raw.common();
        set.insert(QStringLiteral("target"), QStringLiteral("trk-7"));
        set.insert(QStringLiteral("what"), QStringLiteral("solo"));
        set.insert(QStringLiteral("value"), true);
        set.insert(QStringLiteral("soloMode"), QStringLiteral("exclusive"));
        QVERIFY(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)}).value(QStringLiteral("ok")).toBool());
        const QVariantMap tracks = m_mock->state()[QStringLiteral("tracks")].toMap();
        QVERIFY(tracks[QStringLiteral("trk-7")].toMap()[QStringLiteral("solo")].toBool());
        QVERIFY(!tracks[QStringLiteral("trk-8")].toMap()[QStringLiteral("solo")].toBool());
        QVERIFY(tracks[QStringLiteral("trk-7")].toMap()[QStringLiteral("mute")].toBool());  // manual mute retained
        // Track gain through its handle: 0.1 dB per step.
        QVariantMap gain{{QStringLiteral("target"), QStringLiteral("trk-7")}, {QStringLiteral("gesture"), QStringLiteral("tg")}};
        quint64 seq = 0;
        raw.control(contract::kAudioGain, 15, gain, ++seq);
        QTRY_VERIFY(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->state()[QStringLiteral("tracks")].toMap()[QStringLiteral("trk-7")].toMap()[QStringLiteral("gainDb")].toDouble(), 1.5);
        gain.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kAudioGain, 0, gain, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        // Selecting the linked A/V pair publishes its clip gain and trim scope.
        m_mock->selectClip(QStringLiteral("clip-22"));
        refresh();
        const QVariantMap clipGain = timeline().value(QStringLiteral("clipGain")).toMap();
        QCOMPARE(clipGain.value(QStringLiteral("clip")).toInt(), 22);  // native integer ids
        QCOMPARE(clipGain.value(QStringLiteral("unit")).toString(), QStringLiteral("dB"));
        QCOMPARE(clipGain.value(QStringLiteral("policy")).toString(), QStringLiteral("single_keyframe_existing_effect"));
        const QVariantMap trim = timeline().value(QStringLiteral("trim")).toMap();
        QCOMPARE(trim.value(QStringLiteral("clip")).toInt(), 22);
        QCOMPARE(trim.value(QStringLiteral("clips")).toList(), (QVariantList{21, 22}));
        QCOMPARE(trim.value(QStringLiteral("tracks")).toList(), (QVariantList{3, 7}));
        QCOMPARE(trim.value(QStringLiteral("modes")).toStringList(), QStringList{QStringLiteral("resize")});
        QCOMPARE(timeline().value(QStringLiteral("selection")).toMap().value(QStringLiteral("count")).toInt(), 1);
        QVariantMap cg{{QStringLiteral("target"), clipGain.value(QStringLiteral("target"))}, {QStringLiteral("gesture"), QStringLiteral("cg")}};
        raw.control(contract::kAudioGain, -30, cg, ++seq);
        QTRY_VERIFY(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->state()[QStringLiteral("clips")].toMap()[QStringLiteral("clip-22")].toMap()[QStringLiteral("volumeDb")].toDouble(), -3.0);
        QCOMPARE(m_mock->state()[QStringLiteral("tracks")].toMap()[QStringLiteral("trk-7")].toMap()[QStringLiteral("gainDb")].toDouble(), 1.5);
        // One editing writer across controls: another caller's trim is busy.
        RawClient other(connectClient(), QString());
        other.subscribe();
        QVariantMap tr{{QStringLiteral("target"), trim.value(QStringLiteral("target"))}, {QStringLiteral("gesture"), QStringLiteral("t1")},
                       {QStringLiteral("edge"), QStringLiteral("end")}, {QStringLiteral("mode"), QStringLiteral("resize")}};
        other.control(contract::kTrim, 5, tr, 1);
        QTRY_COMPARE(RawClient::code(other.ackFor(1).value(QStringLiteral("outcome")).toMap()), contract::err::Busy);
        cg.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kAudioGain, 0, cg, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        QCOMPARE(m_mock->history().size(), 5);  // mute, two solos, track gain, clip gain
        // Resize only: both clips of the pair, bounded, nothing downstream.
        raw.control(contract::kTrim, 20, tr, ++seq);
        QTRY_VERIFY(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QVariantMap startEdge = tr;
        startEdge.insert(QStringLiteral("gesture"), QStringLiteral("t2"));
        startEdge.insert(QStringLiteral("edge"), QStringLiteral("start"));
        raw.control(contract::kTrim, -80, startEdge, ++seq);  // minStart is 50
        QTRY_VERIFY(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 6);  // the first trim gesture ended when the second started
        const QVariantMap clips = m_mock->state()[QStringLiteral("clips")].toMap();
        for (const char *id : {"clip-21", "clip-22"}) {
            QCOMPARE(clips[QString::fromLatin1(id)].toMap()[QStringLiteral("end")].toInt(), 220);
            QCOMPARE(clips[QString::fromLatin1(id)].toMap()[QStringLiteral("start")].toInt(), 50);
        }
        QCOMPARE(clips[QStringLiteral("clip-31")].toMap()[QStringLiteral("start")].toInt(), 300);
        QVariantMap bad = tr;
        bad.insert(QStringLiteral("gesture"), QStringLiteral("t3"));
        bad.insert(QStringLiteral("mode"), QStringLiteral("ripple"));
        raw.control(contract::kTrim, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedMode);
        bad.remove(QStringLiteral("mode"));  // a missing mode is not the advertised one (as Kdenlive)
        raw.control(contract::kTrim, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedMode);
        bad.insert(QStringLiteral("mode"), QStringLiteral("resize"));
        bad.remove(QStringLiteral("edge"));
        raw.control(contract::kTrim, 1, bad, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::InvalidArguments);
        // A locked track in the declared scope refuses the trim.
        startEdge.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kTrim, 0, startEdge, ++seq);
        QTRY_VERIFY(!raw.ackFor(seq).isEmpty());
        m_mock->focusTrack(QStringLiteral("trk-3"));
        refresh();
        set = raw.common();
        set.insert(QStringLiteral("target"), QStringLiteral("trk-3"));
        set.insert(QStringLiteral("what"), QStringLiteral("lock"));
        set.insert(QStringLiteral("value"), true);
        QVERIFY(raw.call(QStringLiteral("Invoke"), {contract::kCmdTrackSet, QVariant::fromValue(set)}).value(QStringLiteral("ok")).toBool());
        QVERIFY(!timeline().value(QStringLiteral("track")).toMap().contains(QStringLiteral("gain")));  // a video track has no mixer gain
        tr.insert(QStringLiteral("gesture"), QStringLiteral("t4"));
        raw.control(contract::kTrim, 1, tr, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::TrackLocked);
        // A clip whose volume is animated keeps its handle, but gain refuses.
        m_mock->selectClip(QStringLiteral("clip-31"));
        refresh();
        QCOMPARE(timeline().value(QStringLiteral("trim")).toMap().value(QStringLiteral("clips")).toList(), QVariantList{31});
        raw.control(contract::kAudioGain, 1, {{QStringLiteral("target"), timeline().value(QStringLiteral("clipGain")).toMap().value(QStringLiteral("target"))},
                                              {QStringLiteral("gesture"), QStringLiteral("anim")}}, ++seq);
        QTRY_COMPARE(RawClient::code(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap()), contract::err::UnsupportedParameter);
        // Track navigation changes the handle and the epoch.
        const quint64 epoch = raw.epoch();
        raw.control(contract::kTrackFocus, 1, {}, ++seq);
        QTRY_COMPARE(timeline().value(QStringLiteral("track")).toMap().value(QStringLiteral("target")).toString(), QStringLiteral("trk-7"));
        QVERIFY(raw.epoch() > epoch);
    }

    // limits.trimGestureSteps bounds the retained resize steps of one gesture.
    void trimGestureStepLimit()
    {
        m_mock->setTrimGestureSteps(3);
        m_mock->selectClip(QStringLiteral("clip-22"));
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        const QVariantMap tr{{QStringLiteral("target"), QStringLiteral("trim-clip-22")}, {QStringLiteral("gesture"), QStringLiteral("long")},
                             {QStringLiteral("edge"), QStringLiteral("end")}, {QStringLiteral("mode"), QStringLiteral("resize")}};
        for (quint64 seq = 1; seq <= 3; ++seq) {
            raw.control(contract::kTrim, 1, tr, seq);
            QTRY_VERIFY(raw.ackFor(seq).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        }
        raw.control(contract::kTrim, 1, tr, 4);
        QTRY_COMPARE(RawClient::code(raw.ackFor(4).value(QStringLiteral("outcome")).toMap()), contract::err::ResourceLimit);
        QVariantMap end = tr;
        end.insert(QStringLiteral("phase"), QStringLiteral("end"));
        raw.control(contract::kTrim, 0, end, 5);
        QTRY_VERIFY(raw.ackFor(5).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(m_mock->history().size(), 1);
        QCOMPARE(m_mock->state()[QStringLiteral("clips")].toMap()[QStringLiteral("clip-22")].toMap()[QStringLiteral("end")].toInt(), 203);
    }

    // Engine + client + mock: the three knobs edit the three wheels of the
    // focused widget; another caller's gesture is a domain refusal (never keys).
    void daemonThreeWheelKnobs()
    {
        m_mock->focusWheels(QStringLiteral("gamma"));
        auto conn = connectClient();
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_VERIFY(client.isAvailable());
        QTRY_COMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob3"), 1))->layer, QStringLiteral("color-wheels"));
        for (int i = 0; i < 4; ++i) {
            e.handle(PadEvent{QStringLiteral("knob1"), PadEvent::Turn, 1, 0});
        }
        QTRY_VERIFY(qAbs(wheelValue(m_mock, "lift", "g") - 0.04) < 1e-9);
        for (int i = 0; i < 3; ++i) {
            e.handle(PadEvent{QStringLiteral("knob2"), PadEvent::Turn, -1, 0});
        }
        QTRY_VERIFY(qAbs(wheelValue(m_mock, "gamma", "b") - 0.97) < 1e-9);
        e.handle(PadEvent{QStringLiteral("knob3"), PadEvent::Turn, 1, 0});
        QTRY_VERIFY(qAbs(wheelValue(m_mock, "gain", "r") - 1.01) < 1e-9);
        QTRY_COMPARE(m_mock->history().size(), 2);  // lift and gamma committed; gain still open
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->history().size(), 3, 2000);
        QCOMPARE(m_mock->context().value(QStringLiteral("colorWheel")).toMap().value(QStringLiteral("wheel")).toString(), QStringLiteral("gamma"));  // focus never moved
        // Another caller holds the editing gesture: busy, reported, no keys.
        RawClient other(connectClient(), QString());
        other.subscribe();
        other.control(contract::kColorWheel, 1, {{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("o")}}, 1);
        QTRY_VERIFY(!other.ackFor(1).isEmpty());
        QSignalSpy refused(&client, &KdenliveClient::refused);
        e.handle(PadEvent{QStringLiteral("knob3"), PadEvent::Turn, 1, 0});
        QTRY_COMPARE(refused.size(), 1);
        QCOMPARE(refused[0][1].toString(), contract::err::Busy);
        QVERIFY(keys.taps.isEmpty());
        QCOMPARE(client.inFlightMessages(), 0);
    }

    // Engine + client + mock, MR3: the timeline page (key13) drives track
    // focus, toggles from the published state, resize trim and both gains.
    void daemonTimelinePage()
    {
        auto conn = connectClient();
        KdenliveDBusClient client(conn);
        client.setServiceOverride(QString());
        RecordingKeySink keys;
        Engine e(&keys, &client);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_VERIFY(client.isAvailable());
        auto pad = [&e](const QString &control, PadEvent::Type type, int delta = 0) { e.handle(PadEvent{control, type, delta, 0}); };
        auto tracks = [this] { return m_mock->state()[QStringLiteral("tracks")].toMap(); };
        auto clips = [this] { return m_mock->state()[QStringLiteral("clips")].toMap(); };
        pad(QStringLiteral("key13"), PadEvent::KeyDown);
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob2"), 1))->layer, QStringLiteral("timeline-page"));
        // Toggle mute twice: the second press reads the state Kdenlive published.
        pad(QStringLiteral("key1"), PadEvent::KeyDown);
        QTRY_VERIFY(tracks()[QStringLiteral("trk-7")].toMap()[QStringLiteral("mute")].toBool());
        QTRY_VERIFY(client.context().value(QStringLiteral("timeline")).toMap().value(QStringLiteral("track")).toMap().value(QStringLiteral("muted")).toBool());
        pad(QStringLiteral("key1"), PadEvent::KeyDown);
        QTRY_VERIFY(!tracks()[QStringLiteral("trk-7")].toMap()[QStringLiteral("mute")].toBool());
        // Track mixer gain on knob3 (no clip selected), one undo entry.
        for (int i = 0; i < 5; ++i) {
            pad(QStringLiteral("knob3"), PadEvent::Turn, 1);
        }
        QTRY_COMPARE(tracks()[QStringLiteral("trk-7")].toMap()[QStringLiteral("gainDb")].toDouble(), 0.5);
        // Select the linked pair: knob3 becomes its clip gain, knob2 resizes it.
        m_mock->selectClip(QStringLiteral("clip-22"));
        QTRY_COMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob3"), 1))->layer, QStringLiteral("timeline-clip-gain"));
        for (int i = 0; i < 4; ++i) {
            pad(QStringLiteral("knob3"), PadEvent::Turn, -1);
        }
        QTRY_COMPARE(clips()[QStringLiteral("clip-22")].toMap()[QStringLiteral("volumeDb")].toDouble(), -0.4);
        for (int i = 0; i < 6; ++i) {
            pad(QStringLiteral("knob2"), PadEvent::Turn, 1);
        }
        QTRY_COMPARE(clips()[QStringLiteral("clip-21")].toMap()[QStringLiteral("end")].toInt(), 206);
        QCOMPARE(clips()[QStringLiteral("clip-22")].toMap()[QStringLiteral("end")].toInt(), 206);
        pad(QStringLiteral("knob2"), PadEvent::PressDown);  // start edge
        pad(QStringLiteral("knob2"), PadEvent::Turn, 1);
        QTRY_COMPARE(clips()[QStringLiteral("clip-22")].toMap()[QStringLiteral("start")].toInt(), 101);
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->history().size(), 6, 2000);  // mute, unmute, track gain, clip gain, two trims
        // Track focus moves the handle (new epoch); knob3 follows it.
        m_mock->selectClip(QString());
        QTRY_COMPARE(client.epoch(), m_mock->context().value(QStringLiteral("epoch")).toULongLong());  // the client knows the new epoch
        pad(QStringLiteral("knob1"), PadEvent::Turn, 1);
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("track")).toString(), QStringLiteral("trk-8"));
        QTRY_COMPARE(client.context().value(QStringLiteral("timeline")).toMap().value(QStringLiteral("track")).toMap().value(QStringLiteral("target")).toString(), QStringLiteral("trk-8"));
        pad(QStringLiteral("knob3"), PadEvent::Turn, 1);
        QTRY_COMPARE(tracks()[QStringLiteral("trk-8")].toMap()[QStringLiteral("gainDb")].toDouble(), 0.1);
        // Another caller's editing gesture: busy is reported, nothing is typed.
        QTRY_COMPARE_WITH_TIMEOUT(e.activeGestures(), 0, 2000);
        QTRY_COMPARE_WITH_TIMEOUT(m_mock->history().size(), 7, 2000);
        RawClient other(connectClient(), QString());
        other.subscribe();
        other.control(contract::kAudioGain, 1, {{QStringLiteral("target"), QStringLiteral("trk-8")}, {QStringLiteral("gesture"), QStringLiteral("o")}}, 1);
        QTRY_VERIFY(!other.ackFor(1).isEmpty());
        QSignalSpy refused(&client, &KdenliveClient::refused);
        pad(QStringLiteral("knob3"), PadEvent::Turn, 1);
        QTRY_COMPARE(refused.size(), 1);
        QCOMPARE(refused[0][1].toString(), contract::err::Busy);
        QVERIFY(keys.taps.isEmpty());
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
        m_mock->focusWheels(QStringLiteral("lift"));
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
        QSignalSpy notices(&e, &Engine::notice);
        e.setConfig(m_cfg);
        e.setActiveWindow(WindowInfo{QStringLiteral("org.kde.kdenlive"), QStringLiteral("t"), 99, QStringLiteral("0x1")});
        QTRY_COMPARE(client.state(), State::Absent);
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(notices.size(), 1);  // API-only by default: a notice, no keys
        QTest::qWait(30);
        QVERIFY(keys.taps.isEmpty());
        // The explicit per-profile opt-in types the stock shortcut.
        Config c = m_cfg;
        for (auto &p : c.profiles) {
            p.keyFallback = p.kdenlive;
        }
        e.setConfig(c);
        e.handle(PadEvent{QStringLiteral("key1"), PadEvent::KeyDown, 0, 0});
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("I")});  // stock shortcut
    }

    // K23-MR1a (k23-contract-mr1a-actions.md): the curated candidates, context
    // restrictions, editing-action busy, ActionsChanged on enabled/checked,
    // Slip preview and pause, mark-out's exclusive end, and the client refresh.
    void mr1aActionsInventoryAndRestrictions()
    {
        RawClient raw(connectClient(), QString());
        raw.subscribe();
        QVariantMap actions = raw.actionMap();
        QCOMPARE(actions.size(), 71);
        QCOMPARE(QStringList(actions.keys()), [] { QStringList c = MockKdenlive::candidateActions(); c.sort(); return c; }());
        for (const auto &a : std::as_const(actions)) {
            QCOMPARE(QStringList(a.toMap().keys()), (QStringList{QStringLiteral("checkable"), QStringLiteral("checked"), QStringLiteral("enabled"),
                                                                  QStringLiteral("id"), QStringLiteral("shortcut"), QStringLiteral("text")}));
        }
        QVERIFY(!actions.contains(QStringLiteral("roll_tool")) && !actions.contains(QStringLiteral("slide_tool")));  // not fabricated
        // Every action the shipped config binds is a candidate.
        QString err;
        auto def = loadConfig(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"), &err);
        QVERIFY2(def, qPrintable(err));
        QStringList bound;
        for (const Profile &p : def->profiles) {
            auto collect = [&](const QHash<QString, Binding> &bs) {
                for (const Binding &b : bs) {
                    if (b.kind == Binding::Action && !bound.contains(b.name)) {
                        bound << b.name;
                    }
                }
            };
            collect(p.bindings);
            for (const Layer &l : p.layers) {
                collect(l.bindings);
            }
        }
        QVERIFY(bound.size() >= 20);
        for (const QString &id : std::as_const(bound)) {
            QVERIFY2(actions.contains(id), qPrintable(id));
        }
        // Dialog actions are absent: unknown_action, never a fallback invitation.
        for (const char *id : {"file_save", "project_render", "insert_space", "edit_marker"}) {
            QCOMPARE(RawClient::code(raw.call(QStringLiteral("TriggerAction"), {QString::fromLatin1(id), QVariant::fromValue(raw.common())})), contract::err::UnknownAction);
        }
        auto trigger = [&](const QString &id) { return raw.call(QStringLiteral("TriggerAction"), {id, QVariant::fromValue(raw.common())}); };
        auto refresh = [&] { raw.context = raw.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap(); };
        auto enabled = [&](const char *id) { return raw.actionMap().value(QString::fromLatin1(id)).toMap().value(QStringLiteral("enabled")).toBool(); };
        // Undo availability is not permanent, and its change is announced.
        QVERIFY(!enabled("edit_undo"));
        int announced = raw.actionsChanged;
        QCOMPARE(RawClient::code(trigger(QStringLiteral("edit_undo"))), contract::err::ActionDisabled);
        // delete needs focus inside the timeline; no silent redirection.
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("effectStack"));
        QTRY_VERIFY(raw.actionsChanged > announced);
        refresh();
        QVERIFY(!enabled("delete_timeline_clip"));
        QCOMPARE(RawClient::code(trigger(QStringLiteral("delete_timeline_clip"))), contract::err::TargetNotFound);
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("timeline"));
        refresh();
        QTRY_VERIFY(enabled("delete_timeline_clip"));
        const int historyBefore = int(m_mock->history().size());
        announced = raw.actionsChanged;
        QCOMPARE(trigger(QStringLiteral("delete_timeline_clip")).value(QStringLiteral("result")).toMap().value(QStringLiteral("state")).toString(), QStringLiteral("accepted"));
        QTRY_COMPARE(raw.finished.size(), 1);
        QCOMPARE(int(m_mock->history().size()), historyBefore + 1);  // one native undo entry
        QTRY_VERIFY(raw.actionsChanged > announced);                  // undo became available
        QVERIFY(enabled("edit_undo"));
        refresh();
        // Source insertion needs a clip-monitor source and a timeline target.
        m_mock->setSourceOpen(false);
        QTRY_VERIFY(!enabled("insert_to_in_point"));
        QCOMPARE(RawClient::code(trigger(QStringLiteral("overwrite_to_in_point"))), contract::err::TargetNotFound);
        m_mock->setSourceOpen(true);
        // Editing actions: another caller's editing gesture or a native drag -> busy.
        RawClient other(connectClient(), QString());
        other.subscribe();
        m_mock->focusWheels(QStringLiteral("lift"));
        refresh();
        other.context = other.call(QStringLiteral("GetContext")).value(QStringLiteral("result")).toMap();
        other.control(contract::kColorWheel, 2, {{QStringLiteral("target"), cw("lift")}, {QStringLiteral("gesture"), QStringLiteral("g-other")}, {QStringLiteral("axis"), QStringLiteral("value")}}, 1);
        QTRY_VERIFY(!other.ackFor(1).isEmpty());
        QVERIFY(other.ackFor(1).value(QStringLiteral("outcome")).toMap().value(QStringLiteral("ok")).toBool());
        QCOMPARE(RawClient::code(trigger(QStringLiteral("cut_timeline_clip"))), contract::err::Busy);
        const QVariantMap view = trigger(QStringLiteral("zoom_fit"));  // not an editing action
        QVERIFY2(view.value(QStringLiteral("ok")).toBool(), qPrintable(RawClient::code(view)));
        QTRY_COMPARE(raw.finished.size(), 2);
        refresh();
        m_mock->setDragging(true);
        QCOMPARE(RawClient::code(trigger(QStringLiteral("cut_timeline_clip"))), contract::err::Busy);
        m_mock->setDragging(false);
        // Mark out keeps Kdenlive's exclusive end: at frame 1040 the zone ends at 1041.
        m_mock->setPosition(1040);
        QVERIFY(trigger(QStringLiteral("mark_out")).value(QStringLiteral("ok")).toBool());
        QTRY_COMPARE(m_mock->state().value(QStringLiteral("zone")).toMap().value(QStringLiteral("out")).toInt(), 1041);
        // Slip preview: playback start is busy and leaves no fake playing state;
        // pause still works; select_tool + fresh epoch allows playback again.
        const int finishedBefore = int(raw.finished.size());
        QVERIFY(trigger(QStringLiteral("slip_tool")).value(QStringLiteral("ok")).toBool());
        QTRY_COMPARE(m_mock->context().value(QStringLiteral("tool")).toString(), QStringLiteral("slip"));
        QTRY_VERIFY(raw.actionMap().value(QStringLiteral("slip_tool")).toMap().value(QStringLiteral("checked")).toBool());
        refresh();
        QCOMPARE(RawClient::code(trigger(QStringLiteral("monitor_play"))), contract::err::Busy);
        QVERIFY(!m_mock->context().value(QStringLiteral("playing")).toBool());
        QVERIFY(trigger(QStringLiteral("monitor_pause")).value(QStringLiteral("ok")).toBool());
        QVERIFY(trigger(QStringLiteral("select_tool")).value(QStringLiteral("ok")).toBool());
        QTRY_COMPARE(m_mock->context().value(QStringLiteral("tool")).toString(), QStringLiteral("select"));
        QTRY_COMPARE(int(raw.finished.size()), finishedBefore + 3);
        refresh();
        QVERIFY(trigger(QStringLiteral("monitor_play")).value(QStringLiteral("ok")).toBool());
        QTRY_VERIFY(m_mock->context().value(QStringLiteral("playing")).toBool());

        // The daemon's client refreshes its view on ActionsChanged, one refresh in flight.
        m_mock->focusWheels(QString());
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("timeline"));
        KdenliveDBusClient client(connectClient());
        client.setServiceOverride(QString());
        client.attachToPid(1);
        QTRY_COMPARE(client.state(), State::Available);
        QVERIFY(client.supportsAction(QStringLiteral("extract_clip")));
        QVERIFY(client.actionEnabled(QStringLiteral("delete_timeline_clip")));
        m_mock->setContextValue(QStringLiteral("focus"), QStringLiteral("clipMonitor"));
        QTRY_VERIFY(!client.actionEnabled(QStringLiteral("delete_timeline_clip")));
        QTest::qWait(50);  // settle the focus change's refresh
        const int refreshes = client.actionRefreshes();
        m_mock->setSourceOpen(false);  // state the burst's refreshes must pick up
        for (int i = 0; i < 5; ++i) {
            m_mock->announceActionsChanged();  // five signals queued before the client reads any
        }
        QTRY_VERIFY(!client.actionEnabled(QStringLiteral("insert_to_in_point")));
        QTest::qWait(100);
        // The first signal starts a refresh; the other four arrive while it is in
        // flight and collapse into one more (the mock's own coalesced signal for
        // the source change may add one).
        const int burst = client.actionRefreshes() - refreshes;
        QVERIFY2(burst >= 2 && burst <= 3, qPrintable(QString::number(burst)));
        client.attachToPid(0);
    }

    // list-capabilities: read-only queries (no lease), related to the config.
    void listCapabilitiesAgainstMock()
    {
        QString err;
        auto def = loadConfig(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"), &err);
        QVERIFY2(def, qPrintable(err));
        m_mock->setStage(2);
        m_mock->focusWheels(QStringLiteral("gamma"));
        const CapabilityReport r = queryKdenlive(connectClient(), QString(), 3000);
        QCOMPARE(int(r.status), int(CapabilityReport::Status::Available));
        QCOMPARE(m_mock->leaseCount(), 0);  // never subscribed
        QVERIFY(r.capabilities.value(QStringLiteral("controls")).toStringList().contains(contract::kColorWheel));
        QVERIFY(!r.actions.isEmpty());
        QCOMPARE(r.context.value(QStringLiteral("colorWheels")).toList().size(), 3);
        const ConfigFindings f = checkAgainstConfig(r, *def);
        QVERIFY2(f.layersNow.contains(QStringLiteral("kdenlive/color-wheels")), qPrintable(f.layersNow.join(QLatin1Char(' '))));
        // Stage 2 has no MR3 track/trim controls: the config's track and trim pages say so.
        QVERIFY2(f.notOffered.contains(QStringLiteral("control timeline.track (profile kdenlive knob3.turn)")), qPrintable(f.notOffered.join(QLatin1Char('\n'))));
        QVERIFY(f.notOffered.contains(QStringLiteral("control edit.trim (profile kdenlive layer trim knob2.turn)")));
        QVERIFY(f.notOffered.contains(QStringLiteral("command track.set (profile kdenlive layer track-mixer key1)")));
        QVERIFY(!f.notOffered.join(QLatin1Char(' ')).contains(QStringLiteral("colorwheel.nudge")));
        const QString text = formatReport(r, &*def);
        QVERIFY(text.contains(QStringLiteral("colorWheels[3]")));
        QVERIFY(text.contains(QStringLiteral("bound but not offered")));
        const QJsonObject j = reportJson(r, &*def);
        QCOMPARE(j.value(QStringLiteral("status")).toString(), QStringLiteral("available"));
        QVERIFY(j.value(QStringLiteral("contextPaths")).toObject().contains(QStringLiteral("colorWheel.target")));
        // A mode selects layers: the trim page applies once chosen.
        const ConfigFindings trim = checkAgainstConfig(r, *def, {{QStringLiteral("page"), QStringLiteral("trim")}});
        QVERIFY(trim.layersNow.contains(QStringLiteral("kdenlive/trim")));
        QVERIFY(flattenContext({{QStringLiteral("a"), QVariantMap{{QStringLiteral("b"), 1}}}, {QStringLiteral("l"), QVariantList{1, 2}}})
                == (QVariantMap{{QStringLiteral("a.b"), 1}, {QStringLiteral("l"), QStringLiteral("[2 items]")}}));
    }

    void listCapabilitiesInterfaceOffOrOld()
    {
        m_register = false;  // Kdenlive with the interface off (its default)
        const CapabilityReport off = queryKdenlive(connectClient(), QString(), 3000);
        QCOMPARE(int(off.status), int(CapabilityReport::Status::Absent));
        QVERIFY(off.detail.contains(QStringLiteral("UnknownObject")));
        QVERIFY(formatReport(off, nullptr).contains(QStringLiteral("control interface not enabled")));
        QCOMPARE(reportJson(off, nullptr).value(QStringLiteral("status")).toString(), QStringLiteral("absent"));
        m_register = true;
        m_old = new QObject;
        m_oldAdaptor = new OldAdaptor(m_old);
        const CapabilityReport old = queryKdenlive(connectClient(), QString(), 3000);
        QCOMPARE(int(old.status), int(CapabilityReport::Status::Incompatible));
        QVERIFY(old.detail.contains(QStringLiteral("revision 1")));
        QCOMPARE(m_oldAdaptor->subscribes, 0);
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
