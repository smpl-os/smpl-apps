// SPDX-License-Identifier: GPL-2.0-or-later
#include "engine.h"

#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

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
        e.handle(key(1));  // unbound globally: nothing
        QTest::qWait(30);
        QCOMPARE(keys.taps.size(), 4);
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
        QVERIFY(e.kdenliveActive());
        e.handle(key(1));
        QCOMPARE(kd.calls.value(0), QStringLiteral("action mark_in"));
        for (int i = 0; i < 40; ++i) {
            e.handle(turn(1, 1));
        }
        // One update in flight, the other 39 detents wait merged.
        QCOMPARE(kd.calls.size(), 2);
        QCOMPARE(kd.controlDeltas.value(0), 1.0);
        kd.ackAll();
        QTRY_COMPARE(kd.controlDeltas.size(), 2);
        QCOMPARE(kd.controlDeltas.value(1), 39.0);
        QVERIFY(kd.calls.value(1).startsWith(QStringLiteral("control playhead.jog 1")));
        QVERIFY(keys.taps.isEmpty());
    }

    void stockKdenliveUsesFallbackKeys()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAvailable(false);  // no ControlSurface1 interface
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
        e.handle(key(13));  // ripple_tool has no fallback: dropped, not mistyped
        QTest::qWait(20);
        QCOMPARE(keys.taps.size(), 6);
    }

    void tapPacingAndReversal()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAvailable(false);
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.keyRateHz = 50;  // 20 ms per tap
        e.setConfig(c);
        e.setActiveWindow(kKdenlive);
        for (int i = 0; i < 10; ++i) {
            e.handle(turn(1, 1));
        }
        QCOMPARE(keys.taps.size(), 1);  // first tap immediately, rest paced
        QVERIFY(e.pendingTaps() == 9);
        e.handle(turn(1, -1));  // reversing drops the queued opposite motion
        QCOMPARE(e.pendingTaps(), 1);
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("RIGHT"), QStringLiteral("LEFT")}));
    }

    void colorWheelLayerAndAxisCycling()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext({{QStringLiteral("focus"), QStringLiteral("effectStack")},
                       {QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}}},
                       {QStringLiteral("param"), QVariantMap{{QStringLiteral("name"), QStringLiteral("lift_r")}, {QStringLiteral("type"), QStringLiteral("colorwheel")}}}});
        QCOMPARE(e.resolve(QStringLiteral("knob1.turn"))->layer, QStringLiteral("color-wheels"));
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.calls.size(), 1);
        QCOMPARE(kd.calls[0], QStringLiteral("control colorwheel.nudge 1 {\"axis\":\"luma\",\"wheel\":\"lift\"}"));
        e.handle(press(1));
        QCOMPARE(kd.calls.last(), QStringLiteral("notify Lift: r"));
        QCOMPARE(e.modeValue(QStringLiteral("liftAxis")), QStringLiteral("r"));
        e.handle(turn(1, -1));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("control colorwheel.nudge -1")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"axis\":\"r\"")));
        e.handle(turn(3, 1));  // gain wheel keeps its own axis
        QTRY_VERIFY(kd.calls.last().contains(QStringLiteral("\"axis\":\"luma\",\"wheel\":\"gain\"")));
        e.handle(key(6));
        QCOMPARE(kd.calls.last(), QStringLiteral("invoke colorwheel.reset {\"wheel\":\"lift\"}"));
        e.handle(key(1));  // not overridden by the layer: base binding
        QCOMPARE(kd.calls.last(), QStringLiteral("action mark_in"));
    }

    void otherContextLayers()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext({{QStringLiteral("focus"), QStringLiteral("automationEditor")}});
        QCOMPARE(e.resolve(QStringLiteral("knob1.turn"))->layer, QStringLiteral("automation"));
        kd.setContext({{QStringLiteral("focus"), QStringLiteral("effectStack")}});
        QCOMPARE(e.resolve(QStringLiteral("knob1.turn"))->layer, QString());  // no focused param: base jog
        kd.setContext({{QStringLiteral("focus"), QStringLiteral("effectStack")}, {QStringLiteral("param"), QVariantMap{{QStringLiteral("name"), QStringLiteral("level")}}}});
        QCOMPARE(e.resolve(QStringLiteral("knob1.turn"))->layer, QStringLiteral("effect-param"));
        kd.setContext({{QStringLiteral("tool"), QStringLiteral("slip")}});
        QCOMPARE(e.resolve(QStringLiteral("knob2.turn"))->binding.name, QStringLiteral("edit.trim"));
        kd.setContext({{QStringLiteral("tool"), QStringLiteral("select")}});
        QCOMPARE(e.resolve(QStringLiteral("knob2.turn"))->binding.name, QStringLiteral("playhead.shuttle"));
        // Kdenlive profile falls through to the global profile for unbound slots
        QCOMPARE(e.resolve(QStringLiteral("knob1.cw"))->profile, QStringLiteral("global"));
    }

    void failedActionFallsBackToKeys()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(2));
        QCOMPARE(kd.calls.last(), QStringLiteral("action mark_out"));
        Q_EMIT kd.actionFailed(QStringLiteral("mark_out"));
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("O")});
    }

    void focusChangeDropsPendingMotion()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QCOMPARE(kd.controlDeltas.size(), 1);
        e.setActiveWindow(kFirefox);
        QCOMPARE(kd.attachedPid(), 0);
        kd.ackAll();
        QTest::qWait(100);
        QCOMPARE(kd.controlDeltas.size(), 1);  // the two merged detents never reach any app
        QVERIFY(keys.taps.isEmpty());
    }

    void commandsAndExplicitNone()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[\n"
            "            {\"name\":\"app\",\"match\":{\"class\":\"^app$\"},\"bindings\":{\"key1\":{\"command\":[\"true\",\"x\"]},\"key2\":\"none\"}},\n"
            "            {\"name\":\"global\",\"bindings\":{\"key2\":\"a\",\"key3\":\"b\"}}]}", {}, &err);
        QVERIFY2(c, qPrintable(err));
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("app"), {}, 1, {}});
        QSignalSpy spy(&e, &Engine::runCommand);
        e.handle(key(1));
        QCOMPARE(spy.size(), 1);
        QCOMPARE(spy[0][0].toStringList(), (QStringList{QStringLiteral("true"), QStringLiteral("x")}));
        e.handle(key(2));  // "none" blocks the global fallthrough
        e.handle(key(3));  // unbound in app: global applies
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("B")});
    }

    void accelerationMultipliesFastDetents()
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
        QCOMPARE(kd.controlDeltas[0], 1.0);
        QCOMPARE(kd.controlDeltas[1], 4.0);

        // Discrete bindings (volume keys in the global profile) are never multiplied.
        e.setActiveWindow(kFirefox);
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("VOLUMEUP"), QStringLiteral("VOLUMEUP")}));
        QTest::qWait(30);
        QCOMPARE(keys.taps.size(), 2);
    }
};

QTEST_GUILESS_MAIN(TestEngine)
#include "tst_engine.moc"
