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

    // Stock-key fallback is an explicit per-profile opt-in.
    Config optIn() const
    {
        Config c = m_cfg;
        for (auto &p : c.profiles) {
            if (p.kdenlive) {
                p.keyFallback = true;
            }
        }
        return c;
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

    void mouseBindingsPerDetent()
    {
        QString err;
        auto c = parseConfig(R"({"profiles":[{"name":"g","bindings":{"key1":{"mouse":"back"},"knob1.cw":{"mouse":"wheel-down"},"knob1.ccw":{"mouse":"wheel-up"}}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        c->settings.accelFactor = 1.0;
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(*c);
        e.setActiveWindow(kFirefox);
        e.handle(key(1));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        QTRY_COMPARE(keys.taps, (QStringList{QStringLiteral("mouse:back"), QStringLiteral("mouse:wheel-down"), QStringLiteral("mouse:wheel-down"), QStringLiteral("mouse:wheel-down")}));
        keys.taps.clear();
        // Reversing drops wheel motion not yet sent in the old direction.
        for (int i = 0; i < 20; ++i) {
            e.handle(turn(1, 1));
        }
        e.handle(turn(1, -1));
        QTRY_VERIFY(keys.taps.contains(QStringLiteral("mouse:wheel-up")));
        QTest::qWait(300);
        QVERIFY(keys.taps.count(QStringLiteral("mouse:wheel-down")) < 20);
        QCOMPARE(keys.taps.last(), QStringLiteral("mouse:wheel-up"));
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

    // Press+turn: a turn while the knob is held uses knobN.shift.*; the press
    // of a knob with shift bindings fires on release, and only if it did not turn.
    void pressAndTurnShift()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"playhead.jog\"},\"press\":{\"action\":\"monitor_play\"},"
                             "\"shift\":{\"turn\":{\"control\":\"timeline.scroll\"}}},"
                             "\"knob2\":{\"turn\":{\"control\":\"timeline.zoom\"},\"press\":{\"action\":\"zoom_fit\"}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        c->settings.accelFactor = 1;
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
        const PadEvent up1{QStringLiteral("knob1"), PadEvent::PressUp, 0, 0};
        // Hold and turn: shift binding, and the press is swallowed.
        e.handle(press(1));
        QVERIFY(kd.calls.isEmpty());  // deferred to release
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.calls, QStringList{QStringLiteral("control timeline.scroll 1")});
        e.handle(up1);
        QTest::qWait(20);
        QCOMPARE(kd.calls.size(), 1);
        // Not held: the normal turn.
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.calls.size(), 2);
        QCOMPARE(kd.calls.last(), QStringLiteral("control playhead.jog 1"));
        // Press and release without turning: the press fires on release.
        e.handle(press(1));
        QCOMPARE(kd.calls.size(), 2);
        e.handle(up1);
        QCOMPARE(kd.calls.last(), QStringLiteral("action monitor_play"));
        // A knob without shift bindings fires on press, and a held turn is a normal turn.
        e.handle(press(2));
        QCOMPARE(kd.calls.last(), QStringLiteral("action zoom_fit"));
        e.handle(turn(2, -1));
        QTRY_COMPARE(kd.calls.last(), QStringLiteral("control timeline.zoom -1"));
        e.handle(PadEvent{QStringLiteral("knob2"), PadEvent::PressUp, 0, 0});
        QTest::qWait(20);
        QCOMPARE(kd.calls.size(), 5);
        QVERIFY(keys.taps.isEmpty());
        // A deferred press never fires into another profile.
        e.handle(press(1));
        e.setActiveWindow(kFirefox);
        e.handle(up1);
        QTest::qWait(20);
        QCOMPARE(kd.calls.size(), 5);
        QVERIFY(keys.taps.isEmpty());
    }

    // "accel" on a binding overrides settings.accelFactor; 1 turns it off.
    void perBindingAcceleration()
    {
        QString err;
        auto c = parseConfig("{\"settings\":{\"accelFactor\":3,\"accelWindowMs\":1000},"
                             "\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"playhead.jog\"}},"
                             "\"knob2\":{\"turn\":{\"control\":\"timeline.zoom\",\"accel\":1}},"
                             "\"knob3\":{\"turn\":{\"control\":\"timeline.track\",\"accel\":5}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->profiles.first().bindings.value(QStringLiteral("knob3.turn")).accel, 5.0);
        for (const auto &[knob, expected] : {std::pair{1, 3.0}, std::pair{2, 1.0}, std::pair{3, 5.0}}) {
            RecordingKeySink keys;
            FakeKdenliveClient kd;
            kd.setAutoAck(false);
            Engine e(&keys, &kd);
            e.setConfig(*c);
            e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
            e.handle(turn(knob, 1));  // first detent: never accelerated
            e.handle(turn(knob, 1));  // within the window
            kd.ackAll();
            QTRY_COMPARE(kd.controlDeltas.size(), 2);
            QCOMPARE(kd.controlDeltas.at(0), 1.0);
            QCOMPARE(kd.controlDeltas.at(1), expected);
        }
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
        e.setConfig(optIn());
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

    // The default: Kdenlive is driven only through its API. With the interface
    // absent or off nothing is typed; the user gets one clear notice.
    void absentIsApiOnlyByDefault()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setState(State::Absent);
        Engine e(&keys, &kd);
        QSignalSpy notices(&e, &Engine::notice);
        QSignalSpy msgs(&e, &Engine::message);
        e.setConfig(m_cfg);
        QVERIFY(!e.activeProfile() || !e.activeProfile()->keyFallback);
        e.setActiveWindow(kKdenlive);
        QVERIFY(!e.activeProfile()->keyFallback);
        e.handle(key(1));        // action
        e.handle(key(13));       // daemon mode cycle still works
        e.handle(turn(1, -1));   // control
        e.handle(turn(3, 1));
        e.handle(press(1));
        QTest::qWait(60);
        QVERIFY2(keys.taps.isEmpty(), "API-only: no stock shortcuts without keyFallback");
        QVERIFY(kd.calls.isEmpty());
        QCOMPARE(notices.size(), 1);  // once per attachment, not per input
        QVERIFY(notices.first().first().toString().contains(QStringLiteral("Kdenlive control interface not enabled")));
        QVERIFY(notices.first().first().toString().contains(QStringLiteral("keyFallback")));
        int absent = 0;
        for (const auto &m : msgs) {
            absent += m[0].toString().contains(QStringLiteral("not enabled")) ? 1 : 0;
        }
        QCOMPARE(absent, 1);
        // A new attachment (another Kdenlive instance) is told again.
        e.setActiveWindow(kFirefox);
        e.setActiveWindow(WindowInfo{kKdenlive.cls, QStringLiteral("B"), 5151, QStringLiteral("0xb")});
        e.handle(key(1));
        QTRY_COMPARE(notices.size(), 2);
        QVERIFY(keys.taps.isEmpty());
        // Other apps keep their uinput profiles.
        e.setActiveWindow(kFirefox);
        e.handle(key(12));
        QTRY_COMPARE(keys.taps, QStringList{QStringLiteral("PLAYPAUSE")});
    }

    void absentAtCallTimeWithoutOptInOnlyNotices()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QSignalSpy notices(&e, &Engine::notice);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        e.handle(key(2));  // mark_out through the interface
        QTRY_COMPARE(kd.calls, QStringList{QStringLiteral("action mark_out")});
        Q_EMIT kd.actionFailed(QStringLiteral("mark_out"));  // the interface vanished
        QTest::qWait(30);
        QVERIFY(keys.taps.isEmpty());
        QCOMPARE(notices.size(), 1);
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

    // MR1a: busy for shuttle/playback while Slip's trimming preview is active
    // gets a hint; it never types keys.
    void slipPreviewBusyHint()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        QSignalSpy msgs(&e, &Engine::message);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        auto hints = [&] {
            int n = 0;
            for (const auto &m : msgs) {
                n += m[0].toString().contains(QStringLiteral("select_tool")) ? 1 : 0;
            }
            return n;
        };
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)}, {QStringLiteral("tool"), QStringLiteral("select")}});
        Q_EMIT kd.refused(contract::kShuttle, contract::err::Busy, QStringLiteral("another caller"));
        QCOMPARE(hints(), 0);  // a writer conflict is not about the tool
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)}, {QStringLiteral("tool"), QStringLiteral("slip")}});
        Q_EMIT kd.refused(contract::kShuttle, contract::err::Busy, QStringLiteral("monitor trimming preview"));
        Q_EMIT kd.refused(QStringLiteral("monitor_play"), contract::err::Busy, QString());
        Q_EMIT kd.refused(QStringLiteral("cut_timeline_clip"), contract::err::Busy, QString());  // editing: no hint
        QCOMPARE(hints(), 2);
        // The host's disabled playback actions reveal a trimming preview with any tool.
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(3)}, {QStringLiteral("tool"), QStringLiteral("ripple")}});
        Q_EMIT kd.refused(QStringLiteral("monitor_loop_zone"), contract::err::Busy, QString());
        QCOMPARE(hints(), 2);
        kd.disabledActions << QStringLiteral("monitor_play");
        Q_EMIT kd.refused(QStringLiteral("monitor_loop_zone"), contract::err::Busy, QString());
        QCOMPARE(hints(), 3);
        QTest::qWait(20);
        QVERIFY(keys.taps.isEmpty());
    }

    void absentAtCallTimeFallsBackOnlyInSameWindow()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(optIn());
        e.setActiveWindow(kKdenlive);
        e.handle(key(2));  // mark_out
        Q_EMIT kd.actionFailed(QStringLiteral("mark_out"));  // interface vanished: stock allowed (opt-in)
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
        Config c = optIn();
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

    // MR3 timeline page: a daemon mode (key13) selects it, but only while
    // Kdenlive publishes track handles; targets come from the MR3 locations.
    static QVariantMap timelineContext(quint64 epoch, bool clipSelected, bool muted = false)
    {
        // Kdenlive's qualified MR3 shape (TimelineControl::context).
        QVariantMap track{{QStringLiteral("target"), QStringLiteral("trk-7")}, {QStringLiteral("id"), 7}, {QStringLiteral("sequence"), QStringLiteral("s")},
                          {QStringLiteral("audio"), true}, {QStringLiteral("name"), QStringLiteral("A1")},
                          {QStringLiteral("muted"), muted}, {QStringLiteral("solo"), false}, {QStringLiteral("locked"), false},
                          {QStringLiteral("hidden"), false}, {QStringLiteral("targeted"), true},
                          {QStringLiteral("gain"), QVariantMap{{QStringLiteral("target"), QStringLiteral("trk-7")}, {QStringLiteral("value"), 0.0}}}};
        QVariantMap timeline{{QStringLiteral("zoom"), 10}, {QStringLiteral("track"), track}};
        if (clipSelected) {
            timeline.insert(QStringLiteral("clipGain"), QVariantMap{{QStringLiteral("target"), QStringLiteral("gain-c")}, {QStringLiteral("clip"), 22}});
            timeline.insert(QStringLiteral("trim"), QVariantMap{{QStringLiteral("target"), QStringLiteral("trim-c")}, {QStringLiteral("clip"), 22},
                                                                {QStringLiteral("clips"), QVariantList{21, 22}}, {QStringLiteral("tracks"), QVariantList{3, 7}},
                                                                {QStringLiteral("modes"), QStringList{QStringLiteral("resize")}}});
        }
        return {{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(epoch)}, {QStringLiteral("focus"), QStringLiteral("timeline")},
                {QStringLiteral("position"), 0}, {QStringLiteral("timeline"), timeline}};
    }

    void timelinePageNeedsModeAndTrackHandles()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(timelineContext(1, true));
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob2"), 1))->binding.name, QStringLiteral("playhead.shuttle"));  // main page
        e.handle(key(13));
        QCOMPARE(e.modeValue(QStringLiteral("page")), QStringLiteral("timeline"));
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob2"), 1))->layer, QStringLiteral("timeline-page"));
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob3"), 1))->layer, QStringLiteral("timeline-clip-gain"));
        // Without MR3 track handles the page never applies.
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)},
                       {QStringLiteral("timeline"), QVariantMap{{QStringLiteral("track"), QVariantMap{{QStringLiteral("id"), 7}}}}}});
        QCOMPARE(e.resolve(Engine::turnSlots(QStringLiteral("knob2"), 1))->binding.name, QStringLiteral("playhead.shuttle"));
        QCOMPARE(e.resolve(QStringLiteral("key1"))->binding.name, QStringLiteral("mark_in"));
        kd.setContext(timelineContext(3, true));
        // knob2 resizes the declared trim scope; knob3 drives the clip gain.
        e.handle(turn(2, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        QCOMPARE(kd.calls.last().section(QLatin1Char(' '), 0, 1), QStringLiteral("control edit.trim"));
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("target")).toString(), QStringLiteral("trim-c"));
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("mode")).toString(), QStringLiteral("resize"));
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("edge")).toString(), QStringLiteral("end"));
        e.handle(turn(3, -1));
        QTRY_COMPARE(kd.controlOptions.size(), 3);  // trim end barrier, then gain
        QCOMPARE(kd.controlOptions.at(1).value(QStringLiteral("phase")).toString(), QStringLiteral("end"));
        QCOMPARE(kd.controlOptions.last().value(QStringLiteral("target")).toString(), QStringLiteral("gain-c"));
        // No clip selected: knob3 is the track's mixer gain; knob2 has no trim target.
        kd.setContext(timelineContext(4, false));
        e.handle(turn(3, 1));
        QTRY_COMPARE(kd.controlOptions.last().value(QStringLiteral("target")).toString(), QStringLiteral("trk-7"));
        QCOMPARE(kd.calls.last().section(QLatin1Char(' '), 0, 1), QStringLiteral("control audio.gain"));
        const int sent = int(kd.controlOptions.size());
        e.handle(turn(2, 1));
        QTest::qWait(30);
        QCOMPARE(int(kd.controlOptions.size()), sent);  // no trim target: nothing sent, the gain gesture stays open
        QCOMPARE(e.activeGestures(), 1);
        // knob1: track focus; its press switches it to scroll ("control": "$trackKnob").
        e.handle(turn(1, 1));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("control timeline.track 1")));
        e.handle(press(1));
        e.handle(turn(1, -1));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("control timeline.scroll -1")));
        QVERIFY(keys.taps.isEmpty());
    }

    void trackTogglesNeedPublishedState()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        kd.setContext(timelineContext(1, false, true));  // the track is muted
        e.handle(key(13));
        e.handle(key(1));
        QTRY_VERIFY(kd.calls.last().startsWith(QStringLiteral("invoke track.set")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"value\":false")));  // toggles mute off
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"target\":\"trk-7\"")));
        e.handle(key(2));
        QTRY_VERIFY(kd.calls.last().contains(QStringLiteral("\"what\":\"solo\"")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"value\":true")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"soloMode\":\"exclusive\"")));
        // A host that does not publish the state: never toggle blindly.
        QVariantMap ctx = timelineContext(2, false);
        QVariantMap timeline = ctx.value(QStringLiteral("timeline")).toMap();
        QVariantMap track = timeline.value(QStringLiteral("track")).toMap();
        track.remove(QStringLiteral("muted"));
        timeline.insert(QStringLiteral("track"), track);
        ctx.insert(QStringLiteral("timeline"), timeline);
        kd.setContext(ctx);
        const int calls = int(kd.calls.size());
        QSignalSpy msgs(&e, &Engine::message);
        e.handle(key(1));
        QTest::qWait(30);
        QCOMPARE(int(kd.calls.size()), calls);
        QVERIFY(!msgs.isEmpty() && msgs.last().at(0).toString().contains(QStringLiteral("timeline.track.muted")));
        // On a video track key1 hides (layer timeline-video-track).
        ctx = timelineContext(3, false);
        timeline = ctx.value(QStringLiteral("timeline")).toMap();
        track = timeline.value(QStringLiteral("track")).toMap();
        track.insert(QStringLiteral("audio"), false);
        track.insert(QStringLiteral("hidden"), true);
        timeline.insert(QStringLiteral("track"), track);
        ctx.insert(QStringLiteral("timeline"), timeline);
        kd.setContext(ctx);
        e.handle(key(1));
        QTRY_VERIFY(kd.calls.last().contains(QStringLiteral("\"what\":\"hide\"")));
        QVERIFY(kd.calls.last().contains(QStringLiteral("\"value\":false")));
        QVERIFY(keys.taps.isEmpty());
    }

    // Found in the real-Kdenlive acceptance: a second quick detent on a track or
    // parameter-focus knob was sent with the epoch the first step had just
    // replaced (stale_context) and the rest was dropped. The next batch now waits
    // for the new epoch, and the self-caused epoch change keeps those steps.
    void navigationWaitsForItsOwnEpochChange()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setAutoAck(false);
        Engine e(&keys, &kd);
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"kd\",\"match\":{\"class\":\"^kd$\"},\"kdenlive\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"timeline.track\"}},\"knob2\":{\"turn\":{\"control\":\"playhead.jog\"}}}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        c->settings.accelFactor = 1.0;
        e.setConfig(*c);
        e.setActiveWindow(WindowInfo{QStringLiteral("kd"), {}, 5, QStringLiteral("0x5")});
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)}});
        const QVariantMap changed{{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("changed"), true}}}};
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.handle(turn(1, 1));
        e.handle(turn(2, 1));  // other work: a jog that the epoch change must still drop
        QCOMPARE(kd.controlDeltas.size(), 2);  // one track step and one jog in flight
        kd.ackAll(changed);
        QTest::qWait(30);
        QCOMPARE(kd.controlDeltas.size(), 2);  // held: the context still shows the old track
        kd.setContext({{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(2)}});
        QTRY_COMPARE(kd.controlDeltas.size(), 3);
        QCOMPARE(kd.controlDeltas.at(2), 2.0);  // both queued steps, sent under the new epoch
        QVERIFY(kd.calls.last().startsWith(QStringLiteral("control timeline.track 2")));
        // No epoch change follows (e.g. already at the last track): the timer releases it.
        kd.ackAll(changed);  // the batch of 2: held again
        e.handle(turn(1, 1));
        QTest::qWait(30);
        QCOMPARE(kd.controlDeltas.size(), 3);
        QTRY_COMPARE_WITH_TIMEOUT(kd.controlDeltas.size(), 4, 1000);
        QCOMPARE(kd.controlDeltas.at(3), 1.0);
        // An unchanged step releases at once.
        kd.ackAll({{QStringLiteral("ok"), true}, {QStringLiteral("result"), QVariantMap{{QStringLiteral("changed"), false}}}});
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlDeltas.size(), 5);
        QVERIFY(keys.taps.isEmpty());
    }

    // limits.trimGestureSteps: the daemon ends a trim gesture before the host's
    // bound and continues with a fresh one.
    void trimGestureRespectsStepLimit()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        kd.setLimits({{QStringLiteral("trimGestureSteps"), 3}});
        Engine e(&keys, &kd);
        Config c = m_cfg;
        c.settings.coalesceMs = 1;
        e.setConfig(c);
        e.setActiveWindow(kKdenlive);
        kd.setContext(timelineContext(1, true));
        e.handle(key(13));
        for (int i = 0; i < 8; ++i) {
            e.handle(turn(2, 1));
            QTest::qWait(5);  // each detent is its own batch
        }
        e.endAllGestures(false);
        QTRY_VERIFY(!kd.controlOptions.isEmpty() && kd.controlOptions.last().value(QStringLiteral("phase")).toString() == QStringLiteral("end"));
        QHash<QString, int> steps;  // gesture -> batches carrying a delta (updates and ends)
        double total = 0;
        for (int i = 0; i < kd.controlOptions.size(); ++i) {
            total += kd.controlDeltas.at(i);
            if (kd.controlDeltas.at(i) != 0) {
                ++steps[kd.controlOptions.at(i).value(QStringLiteral("gesture")).toString()];
            }
        }
        QCOMPARE(total, 8.0);  // nothing lost
        QVERIFY2(steps.size() >= 3, qPrintable(QString::number(steps.size())));
        for (auto it = steps.cbegin(); it != steps.cend(); ++it) {
            QVERIFY2(it.value() <= 3, qPrintable(it.key() + QLatin1Char('=') + QString::number(it.value())));
        }
    }

    // A multi-key (frame-bound) gesture is forgotten when the playhead moves;
    // a live-grading (whole-clip) gesture continues across playback ticks.
    void seekEndsFrameBoundGestureOnly()
    {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine e(&keys, &kd);
        e.setConfig(m_cfg);
        e.setActiveWindow(kKdenlive);
        auto paramContext = [](int position, bool liveGrading) {
            return QVariantMap{{QStringLiteral("epoch"), QVariant::fromValue<qulonglong>(1)},
                               {QStringLiteral("focus"), QStringLiteral("effectStack")},
                               {QStringLiteral("position"), position},
                               {QStringLiteral("param"), QVariantMap{{QStringLiteral("target"), QStringLiteral("par-level")}, {QStringLiteral("frame"), liveGrading ? -1 : position},
                                                                     {QStringLiteral("liveGrading"), liveGrading}}}};
        };
        kd.setContext(paramContext(0, false));
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 1);
        const QString first = kd.controlOptions.at(0).value(QStringLiteral("gesture")).toString();
        kd.setContext(paramContext(25, false));  // seek: the host ended it
        QCOMPARE(e.activeGestures(), 0);
        QCOMPARE(kd.controlOptions.size(), 1);  // nothing sent for it
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 2);
        QVERIFY(kd.controlOptions.at(1).value(QStringLiteral("gesture")).toString() != first);  // a fresh id
        e.endAllGestures(false);
        kd.setContext(paramContext(30, true));
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 4);
        for (int f = 31; f < 40; ++f) {
            kd.setContext(paramContext(f, true));  // playback ticks
        }
        QCOMPARE(e.activeGestures(), 1);
        e.handle(turn(1, 1));
        QTRY_COMPARE(kd.controlOptions.size(), 5);
        QCOMPARE(kd.controlOptions.at(4).value(QStringLiteral("gesture")).toString(), kd.controlOptions.at(3).value(QStringLiteral("gesture")).toString());
    }

    void provisionalFocusSendsAndTypesNothing()
    {
        for (const State st : {State::Available, State::Absent}) {
            RecordingKeySink keys;
            FakeKdenliveClient kd;
            kd.setState(st);
            Engine e(&keys, &kd);
            e.setConfig(optIn());
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
