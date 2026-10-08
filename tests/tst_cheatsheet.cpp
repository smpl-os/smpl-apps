// SPDX-License-Identifier: GPL-2.0-or-later
// The cheatsheet: binding labels, content per profile and Kdenlive layer,
// activity, visibility, auto-hide and change notification.
#include "bindinglabel.h"
#include "cheatsheet.h"
#include "config.h"
#include "engine.h"
#include "kdenliveclient.h"
#include "keysink.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QSignalSpy>
#include <QTest>
#include <linux/input-event-codes.h>

using namespace cs;
using State = KdenliveClient::State;

namespace {
const WindowInfo kKdenlive{QStringLiteral("org.kde.kdenlive"), QStringLiteral("Untitled - Kdenlive"), 4242, QStringLiteral("0x1")};
const WindowInfo kBrave{QStringLiteral("brave-browser"), QStringLiteral("x"), 77, QStringLiteral("0x2")};

const char *kConfig = R"({
  "cheatsheet": {"opacity": 0.7, "position": "bottom"},
  "profiles": [
    {"name": "Kdenlive", "match": {"class": "^org\\.kde\\.kdenlive"}, "kdenlive": true,
     "modes": {"liftAxis": ["value", "r", "g", "b"]},
     "layers": [
       {"name": "Wheels", "when": {"colorWheels": true}, "bindings": {
          "knob1": {"turn": {"control": "colorwheel.nudge", "options": {"wheel": "lift", "axis": "$liftAxis"}},
                    "press": {"cycle": "liftAxis", "label": "Lift axis"}},
          "key6": {"request": "colorwheel.reset", "params": {"wheel": "lift"}}}}
     ],
     "bindings": {
       "key1": {"action": "mark_in", "fallback": "i"},
       "key2": {"action": "razor_tool"},
       "key3": {"keys": "ctrl+z", "label": "Undo it"},
       "key4": "none",
       "knob1": {"turn": {"control": "playhead.jog"}, "press": {"action": "monitor_play"}}
     }},
    {"name": "Brave", "match": {"class": "^brave"}, "bindings": {
       "key1": "ctrl+t", "knob2.cw": {"mouse": "wheel-down"}, "knob2.ccw": {"mouse": "wheel-up"}}},
    {"name": "global", "bindings": {
       "key15": {"cheatsheet": "toggle"}, "key14": {"cheatsheet": "hold"},
       "key13": {"command": ["/usr/bin/notify-send", "hi"]},
       "knob3": {"ccw": "volumedown", "cw": "volumeup", "press": "mute"}}}
  ]})";

QJsonObject keyEntry(const QJsonObject &c, const QString &control)
{
    for (const auto &k : c.value(QStringLiteral("keys")).toArray()) {
        if (k.toObject().value(QStringLiteral("control")).toString() == control) {
            return k.toObject();
        }
    }
    return {};
}

QJsonObject knobEntry(const QJsonObject &c, const QString &control, const char *field)
{
    for (const auto &k : c.value(QStringLiteral("knobs")).toArray()) {
        if (k.toObject().value(QStringLiteral("control")).toString() == control) {
            return k.toObject().value(QLatin1String(field)).toObject();
        }
    }
    return {};
}

QString label(const QJsonObject &e)
{
    return e.value(QStringLiteral("label")).toString();
}
} // namespace

class TestCheatsheet : public QObject
{
    Q_OBJECT
    Config m_cfg;

    struct Rig {
        RecordingKeySink keys;
        FakeKdenliveClient kd;
        Engine engine{&keys, &kd};
        Cheatsheet sheet{&engine, &kd};
    };

private Q_SLOTS:
    void initTestCase()
    {
        QString err;
        auto c = parseConfig(kConfig, {}, &err);
        QVERIFY2(c, qPrintable(err));
        m_cfg = *c;
    }

