// SPDX-License-Identifier: GPL-2.0-or-later
#include "engine.h"
#include "kdenlivecontract.h"

#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;
using State = KdenliveClient::State;

namespace {
PadEvent key(int n)
{
    return PadEvent{QStringLiteral("key%1").arg(n), PadEvent::KeyDown, 0, 0};
}
PadEvent turn(int knob, int delta)
{
    return PadEvent{QStringLiteral("knob%1").arg(knob), PadEvent::Turn, delta, 0};
}
PadEvent press(int knob)
{
    return PadEvent{QStringLiteral("knob%1").arg(knob), PadEvent::PressDown, 0, 0};
}
const WindowInfo kKdenlive{QStringLiteral("org.kde.kdenlive"), QStringLiteral("Untitled - Kdenlive"), 4242, QStringLiteral("0x1")};
const WindowInfo kFirefox{QStringLiteral("firefox"), QStringLiteral("x"), 77, QStringLiteral("0x2")};

QVariantMap wheelDescriptor(const QString &prefix, const QString &wheel)
{
    return {{QStringLiteral("target"), prefix + QLatin1Char('-') + wheel},
            {QStringLiteral("wheel"), wheel},
            {QStringLiteral("axes"), QStringList{QStringLiteral("value"), QStringLiteral("r"), QStringLiteral("g"), QStringLiteral("b")}}};
}
// MR2 shape: the focused wheel plus the three per-wheel handles of its widget.
QVariantMap wheelContext(quint64 epoch, const QString &prefix = QStringLiteral("cw"), const QString &focused = QStringLiteral("lift"))
{
    return {{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(epoch)},
            {QStringLiteral("focus"), QStringLiteral("effectStack")},
            {QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}, {QStringLiteral("ownerId"), 12}, {QStringLiteral("sequence"), QStringLiteral("s")}}},
            {QStringLiteral("colorWheel"), wheelDescriptor(prefix, focused)},
            {QStringLiteral("colorWheels"), QVariantList{wheelDescriptor(prefix, QStringLiteral("lift")), wheelDescriptor(prefix, QStringLiteral("gamma")),
                                                          wheelDescriptor(prefix, QStringLiteral("gain"))}}};
}
const QStringList kStage1{QStringLiteral("playhead.jog"), QStringLiteral("playhead.shuttle"), QStringLiteral("timeline.zoom")};
} // namespace