    void labels()
    {
        QCOMPARE(prettyChord(KeyChord{Mod::Ctrl | Mod::Shift, KEY_F14}), QStringLiteral("Ctrl+Shift+F14"));
        QCOMPARE(prettyChord(KeyChord{Mod::Meta, KEY_A}), QStringLiteral("Super+A"));
        QCOMPARE(prettyKeyName(KEY_VOLUMEUP), QStringLiteral("Volume up"));
        QCOMPARE(prettyKeyName(KEY_PAGEDOWN), QStringLiteral("Page Down"));
        QCOMPARE(prettyKeyName(KEY_KP7), QStringLiteral("Num 7"));
        QCOMPARE(prettyKeyName(KEY_PLAYPAUSE), QStringLiteral("Play/Pause"));
        QCOMPARE(prettyKeyName(KEY_F24), QStringLiteral("F24"));
        QCOMPARE(prettyIdentifier(QStringLiteral("razor_tool")), QStringLiteral("Razor tool"));
        QCOMPARE(prettyIdentifier(QStringLiteral("liftAxis")), QStringLiteral("Lift axis"));
        QCOMPARE(prettyIdentifier(QStringLiteral("playhead.jog")), QStringLiteral("Playhead jog"));

        QHash<QString, QString> modes{{QStringLiteral("liftAxis"), QStringLiteral("r")}, {QStringLiteral("page"), QStringLiteral("trim")}};
        LabelEnv env;
        env.modeValue = [&modes](const QString &m) { return modes.value(m); };
        env.context = {{QStringLiteral("param"), QVariantMap{{QStringLiteral("name"), QStringLiteral("brightness_level")}}}};
        auto lbl = [&](const char *json) {
            QString err;
            const auto b = parseBinding(QJsonDocument::fromJson(QByteArray("[") + json + "]").array().at(0), &err);
            if (!b) {
                return QStringLiteral("PARSE ERROR ") + err;
            }
            return bindingLabel(*b, env);
        };
        QCOMPARE(lbl(R"("ctrl+z")"), QStringLiteral("Ctrl+Z"));
        QCOMPARE(lbl(R"({"keys": ["ctrl+k", "x"]})"), QStringLiteral("Ctrl+K X"));
        QCOMPARE(lbl(R"({"keys": "ctrl+z", "label": "Undo"})"), QStringLiteral("Undo"));
        QCOMPARE(lbl(R"({"mouse": "wheel-down"})"), QStringLiteral("Scroll down"));
        QCOMPARE(lbl(R"({"mouse": "back"})"), QStringLiteral("Back"));
        QCOMPARE(lbl(R"({"action": "mark_in"})"), QStringLiteral("Set Zone In"));            // catalog title
        QCOMPARE(lbl(R"({"action": "some_new_action"})"), QStringLiteral("Some new action"));
        QCOMPARE(lbl(R"({"control": "playhead.jog"})"), QStringLiteral("Jog"));
        QCOMPARE(lbl(R"({"control": "colorwheel.nudge", "options": {"wheel": "lift", "axis": "$liftAxis"}})"), QStringLiteral("Lift R"));
        QCOMPARE(lbl(R"({"control": "colorwheel.nudge", "options": {"wheel": "gain", "axis": "value", "step": "fine"}})"), QStringLiteral("Gain (fine)"));
        QCOMPARE(lbl(R"({"control": "param.nudge", "options": {"keyframe": "create"}})"), QStringLiteral("Brightness level (+key)"));
        QCOMPARE(lbl(R"({"control": "edit.trim", "options": {"edge": "end"}})"), QStringLiteral("Trim out"));
        QCOMPARE(lbl(R"({"control": "$page"})"), QStringLiteral("Trim"));  // a mode picks the control
        QCOMPARE(lbl(R"({"request": "colorwheel.reset", "params": {"wheel": "Gamma"}})"), QStringLiteral("Reset gamma"));
        QCOMPARE(lbl(R"({"request": "track.set", "params": {"what": "mute", "value": "$!ctx:timeline.track.muted"}})"), QStringLiteral("Mute track"));
        QCOMPARE(lbl(R"({"request": "param.reset"})"), QStringLiteral("Reset parameter"));
        QCOMPARE(lbl(R"({"cycle": "liftAxis"})"), QStringLiteral("Lift axis"));
        QCOMPARE(lbl(R"({"command": ["/usr/bin/notify-send", "x"]})"), QStringLiteral("Run notify-send"));
        QCOMPARE(lbl(R"({"cheatsheet": "toggle"})"), QStringLiteral("Cheatsheet"));
        QCOMPARE(lbl(R"("none")"), QString());
        Binding cyc;
        cyc.kind = Binding::Cycle;
        cyc.name = QStringLiteral("liftAxis");
        QCOMPARE(bindingState(cyc, env), QStringLiteral("r"));
        QCOMPARE(bindingKindName(Binding::Mouse), QStringLiteral("mouse"));
    }

    void contentPerProfile()
    {
        Rig r;
        r.engine.setConfig(m_cfg);
        r.engine.setActiveWindow(kBrave);
        QJsonObject c = r.sheet.content();
        QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));
        QCOMPARE(c.value(QStringLiteral("visible")).toBool(), false);
        QCOMPARE(c.value(QStringLiteral("keys")).toArray().size(), 15);
        QCOMPARE(c.value(QStringLiteral("knobs")).toArray().size(), 3);
        QCOMPARE(label(keyEntry(c, QStringLiteral("key1"))), QStringLiteral("Ctrl+T"));
        QCOMPARE(keyEntry(c, QStringLiteral("key1")).value(QStringLiteral("row")).toInt(), 0);
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob2"), "cw")), QStringLiteral("Scroll down"));
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob2"), "ccw")), QStringLiteral("Scroll up"));
        // Falls through to global where Brave binds nothing.
        const QJsonObject k15 = keyEntry(c, QStringLiteral("key15"));
        QCOMPARE(label(k15), QStringLiteral("Cheatsheet"));
        QCOMPARE(k15.value(QStringLiteral("profile")).toString(), QStringLiteral("global"));
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob3"), "press")), QStringLiteral("Mute"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key13"))), QStringLiteral("Run notify-send"));
        const QJsonObject k2 = keyEntry(c, QStringLiteral("key2"));
        QCOMPARE(k2.value(QStringLiteral("bound")).toBool(), false);
        QCOMPARE(k2.value(QStringLiteral("kind")).toString(), QStringLiteral("none"));
        QCOMPARE(c.value(QStringLiteral("options")).toObject().value(QStringLiteral("opacity")).toDouble(), 0.7);
        QCOMPARE(c.value(QStringLiteral("options")).toObject().value(QStringLiteral("position")).toString(), QStringLiteral("bottom"));
        QCOMPARE(c.value(QStringLiteral("window")).toObject().value(QStringLiteral("class")).toString(), kBrave.cls);
        QVERIFY(!c.contains(QStringLiteral("context")));

        // An unknown app: the global profile.
        r.engine.setActiveWindow(WindowInfo{QStringLiteral("gimp"), QString(), 9, QStringLiteral("0x9")});
        c = r.sheet.content();
        QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("global"));
        QCOMPARE(c.value(QStringLiteral("title")).toString(), QStringLiteral("global"));
        QCOMPARE(keyEntry(c, QStringLiteral("key1")).value(QStringLiteral("bound")).toBool(), false);

        // The layout follows the provider.
        r.sheet.setLayoutProvider([] { return *builtinBoardProfile(QStringLiteral("generic-3k1e")); });
        c = r.sheet.content();
        QCOMPARE(c.value(QStringLiteral("keys")).toArray().size(), 3);
        QCOMPARE(c.value(QStringLiteral("knobs")).toArray().size(), 1);
        QCOMPARE(c.value(QStringLiteral("layout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("generic-3k1e"));
    }

    void kdenliveLayersAndActivity()
    {
        Rig r;
        r.engine.setConfig(m_cfg);
        r.kd.setState(State::Available);
        r.kd.setContext({{QStringLiteral("focus"), QStringLiteral("timeline")}});
        r.engine.setActiveWindow(kKdenlive);
        QJsonObject c = r.sheet.content();
        QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("Kdenlive"));
        QCOMPARE(c.value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive"));
        QCOMPARE(c.value(QStringLiteral("context")).toObject().value(QStringLiteral("focus")).toString(), QStringLiteral("timeline"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key1"))), QStringLiteral("Set Zone In"));
        QCOMPARE(keyEntry(c, QStringLiteral("key1")).value(QStringLiteral("active")).toBool(), true);
        QCOMPARE(label(keyEntry(c, QStringLiteral("key2"))), QStringLiteral("Razor Tool"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key3"))), QStringLiteral("Undo it"));
        QCOMPARE(keyEntry(c, QStringLiteral("key3")).value(QStringLiteral("custom")).toBool(), true);
        QCOMPARE(keyEntry(c, QStringLiteral("key4")).value(QStringLiteral("bound")).toBool(), false);  // explicit none
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob1"), "cw")), QStringLiteral("Jog"));
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob1"), "press")), QStringLiteral("Play/Pause"));
        QCOMPARE(c.value(QStringLiteral("notice")).toString(), QString());

        // The colour wheels open: the layer takes over knob 1 and key 6.
        r.kd.setContext({{QStringLiteral("focus"), QStringLiteral("effectStack")}, {QStringLiteral("colorWheels"), true}});
        c = r.sheet.content();
        QCOMPARE(c.value(QStringLiteral("layers")).toArray(), QJsonArray{QStringLiteral("Wheels")});
        QCOMPARE(c.value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive · Wheels"));
        QJsonObject p = knobEntry(c, QStringLiteral("knob1"), "press");
        QCOMPARE(label(p), QStringLiteral("Lift axis"));
        QCOMPARE(p.value(QStringLiteral("state")).toString(), QStringLiteral("value"));
        QCOMPARE(p.value(QStringLiteral("layer")).toString(), QStringLiteral("Wheels"));
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob1"), "cw")), QStringLiteral("Lift"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key6"))), QStringLiteral("Reset lift"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key1"))), QStringLiteral("Set Zone In"));  // the base binding still shows
        // Cycling the axis changes the labels.
        r.engine.handle(PadEvent{QStringLiteral("knob1"), PadEvent::PressDown, 0, 0});
        r.engine.handle(PadEvent{QStringLiteral("knob1"), PadEvent::PressUp, 0, 0});
        c = r.sheet.content();
        QCOMPARE(knobEntry(c, QStringLiteral("knob1"), "press").value(QStringLiteral("state")).toString(), QStringLiteral("r"));
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob1"), "ccw")), QStringLiteral("Lift R"));

        // Capabilities: a control Kdenlive does not offer is inactive.
        r.kd.setControlCapabilities({QStringLiteral("playhead.jog")}, {});
        c = r.sheet.content();
        QCOMPARE(knobEntry(c, QStringLiteral("knob1"), "cw").value(QStringLiteral("active")).toBool(), false);
        QCOMPARE(keyEntry(c, QStringLiteral("key6")).value(QStringLiteral("active")).toBool(), false);  // no colorwheel.reset

        // Interface off and no keyFallback: everything Kdenlive is inactive, with a notice.
        r.kd.setState(State::Absent);
        c = r.sheet.content();
        QCOMPARE(keyEntry(c, QStringLiteral("key1")).value(QStringLiteral("active")).toBool(), false);
        QCOMPARE(keyEntry(c, QStringLiteral("key3")).value(QStringLiteral("active")).toBool(), true);  // plain keys still work
        QCOMPARE(c.value(QStringLiteral("notice")).toString(), Engine::absentNotice());
        QVERIFY(!c.contains(QStringLiteral("context")));
        // Not answered yet.
        r.kd.setState(State::Pending);
        QCOMPARE(r.sheet.content().value(QStringLiteral("notice")).toString(), QStringLiteral("Waiting for Kdenlive to answer"));
        // With keyFallback, actions with stock keys are typed: active.
        Config fb = m_cfg;
        fb.profiles[0].keyFallback = true;
        r.engine.setConfig(fb);
        r.kd.setState(State::Absent);
        c = r.sheet.content();
        QCOMPARE(keyEntry(c, QStringLiteral("key1")).value(QStringLiteral("active")).toBool(), true);   // fallback "i"
        QCOMPARE(keyEntry(c, QStringLiteral("key2")).value(QStringLiteral("active")).toBool(), false);  // no fallback
        QCOMPARE(c.value(QStringLiteral("notice")).toString(), QString());
    }

    void visibilityAndChanges()
    {
        Rig r;
        r.engine.setConfig(m_cfg);
        r.kd.setState(State::Available);
        r.kd.setContext({{QStringLiteral("focus"), QStringLiteral("timeline")}});
        r.engine.setActiveWindow(kKdenlive);
        QSignalSpy vis(&r.sheet, &Cheatsheet::visibilityChanged);
        QSignalSpy changed(&r.sheet, &Cheatsheet::changed);

        // Hidden: changes cost nothing and announce nothing.
        r.kd.setContext({{QStringLiteral("colorWheels"), true}});
        QTest::qWait(100);
        QCOMPARE(changed.count(), 0);

        // The toggle key (global key15, through the Kdenlive profile's fall-through).
        r.engine.handle(PadEvent{QStringLiteral("key15"), PadEvent::KeyDown, 0, 0});
        QVERIFY(r.sheet.isVisible());
        QCOMPARE(vis.count(), 1);
        QCOMPARE(changed.count(), 1);
        QCOMPARE(changed.last().at(0).toJsonObject().value(QStringLiteral("visible")).toBool(), true);
        QCOMPARE(changed.last().at(0).toJsonObject().value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive · Wheels"));

        // Playhead ticks change nothing shown: no notification.
        for (int i = 0; i < 10; ++i) {
            QVariantMap ctx{{QStringLiteral("colorWheels"), true}, {QStringLiteral("serial"), i}, {QStringLiteral("position"), i * 40}};
            r.kd.setContext(ctx);
        }
        QTest::qWait(150);
        QCOMPARE(changed.count(), 1);
        // The wheels close: one coalesced update.
        r.kd.setContext({{QStringLiteral("focus"), QStringLiteral("timeline")}});
        r.kd.setContext({{QStringLiteral("focus"), QStringLiteral("timeline")}, {QStringLiteral("serial"), 99}});
        QTRY_COMPARE(changed.count(), 2);
        QTest::qWait(100);
        QCOMPARE(changed.count(), 2);
        QCOMPARE(label(knobEntry(changed.last().at(0).toJsonObject(), QStringLiteral("knob1"), "cw")), QStringLiteral("Jog"));
        // Focus moves to Brave.
        r.engine.setActiveWindow(kBrave);
        QTRY_COMPARE(changed.count(), 3);
        QCOMPARE(changed.last().at(0).toJsonObject().value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));

        // Toggle again: hidden; a later change is not announced.
        r.engine.handle(PadEvent{QStringLiteral("key15"), PadEvent::KeyDown, 0, 0});
        QVERIFY(!r.sheet.isVisible());
        QCOMPARE(vis.count(), 2);
        QCOMPARE(vis.last().at(0).toBool(), false);
        r.engine.setActiveWindow(kKdenlive);
        QTest::qWait(100);
        QCOMPARE(changed.count(), 3);

        // Hold (global key14): shown while held.
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyDown, 0, 0});
        QVERIFY(r.sheet.isVisible());
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyUp, 0, 0});
        QVERIFY(!r.sheet.isVisible());
        QCOMPARE(vis.count(), 4);

        // request() as the D-Bus methods use it.
        r.sheet.request(QStringLiteral("toggle"));
        QVERIFY(r.sheet.isVisible());
        r.sheet.show();  // already shown: no second announcement
        QCOMPARE(vis.count(), 5);
        r.sheet.hide();
        r.sheet.hide();
        QCOMPARE(vis.count(), 6);
    }

    void autoHide()
    {
        Rig r;
        Config cfg = m_cfg;
        cfg.cheatsheet.autoHideMs = 250;
        r.engine.setConfig(cfg);
        r.engine.setActiveWindow(kBrave);
        r.sheet.toggle();
        QVERIFY(r.sheet.isVisible());
        // Pad input keeps it up.
        for (int i = 0; i < 4; ++i) {
            QTest::qWait(150);
            r.sheet.noteInput();
        }
        QVERIFY(r.sheet.isVisible());
        QTRY_VERIFY_WITH_TIMEOUT(!r.sheet.isVisible(), 1000);
        // Held: no auto-hide while the key is down.
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyDown, 0, 0});
        QTest::qWait(400);
        QVERIFY(r.sheet.isVisible());
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyUp, 0, 0});
        QVERIFY(!r.sheet.isVisible());
    }

    void autoHideDefault()
    {
        // Nobody gets stuck with it: unset, a sheet that nothing holds goes after 8 s.
        Rig r;
        Config cfg = m_cfg;
        cfg.cheatsheet.autoHideMs.reset();
        r.engine.setConfig(cfg);
        r.engine.setActiveWindow(kBrave);
        r.engine.handle(PadEvent{QStringLiteral("key15"), PadEvent::KeyDown, 0, 0});  // the toggle key
        r.engine.handle(PadEvent{QStringLiteral("key15"), PadEvent::KeyUp, 0, 0});
        QVERIFY(r.sheet.isVisible());
        QVERIFY(r.sheet.autoHideActive());
        QCOMPARE(r.sheet.autoHideIntervalMs(), 8000);
        r.sheet.hide();
        r.sheet.show();  // ShowCheatsheet / ToggleCheatsheet
        QVERIFY(r.sheet.autoHideActive());
        QCOMPARE(r.sheet.autoHideIntervalMs(), 8000);
        r.sheet.forceHide();
        QVERIFY(!r.sheet.autoHideActive());
        // A held "hold" key keeps it while held, without a timer.
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyDown, 0, 0});
        QVERIFY(r.sheet.isVisible());
        QVERIFY(!r.sheet.autoHideActive());
        r.engine.handle(PadEvent{QStringLiteral("key14"), PadEvent::KeyUp, 0, 0});
        QVERIFY(!r.sheet.isVisible());
        // Explicitly 0: until hidden.
        cfg.cheatsheet.autoHideMs = 0;
        r.engine.setConfig(cfg);
        r.sheet.toggle();
        QVERIFY(r.sheet.isVisible());
        QVERIFY(!r.sheet.autoHideActive());
        QCOMPARE(r.sheet.content().value(QStringLiteral("options")).toObject().value(QStringLiteral("autoHideMs")).toInt(), 0);
        cfg.cheatsheet.autoHideMs.reset();
        r.engine.setConfig(cfg);
        QCOMPARE(r.sheet.content().value(QStringLiteral("options")).toObject().value(QStringLiteral("autoHideMs")).toInt(), 8000);
    }

    void forceHideIsIdempotent()
    {
        Rig r;
        r.engine.setConfig(m_cfg);
        QSignalSpy visible(&r.sheet, &Cheatsheet::visibilityChanged);
        QSignalSpy forced(&r.sheet, &Cheatsheet::hideForced);
        r.sheet.forceHide();  // hidden already: still tells the renderer
        r.sheet.forceHide();
        QCOMPARE(forced.count(), 2);
        QCOMPARE(visible.count(), 0);
        r.sheet.show();
        r.sheet.forceHide();
        QVERIFY(!r.sheet.isVisible());
        QCOMPARE(visible.count(), 2);
        QCOMPARE(forced.count(), 3);
    }

    void previews()
    {
        // Any window and context, from a config alone; Kdenlive assumed to answer.
        QJsonObject c = Cheatsheet::preview(m_cfg, *builtinBoardProfile(QStringLiteral("sy181-15k3e")), kKdenlive.cls, QString(),
                                            {{QStringLiteral("colorWheels"), true}});
        QCOMPARE(c.value(QStringLiteral("preview")).toBool(), true);
        QCOMPARE(c.value(QStringLiteral("title")).toString(), QStringLiteral("Kdenlive · Wheels"));
        QCOMPARE(label(keyEntry(c, QStringLiteral("key6"))), QStringLiteral("Reset lift"));
        QCOMPARE(keyEntry(c, QStringLiteral("key6")).value(QStringLiteral("active")).toBool(), true);
        c = Cheatsheet::preview(m_cfg, *builtinBoardProfile(QStringLiteral("sy181-15k3e")), kKdenlive.cls, QString(), {});
        QCOMPARE(label(knobEntry(c, QStringLiteral("knob1"), "cw")), QStringLiteral("Jog"));
        QCOMPARE(c.value(QStringLiteral("notice")).toString(), QString());
        c = Cheatsheet::preview(m_cfg, *builtinBoardProfile(QStringLiteral("generic-16k3e")), QStringLiteral("brave-browser"), QString(), {});
        QCOMPARE(c.value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));
        QCOMPARE(c.value(QStringLiteral("keys")).toArray().size(), 16);

        // A preview does not disturb the live sheet.
        Rig r;
        r.engine.setConfig(m_cfg);
        r.engine.setActiveWindow(kBrave);
        QSignalSpy vis(&r.sheet, &Cheatsheet::visibilityChanged);
        const QJsonObject p = r.sheet.previewFor(kKdenlive.cls, QString(), {});
        QCOMPARE(p.value(QStringLiteral("profile")).toString(), QStringLiteral("Kdenlive"));
        QCOMPARE(r.sheet.content().value(QStringLiteral("profile")).toString(), QStringLiteral("Brave"));
        QCOMPARE(vis.count(), 0);
    }
};

QTEST_GUILESS_MAIN(TestCheatsheet)
#include "tst_cheatsheet.moc"