class TestEngine : public QObject
{
    Q_OBJECT
    Config m_cfg;

private Q_SLOTS:
    void initTestCase()
    {
        QTemporaryDir home;
        qputenv("HOME", home.path().toLocal8Bit());
        QString err;
        auto c = loadConfig(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"), &err);
        QVERIFY2(c, qPrintable(err));
        m_cfg = *c;
        m_cfg.settings.accelFactor = 1.0;  // deterministic detent counts
    }

    void globalProfileForOtherApps()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kFirefox);
        QCOMPARE(e.activeProfile()->name, QStringLiteral("global"));
        e.handle(turn(1, 1));
        e.handle(turn(1, -1));
        e.handle(press(1));
        e.handle(key(12));
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("VOLUMEUP"), QStringLiteral("VOLUMEDOWN"), QStringLiteral("MUTE"), QStringLiteral("PLAYPAUSE")}));
        QVERIFY(kd.calls.isEmpty());
        QCOMPARE(kd.attachedPid(), 0);
    }

    void kdenliveActionsAndCoalescedJog()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        QCOMPARE(kd.attachedPid(), 4242);
        e.handle(key(1));
        QCOMPARE(kd.calls.value(0), QStringLiteral("action mark_in"));
        for (int i = 0; i < 40; ++i) {
            e.handle(turn(1, 1));
        }
        QCOMPARE(kd.controlDeltas.size(), 1);  // one in flight, 39 merged
        kd.ackAll();
        QTRY_COMPARE(kd.controlDeltas.size(), 2);
        QCOMPARE(kd.controlDeltas.value(1), 39.0);
        // Transport controls carry no gesture/target.
        QVERIFY(!kd.controlOptions.value(0).contains(QStringLiteral("gesture")));
        QVERIFY(keys.taps.isEmpty());
    }

    // Item 2: acks release only their exact key.
    void ackReleasesOnlyItsOwnKey()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"playhead.jog\",\"options\":{\"monitor\":\"clip\"}}},"
                             "\"knob2\":{\"turn\":{\"control\":\"playhead.jog\",\"options\":{\"monitor\":\"project\"}}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        c->settings.ackTimeoutMs = 5000;  // no timeout releases during the test
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
        e.handle(turn(1, 1));  // clip monitor jog
        e.handle(turn(2, 1));  // project monitor jog: same control name, other options
        QCOMPARE(kd.controlKeys.size(), 2);
        const QString liftKey = kd.controlKeys.at(0);
        e.handle(turn(1, 1));
        e.handle(turn(2, 1));
        QCOMPARE(kd.controlKeys.size(), 2);  // both waiting for their own ack
        Q_EMIT kd.controlAcked(liftKey, {{QStringLiteral("ok"), true}});  // ack for lift only
        QTRY_COMPARE(kd.controlKeys.size(), 3);
        QCOMPARE(kd.controlKeys.at(2), liftKey);  // gamma is still held back
        QTest::qWait(30);
        QVERIFY2(kd.controlKeys.size() == 3, qPrintable(kd.calls.join(QStringLiteral(" | "))));
    }

    // Item 4: no keyboard fallback unless the interface is absent.
    void fallbackOnlyWhenInterfaceAbsent()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setState(State::Absent);  // stock Kdenlive or interface off (the default)
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(1));
        e.handle(key(6));
        e.handle(turn(1, -1));
        e.handle(turn(1, -1));
        e.handle(turn(3, 1));
        e.handle(press(1));
        QTRY_COMPARE(keys.taps,
                     (QStringList{QStringLiteral("I"), QStringLiteral("shift+R"), QStringLiteral("LEFT"), QStringLiteral("LEFT"), QStringLiteral("ctrl+EQUAL"),
                                  QStringLiteral("SPACE")}));
        QVERIFY(kd.calls.isEmpty());
    }

    void pendingNeverTypesKeys()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setState(State::Pending);  // no definite answer yet (or a timeout)
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(1));
        e.handle(turn(1, 1));
        QTest::qWait(30);
        QVERIFY(keys.taps.isEmpty());
        QVERIFY(kd.calls.isEmpty());
        QVERIFY(kd.retries >= 1);  // asks again instead
    }

    void domainRefusalNeverTypesKeys()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QSignalSpy msgs(&e, &Engine::message);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(14));  // edit_undo, fallback ctrl+z
        Q_EMIT kd.refused(QStringLiteral("edit_undo"), contract::err::Modal, QStringLiteral("modal dialog open"));
        Q_EMIT kd.refused(QStringLiteral("edit_undo"), contract::err::ActionDisabled, QString());
        QTest::qWait(30);
        QVERIFY(keys.taps.isEmpty());
        bool reported = false;
        for (const auto &m : msgs) {
            reported = reported || m[0].toString().contains(QStringLiteral("modal"));
        }
        QVERIFY(reported);
    }

    void absentAtCallTimeFallsBackOnlyInSameWindow()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(2));  // mark_out
        Q_EMIT kd.actionFailed(QStringLiteral("mark_out"));  // interface vanished: stock allowed
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("O")});
        e.handle(key(14));
        e.setActiveWindow(kFirefox);
        Q_EMIT kd.actionFailed(QStringLiteral("edit_undo"));
        QTest::qWait(30);
        QCOMPARE(keys.taps.size(), 1);  // never into another window
    }

    // Staged capabilities: only advertised names are used.
    void capabilityGating()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setCapabilities(kStage1, {QStringLiteral("mark_in")}, {});
        Engine e(&keys, &kd);
        QSignalSpy msgs(&e, &Engine::message);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(1));
        e.handle(turn(1, 1));  // colour wheel layer: colorwheel.nudge not offered in MR1
        e.handle(turn(1, 1));
        e.handle(key(6));      // colorwheel.reset not offered
        e.handle(key(2));      // mark_out not in the allowlist
        e.handle(key(1));      // mark_in is
        QTest::qWait(30);
        QCOMPARE(kd.calls, QStringList{QStringLiteral("action mark_in")});
        QVERIFY(keys.taps.isEmpty());  // a capability gap is not a reason to type keys
        int unsupported = 0;
        for (const auto &m : msgs) {
            unsupported += m[0].toString().contains(QStringLiteral("does not offer")) ? 1 : 0;
        }
        QCOMPARE(unsupported, 3);  // reported once per name
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)}});
        e.handle(turn(1, 1));  // base layer: jog is MR1
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("control playhead.jog")));
    }

    void gestureTargetAndEndBarrier()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.gestureIdleMs = 100;
        e.setConfig(c);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(7));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 2);
        const QVariantMap first = kd.controlOptions.at(0);
        QCOMPARE(first.value(QStringLiteral("target")).toString(), QStringLiteral("cw-lift"));
        QCOMPARE(first.value(QStringLiteral("phase")).toString(), QStringLiteral("update"));
        QCOMPARE(first.value(QStringLiteral("wheel")).toString(), QStringLiteral("lift"));
        QCOMPARE(first.value(QStringLiteral("axis")).toString(), QStringLiteral("value"));
        const QString gesture = first.value(QStringLiteral("gesture")).toString();
        QVERIFY(!gesture.isEmpty());
        QCOMPARE(kd.controlOptions.at(1).value(QStringLiteral("gesture")).toString(), gesture);
        // Idle: an explicit end barrier with zero delta.
        QTRY_COMPARE_WITH_TIMEOUT(kd.controlOptions.size(), 3, 1000);
        QCOMPARE(kd.controlOptions.at(2).value(QStringLiteral("phase")).toString(), QStringLiteral("end"));
        QCOMPARE(kd.controlOptions.at(2).value(QStringLiteral("gesture")).toString(), gesture);
        QCOMPARE(kd.controlDeltas.at(2), 0.0);
        QCOMPARE(e.activeGestures(), 0);
        // The next turn is a new gesture.
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 4);
        QVERIFY(kd.controlOptions.at(3).value(QStringLiteral("gesture")).toString() != gesture);
    }

    void discreteOperationEndsGesturesFirst()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(3));
        e.handle(turn(3, 1));
        QTRY_COMPARE(kd.calls.size(), 1);
        e.handle(key(1));  // mark_in
        QTRY_COMPARE(kd.calls.size(), 3);
        QVERIFY(kd.calls.at(1).contains(QStringLiteral("(end)")));
        QCOMPARE(kd.calls.at(2), QStringLiteral("action mark_in"));
        e.handle(turn(3, 1));  // a new gesture...
        QTRY_COMPARE(kd.calls.size(), 4);
        QCOMPARE(e.activeGestures(), 1);
        e.handle(press(1));    // ...is closed by the (discrete) mode cycle
        QCOMPARE(e.activeGestures(), 0);
        QVERIFY(kd.calls.at(4).contains(QStringLiteral("(end)")));
        QCOMPARE(kd.calls.at(5), QStringLiteral("notify Lift: r"));
    }

    void epochChangeDropsQueuedMotion()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(10));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QCOMPARE(kd.controlDeltas.size(), 1);
        kd.setContext(wheelContext(11, QStringLiteral("cw2")));  // focus moved to another effect
        QCOMPARE(e.activeGestures(), 0);
        kd.ackAll();
        QTest::qWait(50);
        QCOMPARE(kd.controlDeltas.size(), 1);  // the two queued detents never reach cw-2
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlDeltas.size(), 2);
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("target")).toString(), QStringLiteral("cw2-lift"));
        // A context update without a new epoch (value/serial only) keeps the gesture.
        QVariantMap ctx = wheelContext(11, QStringLiteral("cw2"));
        ctx.insert(QStringLiteral("position"), 99);
        kd.setContext(ctx);
        QCOMPARE(e.activeGestures(), 1);
    }

    void missingTargetSendsNothing()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        // Without a host-issued param target the effect-param layer does not apply.
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)},
                       {QStringLiteral("focus"), QStringLiteral("effectStack")},
                       {QStringLiteral("param"), QVariantMap{{QStringLiteral("name"), QStringLiteral("level")}}}});
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob1"), 1))->layer, QString());
        // Editing bindings never guess a target (no first-matching-effect fallback).
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"key1\":{\"request\":\"colorwheel.reset\",\"params\":{\"wheel\":\"lift\"},\"fallback\":\"x\"},"
                             "\"knob1\":{\"turn\":{\"control\":\"colorwheel.nudge\",\"options\":{\"wheel\":\"lift\"},\"fallback\":[\"a\",\"b\"]}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
        kd.calls.clear();
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)}, {QStringLiteral("focus"), QStringLiteral("timeline")}});
        e.handle(key(1));
        e.handle(turn(1, 1));
        QTest::qWait(30);
        QVERIFY(kd.calls.isEmpty());
        QVERIFY(keys.taps.isEmpty());  // available interface: never keys
    }

    void hoverTargetIsOptIn()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"colorwheel.nudge\",\"options\":{\"wheel\":\"gain\",\"axis\":\"value\"}}},"
                             "\"knob2\":{\"turn\":{\"control\":\"colorwheel.nudge\",\"targetFrom\":\"hoveredColorWheel.target\",\"options\":{\"wheel\":\"gain\"}}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 9, QStringLiteral("0x9")});
        QVariantMap ctx = wheelContext(4);
        auto hover = [](const QString &target, const QString &wheel) {
            return QVariantMap{{QStringLiteral("target"), target}, {QStringLiteral("wheel"), wheel}};
        };
        ctx.insert(QStringLiteral("hoveredColorWheel"), hover(QStringLiteral("cw-hover"), QStringLiteral("gain")));
        kd.setContext(ctx);
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        QCOMPARE(kd.controlOptions.at(0).value(QStringLiteral("target")).toString(), QStringLiteral("cw-gain"));  // colorWheels, not hover
        e.handle(turn(2, 1));
        // One active gesture per host: knob1's gesture ends before knob2's starts.
        QTRY_COMPARE(kd.controlOptions.size(), 3);
        QCOMPARE(kd.controlOptions.at(1).value(QStringLiteral("phase")).toString(), QStringLiteral("end"));
        QCOMPARE(kd.controlOptions.at(1).value(QStringLiteral("target")).toString(), QStringLiteral("cw-gain"));
        QCOMPARE(kd.controlOptions.at(2).value(QStringLiteral("target")).toString(), QStringLiteral("cw-hover"));
        QCOMPARE(e.activeGestures(), 1);
        // Hover moving (no epoch change) never retargets the open gesture.
        ctx.insert(QStringLiteral("hoveredColorWheel"), hover(QStringLiteral("cw-elsewhere"), QStringLiteral("gain")));
        kd.setContext(ctx);
        e.handle(turn(2, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 4);
        QCOMPARE(kd.controlOptions.at(3).value(QStringLiteral("target")).toString(), QStringLiteral("cw-hover"));
        QCOMPARE(kd.controlOptions.at(3).value(QStringLiteral("gesture")).toString(), kd.controlOptions.at(2).value(QStringLiteral("gesture")).toString());
        // Once that gesture has ended, a new one reads the hover again.
        e.endAllGestures(false);
        e.handle(turn(2, 1));
        QTRY_VERIFY(kd.controlOptions.last().value(QStringLiteral("target")).toString() == QStringLiteral("cw-elsewhere"));
        // A hovered wheel of another kind is not this binding's wheel: nothing is sent.
        e.endAllGestures(false);
        ctx.insert(QStringLiteral("hoveredColorWheel"), hover(QStringLiteral("cw-other"), QStringLiteral("lift")));
        kd.setContext(ctx);
        const int sent = int(kd.controlOptions.size());
        e.handle(turn(2, 1));
        QTest::qWait(30);
        QCOMPARE(int(kd.controlOptions.size()), sent);
    }

    // MR2: three knobs edit the three wheels through the colorWheels handles,
    // without moving keyboard focus; the host's single active gesture is
    // respected by ending ours before another starts.
    void threeKnobsUseColorWheelsHandles()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(2, QStringLiteral("w"), QStringLiteral("gamma")));  // gamma has keyboard focus
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob3"), 1))->layer, QStringLiteral("color-wheels"));
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        e.handle(turn(2, -1));
        QTRY_COMPARE(kd.controlOptions.size(), 3);
        e.handle(turn(3, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 5);
        QStringList seen;
        for (const auto &o : std::as_const(kd.controlOptions)) {
            seen << o.value(QStringLiteral("target")).toString() + QLatin1Char('/') + o.value(QStringLiteral("phase")).toString() + QLatin1Char('/')
                    + o.value(QStringLiteral("wheel")).toString();
        }
        QCOMPARE(seen, (QStringList{QStringLiteral("w-lift/update/lift"), QStringLiteral("w-lift/end/lift"), QStringLiteral("w-gamma/update/gamma"),
                                    QStringLiteral("w-gamma/end/gamma"), QStringLiteral("w-gain/update/gain")}));
        QCOMPARE(e.activeGestures(), 1);
        // Per-wheel resets use the same handles.
        e.handle(key(8));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("invoke colorwheel.reset")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"target\":\"w-gain\"")));
        QCOMPARE(e.activeGestures(), 0);  // the reset ended the gain gesture first
    }

    // Without colorWheels (older host or no widget), only the focused wheel is
    // addressable, and only by a binding for that wheel; a descriptor without a
    // wheel kind is accepted as is.
    void wheelTargetFallbacks()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"colorwheel.nudge\",\"options\":{\"wheel\":\"lift\"}}},"
                             "\"knob2\":{\"turn\":{\"control\":\"colorwheel.nudge\",\"options\":{\"wheel\":\"gamma\"}}},"
                             "\"knob3\":{\"turn\":{\"control\":\"playhead.jog\"}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
        QVariantMap ctx = wheelContext(3, QStringLiteral("f"), QStringLiteral("gamma"));
        ctx.remove(QStringLiteral("colorWheels"));
        kd.setContext(ctx);
        e.handle(turn(1, 1));  // lift: not focused, no handle
        QTest::qWait(30);
        QVERIFY(kd.controlOptions.isEmpty());
        e.handle(turn(2, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        QCOMPARE(kd.controlOptions.at(0).value(QStringLiteral("target")).toString(), QStringLiteral("f-gamma"));
        // Any other control ends the open editing gesture first (Kdenlive would).
        e.handle(turn(3, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 3);
        QCOMPARE(kd.controlOptions.at(1).value(QStringLiteral("phase")).toString(), QStringLiteral("end"));
        QVERIFY(kd.calls.last().startsWith(QStringLiteral("control playhead.jog")));
        QCOMPARE(e.activeGestures(), 0);
        // A wheel descriptor without a kind is taken as is.
        ctx.insert(QStringLiteral("colorWheel"), QVariantMap{{QStringLiteral("target"), QStringLiteral("legacy")}});
        ctx.insert(QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(4));
        kd.setContext(ctx);
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 4);
        QCOMPARE(kd.controlOptions.at(3).value(QStringLiteral("target")).toString(), QStringLiteral("legacy"));
    }

    void tapPacingAndReversal()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setState(State::Absent);
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.keyRateHz = 50;  // 20 ms per tap
        e.setConfig(c);
        e.setActiveWindow(kKdenlive);
        for (int i = 0; i < 10; ++i) {
            e.handle(turn(1, 1));
        }
        QCOMPARE(keys.taps.size(), 1);
        QCOMPARE(e.pendingTaps(), 9);
        e.handle(turn(1, -1));  // reversing drops the queued opposite motion
        QCOMPARE(e.pendingTaps(), 1);
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("RIGHT"), QStringLiteral("LEFT")}));
    }

    void colourWheelAxisCycling()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(wheelContext(1));
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob1"), 1))->layer, QStringLiteral("color-wheels"));
        e.handle(press(1));
        QCOMPARE(kd.calls.last(), QStringLiteral("notify Lift: r"));
        e.handle(turn(1, -1));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("control colorwheel.nudge -1")));
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("axis")).toString(), QStringLiteral("r"));
        e.handle(key(6));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("invoke colorwheel.reset")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"target\":\"cw-lift\"")));
    }

    void trimLayerNeedsQualifiedToolAndTarget()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)}, {QStringLiteral("tool"), QStringLiteral("slip")}});
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob2"), 1))->binding.name, QStringLiteral("playhead.shuttle"));
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)},
                       {QStringLiteral("tool"), QStringLiteral("ripple")},
                       {QStringLiteral("edit"), QVariantMap{{QStringLiteral("target"), QStringLiteral("ed-1")}}}});
        e.handle(turn(2, -1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        const QVariantMap o = kd.controlOptions.first();
        QCOMPARE(o.value(QStringLiteral("mode")).toString(), QStringLiteral("ripple"));
        QCOMPARE(o.value(QStringLiteral("edge")).toString(), QStringLiteral("end"));
        QCOMPARE(o.value(QStringLiteral("target")).toString(), QStringLiteral("ed-1"));
    }

    void provisionalFocusSendsAndTypesNothing()
    {
        for (const State st : {State::Available, State::Absent}) {
            RecordingKeySink keys;
            FakeKdenliveClient kd;
            kd.setState(st);
            Engine e(&keys, &kd);
            e.setConfig(m_cfg);
            e.setActiveWindow(kKdenlive);  // instance A, pid 4242
            // Focus moves to Kdenlive instance B; its pid is not known yet.
            e.setActiveWindow(WindowInfo{kKdenlive.cls, QStringLiteral("B"), 0, QStringLiteral("0xb")});
            QCOMPARE(kd.attachedPid(), 4242);  // still attached to A
            e.handle(key(1));
            e.handle(turn(1, 1));
            QTest::qWait(30);
            QVERIFY2(kd.calls.isEmpty(), "nothing goes to the unfocused instance A");
            QVERIFY2(keys.taps.isEmpty(), "A's absence says nothing about B: no keys");
            // The query answers: B attaches, input flows normally again.
            e.setActiveWindow(WindowInfo{kKdenlive.cls, QStringLiteral("B"), 5151, QStringLiteral("0xb")});
            QCOMPARE(kd.attachedPid(), 5151);
            e.handle(key(1));
            if (st == State::Available) {
                QTRY_COMPARE(kd.calls, QStringList{QStringLiteral("action mark_in")});
            } else {
                QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("I")});
            }
        }
    }

    void provisionalFocusKeepsAttachment()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        WindowInfo provisional = kKdenlive;
        provisional.pid = 0;
        provisional.title = QStringLiteral("other title");
        e.setActiveWindow(provisional);
        QCOMPARE(kd.attachedPid(), 4242);
        e.setActiveWindow(kFirefox);
        QCOMPARE(kd.attachedPid(), 0);
    }

    void turnPrecedencePerLevel()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":["
                             "{\"name\":\"term\",\"match\":{\"class\":\"^term$\"},\"bindings\":{\"knob1\":{\"cw\":\"ctrl+tab\"},\"knob3\":{\"turn\":\"none\"}}},"
                             "{\"name\":\"global\",\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"x\",\"fallback\":[\"volumedown\",\"volumeup\"]}},"
                             " \"knob3\":{\"ccw\":\"volumedown\",\"cw\":\"volumeup\"}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("term"), {}, 2, QStringLiteral("0x2")});
        e.handle(turn(1, 1));
        e.handle(turn(3, 1));
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("ctrl+TAB")});
        QTest::qWait(30);
        QCOMPARE(keys.taps.size(), 1);
    }

    void windowSwitchDropsPendingWork()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.keyRateHz = 20;
        e.setConfig(c);
        e.setActiveWindow(kFirefox);
        for (int i = 0; i < 6; ++i) {
            e.handle(turn(1, 1));
        }
        e.setActiveWindow(WindowInfo{QStringLiteral("kitty"), QStringLiteral("shell"), 88, QStringLiteral("0x3")});
        QCOMPARE(e.pendingTaps(), 0);
        e.setActiveWindow(kKdenlive);
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.setActiveWindow(kFirefox);
        kd.ackAll();
        QTest::qWait(50);
        QCOMPARE(kd.controlDeltas.size(), 1);
    }

    void commandsAndAcceleration()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.accelFactor = 4;
        c.settings.accelWindowMs = 1000;
        e.setConfig(c);
        e.setActiveWindow(kKdenlive);
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        kd.ackAll();
        QTRY_COMPARE(kd.controlDeltas.size(), 2);
        QCOMPARE(kd.controlDeltas[1], 4.0);
        e.setActiveWindow(kFirefox);
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("VOLUMEUP"), QStringLiteral("VOLUMEUP")}));
    }
};

QTEST_GUILESS_MAIN(TestEngine)
#include "tst_engine.moc"
