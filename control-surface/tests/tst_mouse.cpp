// SPDX-License-Identifier: GPL-2.0-or-later
#include "kdenlivecontract.h"
#include "mapping.h"
#include "mockkdenlive.h"
#include "service.h"
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDBusPendingCallWatcher>
#include <QDBusPendingReply>
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QTemporaryDir>
#include <QTest>
#include <linux/input.h>

using namespace cs;
using namespace cs::mouse;

class MouseTest : public QObject {
    Q_OBJECT
private Q_SLOTS:
    void profilesAndPreservation();
    void invalidConfigurations_data();
    void invalidConfigurations();
    void heldReferenceCountsAndCleanup();
    void normalInputAndIdentify();
    void repeatsCommandsAndWheel();
    void motionScrollAndUnmappedPassthrough();
    void learningPreservesConfigurationAndBackups();
    void learningRefusesInvalidChanges();
    void dbusReadinessAndFailureContract();
    void mouseUsesSharedKdenliveDispatcher();
    void focusLossUsesGlobalEvenForWildcardProfiles();
    void identifyCancelsSharedDelayedSequences();
    void identifySuppressesCompositeKeyboardAndExtraButtons();
    void wheelDirectionsIdentifyWithoutScrolling();
    void mappedWheelHighResolutionPairsOnlyDispatchOnce();
    void guidedCalibrationCapturesAllTwelveAndReplacesAtomically();
    void fullSetupCapturesKeysAndWheelDirectionsAfterArming();
    void fullSetupRetrySkipCancelAndConcurrentEdits();
};

void MouseTest::profilesAndPreservation() {
    QString error;
    auto config = Mapping::parse(R"({
          "version":1, "unknown":"retained by Settings", "devices":{
            "generic":{"profiles":"not our device"},
            "mmo-12":{"learned":{"grid-1":{"source":"input1/keyboard","code":2}},
            "profiles":[
              {"name":"Browser","match":{"class":"^firefox$"},"bindings":{"grid-1":"alt+left","grid-2":"none"}},
              {"name":"Editor","match":{"title":"document"},"fallthrough":false,"bindings":{}},
              {"name":"global","bindings":{"grid-1":"ctrl+c","grid-3":{"command":["echo","hi"]}}}]}}})",
                                 &error);
    QVERIFY2(config, qPrintable(error));
    QCOMPARE(config->slot({"input1/keyboard", KEY_1}, false), QString("grid-1"));
    QVERIFY(config->slot({"input2/keyboard", KEY_1}, false).isEmpty());
    QVERIFY(config->slot({"input1/keyboard", KEY_2}, false).isEmpty());
    QCOMPARE(config->slot({"input0/pointer", BTN_SIDE}, true), QString("btn-back"));
    QCOMPARE(config->binding("grid-1", {"firefox", "", 0, ""}, true)->keys.first().mods, Mod::Alt);
    QCOMPARE(config->binding("grid-1", {"firefox", "", 0, ""}, false)->keys.first().mods, Mod::Ctrl);
    QCOMPARE(config->binding("grid-2", {"firefox", "", 0, ""}, true)->kind, Binding::None);
    QCOMPARE(config->binding("grid-3", {"firefox", "", 0, ""}, true)->kind, Binding::Command);
    QVERIFY(!config->binding("grid-1", {"editor", "document", 0, ""}, true));
    QCOMPARE(config->unlearned(), (QStringList{"grid-2", "grid-3"}));
}

void MouseTest::invalidConfigurations_data() {
    QTest::addColumn<QByteArray>("device");
    QTest::newRow("profiles type") << QByteArray(R"({"profiles":5})");
    QTest::newRow("bad regex") << QByteArray(R"({"profiles":[{"match":{"class":"["}}]})");
    QTest::newRow("unknown match") << QByteArray(R"({"profiles":[{"match":{"app":"firefox"}}]})");
    QTest::newRow("bad shortcut") << QByteArray(R"({"profiles":[{"bindings":{"grid-1":"ctrl+not-a-key"}}]})");
    QTest::newRow("unsafe primary") << QByteArray(R"({"profiles":[{"bindings":{"btn-left":"none"}}]})");
    QTest::newRow("unsafe primary wheel") << QByteArray(R"({"profiles":[{"bindings":{"btn-right":{"mouse":"wheel-up"}}}]})");
    QTest::newRow("unsupported API") << QByteArray(R"({"profiles":[{"bindings":{"grid-1":{"action":"edit_undo"}}}]})");
    QTest::newRow("nested API outside app") << QByteArray(
        R"({"profiles":[{"bindings":{"grid-1":{"do":[{"action":"edit_undo","fallback":"ctrl+z"}]}}}]})");
    QTest::newRow("layer API outside app") << QByteArray(
        R"({"profiles":[{"layers":[{"name":"edit","when":{"$mode":"edit"},"bindings":{"grid-1":{"action":"edit_undo"}}}]}]})");
    QTest::newRow("API global") << QByteArray(R"({"profiles":[{"kdenlive":true,"bindings":{"grid-1":{"action":"edit_undo"}}}]})");
    QTest::newRow("API fallback") << QByteArray(R"({"profiles":[{"match":{"class":"kdenlive"},"kdenlive":true,"keyFallback":true}]})");
    QTest::newRow("keypad held layer") << QByteArray(
        R"({"profiles":[{"layers":[{"name":"held","when":{"held":"key1"},"bindings":{"grid-1":"a"}}]}]})");
    QTest::newRow("keypad cheatsheet") << QByteArray(R"({"profiles":[{"bindings":{"grid-1":{"cheatsheet":"toggle"}}}]})");
    QTest::newRow("layer keypad shorthand") << QByteArray(
        R"({"profiles":[{"layers":[{"name":"bad","when":{"$mode":"a"},"bindings":{"knob1":{"turn":"a"}}}]}]})");
    QTest::newRow("unknown slot") << QByteArray(R"({"profiles":[{"bindings":{"thumb-99":"a"}}]})");
    QTest::newRow("duplicate learning") << QByteArray(
        R"({"learned":{"grid-1":{"source":"input1/keyboard","code":2},"grid-2":{"source":"input1/keyboard","code":2}}})");
    QTest::newRow("fractional learning") << QByteArray(R"({"learned":{"grid-1":{"source":"input1/keyboard","code":2.5}}})");
    QTest::newRow("wrong primary identity") << QByteArray(R"({"learned":{"btn-left":{"source":"input0/pointer","code":273}}})");
    QTest::newRow("primary as side key") << QByteArray(R"({"learned":{"grid-1":{"source":"input0/pointer","code":272}}})");
    QTest::newRow("wrong relative direction") << QByteArray(R"({"learned":{"wheel-up":{"source":"input0/pointer","type":2,"code":8,"direction":-1}}})");
    QTest::newRow("fractional relative direction") << QByteArray(R"({"learned":{"wheel-up":{"source":"input0/pointer","type":2,"code":8,"direction":1.5}}})");
    QTest::newRow("relative as side key") << QByteArray(R"({"learned":{"grid-1":{"source":"input0/pointer","type":2,"code":8,"direction":1}}})");
}

void MouseTest::invalidConfigurations() {
    QFETCH(QByteArray, device);
    QString error;
    QVERIFY(!Mapping::parse("{\"devices\":{\"mmo-12\":" + device + "}}", &error));
    QVERIFY(!error.isEmpty());
}

void MouseTest::heldReferenceCountsAndCleanup() {
    QString error;
    Router router;
    router.setMapping(*Mapping::parse(R"({"devices":{"mmo-12":{
            "learned":{"grid-1":{"source":"input1/keyboard","code":2},"grid-2":{"source":"input1/keyboard","code":3}},
            "profiles":[{"bindings":{"grid-1":"ctrl+c","grid-2":"ctrl+c"}}]}}})",
                                      &error));
    QList<InputTriple> events;
    router.output = [&](bool pointer, const InputTriple &event) {
        QVERIFY(!pointer);
        if (event.type != EV_SYN)
            events << event;
    };
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_2}, false, 1));
    QCOMPARE(events, (QList<InputTriple>{{EV_KEY, KEY_LEFTCTRL, 1}, {EV_KEY, KEY_C, 1}}));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QCOMPARE(events.size(), 2);
    router.releaseAll();
    QCOMPARE(events, (QList<InputTriple>{{EV_KEY, KEY_LEFTCTRL, 1}, {EV_KEY, KEY_C, 1}, {EV_KEY, KEY_C, 0}, {EV_KEY, KEY_LEFTCTRL, 0}}));
    // Reload/focus/mode switches cannot leak an unmatched key-up.
    QVERIFY(router.key({"input1/keyboard", KEY_2}, false, 0));
    QCOMPARE(events.size(), 4);
    router.disconnected();
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    router.disconnected();
    QCOMPARE(events.size(), 8);
}

void MouseTest::normalInputAndIdentify() {
    QString error;
    Router router;
    router.setMapping(*Mapping::parse(R"({"devices":{"mmo-12":{"profiles":[{"bindings":{
            "btn-left":{"mouse":"right"},"btn-back":{"command":["never","run"]}}}]}}})",
                                      &error));
    QList<InputTriple> events;
    int launches = 0;
    router.output = [&](bool, const InputTriple &event) { events << event; };
    router.command = [&](const Binding &) { ++launches; };
    QString selected;
    router.pressed = [&](const QString &slot, const SourceKey &) { selected = slot; };
    QVERIFY(!router.key({"input1/keyboard", KEY_A}, false, 1));
    QVERIFY(!router.key({"input1/keyboard", KEY_A}, false, 2));
    QVERIFY(!router.key({"input1/keyboard", KEY_A}, false, 0));
    QVERIFY(router.key({"input0/pointer", BTN_LEFT}, true, 1));
    QCOMPARE(events.first(), (InputTriple{EV_KEY, BTN_RIGHT, 1}));
    QVERIFY(router.key({"input0/pointer", BTN_LEFT}, true, 0));
    QCOMPARE(events[2], (InputTriple{EV_KEY, BTN_RIGHT, 0}));
    events.clear();
    router.identify = true;
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QCOMPARE(selected, QString("btn-back"));
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
    QCOMPARE(launches, 0);
    QVERIFY(events.isEmpty());
    QVERIFY(router.key({"input1/keyboard", KEY_A}, false, 1));
    QVERIFY(selected.isEmpty()); // Unknown codes are never guessed to be a grid button.
    router.identify = false;
    QVERIFY(router.key({"input1/keyboard", KEY_A}, false, 0));
    QVERIFY(!router.key({"input0/pointer", BTN_LEFT}, true, 0));
}

void MouseTest::repeatsCommandsAndWheel() {
    QString error;
    Router router;
    router.setMapping(*Mapping::parse(R"({"devices":{"mmo-12":{"profiles":[{"bindings":{
          "btn-middle":"ctrl+v","btn-back":{"command":["echo","ok"]},"btn-forward":{"mouse":"wheel-up"}}}]}}})",
                                      &error));
    QList<InputTriple> events;
    router.output = [&](bool, const InputTriple &event) {
        if (event.type != EV_SYN)
            events << event;
    };
    int launches = 0;
    router.command = [&](const Binding &) { ++launches; };
    QVERIFY(router.key({"input0/pointer", BTN_MIDDLE}, true, 1));
    QVERIFY(router.key({"input0/pointer", BTN_MIDDLE}, true, 2));
    QCOMPARE(events.last(), (InputTriple{EV_KEY, KEY_V, 2}));
    QVERIFY(router.key({"input0/pointer", BTN_MIDDLE}, true, 0));
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 2));
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
    QCOMPARE(launches, 1);
    QVERIFY(router.key({"input0/pointer", BTN_EXTRA}, true, 1));
    QCOMPARE(events.last(), (InputTriple{EV_REL, REL_WHEEL, 1}));
    QVERIFY(router.key({"input0/pointer", BTN_EXTRA}, true, 0));
}

void MouseTest::motionScrollAndUnmappedPassthrough() {
    Router router;
    router.output = [](bool, const InputTriple &) { QFAIL("Unmapped input must not generate an action"); };
    for (bool identify : {false, true}) {
        router.identify = identify;
        for (const auto &event : QList<InputTriple>{{EV_REL, REL_X, 123},
                                                    {EV_REL, REL_Y, -42},
                                                    {EV_REL, REL_WHEEL, -1},
                                                    {EV_REL, REL_HWHEEL, 1},
                                                    {EV_REL, REL_WHEEL_HI_RES, 120},
                                                    {EV_REL, REL_HWHEEL_HI_RES, -120},
                                                    {EV_SYN, SYN_REPORT, 0},
                                                    {EV_ABS, ABS_X, 1234},
                                                    {EV_KEY, BTN_LEFT, 1},
                                                    {EV_KEY, BTN_LEFT, 0}})
            QCOMPARE(router.consume("input0/pointer", true, event),
                     identify && event.type == EV_REL &&
                         (event.code == REL_WHEEL || event.code == REL_HWHEEL || event.code == REL_WHEEL_HI_RES ||
                          event.code == REL_HWHEEL_HI_RES));
        QCOMPARE(router.consume("input1/keyboard", false, {EV_KEY, KEY_A, 1}), identify);
        QCOMPARE(router.consume("input1/keyboard", false, {EV_KEY, KEY_A, 0}), identify);
    }
}

void MouseTest::identifySuppressesCompositeKeyboardAndExtraButtons() {
    Router router;
    QVERIFY(!router.key({"input1/pointer", KEY_A}, true, 1));
    router.identify = true;
    QVERIFY(router.key({"input1/pointer", KEY_A}, true, 2));
    QVERIFY(!router.key({"input1/pointer", KEY_A}, true, 0)); // Release a press that already reached the desktop.
    router.output = [](bool, const InputTriple &) { QFAIL("Identify must not generate mapped actions"); };
    QString selected;
    router.pressed = [&](const QString &slot, const SourceKey &) { selected = slot; };
    for (bool pointer : {false, true}) {
        for (int code : {KEY_1, KEY_A, KEY_LEFTCTRL, BTN_SIDE, BTN_EXTRA, BTN_FORWARD, BTN_BACK, BTN_TASK}) {
            QVERIFY(router.consume("input1/mixed", pointer, {EV_KEY, code, 1}));
            QVERIFY(router.consume("input1/mixed", pointer, {EV_KEY, code, 2}));
            if (code == BTN_SIDE || code == BTN_BACK)
                QCOMPARE(selected, QStringLiteral("btn-back"));
            if (code == BTN_EXTRA || code == BTN_FORWARD)
                QCOMPARE(selected, QStringLiteral("btn-forward"));
            router.releaseAll();
            router.identify = false;
            QVERIFY(router.consume("input1/mixed", pointer, {EV_KEY, code, 0}));
            router.identify = true;
        }
        for (int code : {BTN_LEFT, BTN_RIGHT, BTN_MIDDLE}) {
            QVERIFY(!router.consume("input1/mixed", pointer, {EV_KEY, code, 1}));
            QVERIFY(!router.consume("input1/mixed", pointer, {EV_KEY, code, 0}));
        }
    }
}

void MouseTest::wheelDirectionsIdentifyWithoutScrolling() {
    Router router;
    router.identify = true;
    QStringList selected;
    router.scrolled = [&](const QString &slot, const QString &, const InputTriple &) { selected << slot; };
    for (bool pointer : {false, true}) {
        const QString source = pointer ? "input0/pointer" : "input1/keyboard";
        for (int sign : {-1, 1}) {
            for (const auto &axes : {qMakePair(REL_WHEEL, REL_WHEEL_HI_RES), qMakePair(REL_HWHEEL, REL_HWHEEL_HI_RES)}) {
                const auto count = selected.size();
                QVERIFY(router.consume(source, pointer, {EV_REL, axes.second, sign * 15}));
                QVERIFY(router.consume(source, pointer, {EV_REL, axes.first, sign}));
                QCOMPARE(selected.size(), count + 1); // Legacy + high-res are one physical direction.
                QCOMPARE(selected.last(), Mapping::wheelSlot(axes.first, sign));
                QVERIFY(!router.consume(source, pointer, {EV_REL, REL_X, 7}));
                QVERIFY(!router.consume(source, pointer, {EV_SYN, SYN_REPORT, 0}));
                QVERIFY(router.consume(source, pointer, {EV_REL, axes.first, sign}));
                QCOMPARE(selected.size(), count + 2);
                router.consume(source, pointer, {EV_SYN, SYN_REPORT, 0});
            }
        }
    }
    router.identify = false;
    for (int code : {REL_WHEEL, REL_HWHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES})
        QVERIFY(!router.consume("input0/pointer", true, {EV_REL, code, 120}));
    QVERIFY(!router.consume("input1/keyboard", false, {EV_REL, REL_DIAL, 1}));
    router.identify = true;
    QVERIFY(router.consume("input1/keyboard", false, {EV_REL, REL_DIAL, -1}));
    QVERIFY(selected.last().isEmpty()); // Unknown directions are observable, not assigned to a guessed position.
}

void MouseTest::mappedWheelHighResolutionPairsOnlyDispatchOnce() {
    QString error;
    const auto mapping = Mapping::parse(R"({"devices":{"mmo-12":{"profiles":[{"bindings":{
      "wheel-up":{"command":["up"]},"wheel-down":{"command":["down"]},
      "wheel-tilt-left":{"command":["left"]},"wheel-tilt-right":{"command":["right"]}}}]}}})",
                                        &error);
    QVERIFY2(mapping, qPrintable(error));
    Router router;
    router.setMapping(*mapping);
    QStringList commands;
    router.command = [&](const Binding &binding) { commands << binding.argv.first(); };
    for (bool highFirst : {true, false}) {
        for (const auto &axes : {qMakePair(REL_WHEEL, REL_WHEEL_HI_RES), qMakePair(REL_HWHEEL, REL_HWHEEL_HI_RES)}) {
            for (int sign : {-1, 1}) {
                const auto before = commands.size();
                const InputTriple legacy{EV_REL, axes.first, sign}, high{EV_REL, axes.second, 120 * sign};
                QVERIFY(router.consume("input1/keyboard", false, highFirst ? high : legacy));
                QVERIFY(router.consume("input1/keyboard", false, highFirst ? legacy : high));
                router.consume("input1/keyboard", false, {EV_SYN, SYN_REPORT, 0});
                QCOMPARE(commands.size(), before + 1);
            }
        }
    }
    const auto before = commands.size();
    for (int i = 0; i < 8; ++i) {
        QVERIFY(router.consume("input0/pointer", true, {EV_REL, REL_WHEEL_HI_RES, 15}));
        router.consume("input0/pointer", true, {EV_SYN, SYN_REPORT, 0});
        QCOMPARE(commands.size(), before + (i == 7 ? 1 : 0));
    }
    router.consume("input0/pointer", true, {EV_REL, REL_WHEEL_HI_RES, 60});
    router.releaseAll(); // No residual fraction or pending wheel action may escape Identify entry.
    router.identify = true;
    router.consume("input0/pointer", true, {EV_SYN, SYN_REPORT, 0});
    QCOMPARE(commands.size(), before + 1);
}

void MouseTest::guidedCalibrationCapturesAllTwelveAndReplacesAtomically() {
    QTemporaryDir dir;
    const QString path = dir.filePath("mouse.json");
    QString error;
    QVERIFY(saveLearnedButton(path, "grid-1", {"old/keyboard", KEY_F12}, &error));
    GridCalibration calibration;
    calibration.start();
    QVERIFY(!calibration.capture({"input1/pointer", BTN_LEFT}, &error));
    QVERIFY(!calibration.capture({"input1/pointer", KEY_LEFTCTRL}, &error));
    Router router;
    router.identify = true;
    router.pressed = [&](const QString &, const SourceKey &key) { QVERIFY2(calibration.capture(key, &error), qPrintable(error)); };
    for (int i = 1; i <= 12; ++i) {
        QCOMPARE(calibration.target(), QStringLiteral("grid-%1").arg(i));
        // Arbitrary non-factory codes prove this is captured calibration, not a guessed table.
        QVERIFY(router.key({"input1/pointer", KEY_F1 + i - 1}, true, 1));
        QVERIFY(router.key({"input1/pointer", KEY_F1 + i - 1}, true, 0));
        if (i == 1)
            QVERIFY(!calibration.capture({"input1/pointer", KEY_F1}, &error));
    }
    QVERIFY(calibration.complete());
    QVERIFY2(saveLearnedButtons(path, calibration.inputs(), &error), qPrintable(error));
    QFile file(path);
    QVERIFY(file.open(QIODevice::ReadOnly));
    const auto mapping = Mapping::parse(file.readAll(), &error);
    QVERIFY2(mapping, qPrintable(error));
    QCOMPARE(mapping->learned.size(), 12);
    router.setMapping(*mapping);
    router.pressed = [&](const QString &slot, const SourceKey &key) {
        QCOMPARE(slot, QStringLiteral("grid-%1").arg(key.code - KEY_F1 + 1));
    };
    for (int i = 0; i < 12; ++i) {
        QVERIFY(router.key({"input1/pointer", KEY_F1 + i}, true, 1));
        QVERIFY(router.key({"input1/pointer", KEY_F1 + i}, true, 0));
    }
    calibration.start();
    QVERIFY(calibration.capture({"input1/keyboard", KEY_A}, &error));
    calibration.cancel();
    QVERIFY(!calibration.active());
    QVERIFY(calibration.inputs().isEmpty());
    QCOMPARE(mapping->learned.size(), 12);
}

void MouseTest::fullSetupCapturesKeysAndWheelDirectionsAfterArming() {
    QTemporaryDir dir;
    const auto path = dir.filePath("mouse.json");
    SetupFlow setup;
    setup.start();
    QCOMPARE(SetupFlow::steps().size(), 22);
    Router router;
    router.identify = true;
    std::optional<SourceKey> pending;
    QString error;
    int captures = 0;
    router.pressed = [&](const QString &, const SourceKey &key) {
        if (setup.armed())
            pending = key;
    };
    router.released = [&](const SourceKey &key) {
        if (pending && *pending == key) {
            QVERIFY2(setup.capture(key, &error), qPrintable(error));
            pending.reset();
            ++captures;
        }
    };
    router.scrolled = [&](const QString &, const QString &source, const InputTriple &event) {
        if (setup.armed()) {
            QVERIFY2(setup.capture(SourceKey::relative(source, event.code, event.value), &error), qPrintable(error));
            ++captures;
        }
    };
    for (int index = 0; index < SetupFlow::steps().size(); ++index) {
        const auto slot = setup.target();
        // Ordinary navigation clicks before arming cannot become captures.
        QVERIFY(!router.key({"input0/pointer", BTN_LEFT}, true, 1));
        QVERIFY(!router.key({"input0/pointer", BTN_LEFT}, true, 0));
        QVERIFY(!setup.captured());
        QCOMPARE(captures, index);
        QVERIFY2(setup.arm(&error), qPrintable(error));
        if (slot.startsWith("wheel-")) {
            const bool horizontal = slot.startsWith("wheel-tilt-");
            const int code = horizontal ? REL_HWHEEL : REL_WHEEL;
            const int direction = slot == "wheel-up" || slot == "wheel-tilt-right" ? 1 : -1;
            QVERIFY(router.consume("input1/keyboard", false, {EV_REL, horizontal ? REL_HWHEEL_HI_RES : REL_WHEEL_HI_RES, direction * 120}));
            QVERIFY(router.consume("input1/keyboard", false, {EV_REL, code, direction}));
            router.consume("input1/keyboard", false, {EV_SYN, SYN_REPORT, 0});
        } else {
            const int code = slot == "btn-left" ? BTN_LEFT : slot == "btn-right" ? BTN_RIGHT : slot == "btn-middle" ? BTN_MIDDLE :
                             slot.startsWith("grid-") ? KEY_F1 + slot.mid(5).toInt() - 1 : KEY_PROG1 + index - 19;
            const bool primary = index < 3;
            QCOMPARE(router.key({"input0/pointer", code}, true, 1), !primary);
            QVERIFY(!setup.captured()); // A held button is not confirmed until release.
            QCOMPARE(router.key({"input0/pointer", code}, true, 0), !primary);
        }
        QCOMPARE(captures, index + 1);
        QVERIFY(setup.captured());
        QVERIFY(!setup.armed());
        QVERIFY(!QFile::exists(path)); // Captures are staged, not incremental writes.
        QVERIFY2(setup.next(false, &error), qPrintable(error));
    }
    QVERIFY(setup.review());
    QVERIFY2(saveLearnedButtons(path, setup.inputs(), &error), qPrintable(error));
    QFile file(path);
    QVERIFY(file.open(QIODevice::ReadOnly));
    const auto mapping = Mapping::parse(file.readAll(), &error);
    QVERIFY2(mapping, qPrintable(error));
    QCOMPARE(mapping->learned.size(), 22);
    QCOMPARE(mapping->slot({"input0/pointer", BTN_LEFT}, true), QString("btn-left"));
    QCOMPARE(mapping->slot({"input0/pointer", KEY_F1 + 11}, true), QString("grid-12"));
    QCOMPARE(mapping->relativeSlot("input1/keyboard", REL_HWHEEL_HI_RES, -120), QString("wheel-tilt-left"));
    QCOMPARE(mapping->relativeSlot("input1/keyboard", REL_WHEEL, 1), QString("wheel-up"));
    QCOMPARE(mapping->relativeSlot("input1/keyboard", REL_WHEEL, -1), QString("wheel-down"));
    QCOMPARE(mapping->slot({"input0/pointer", KEY_PROG1}, true), QString("btn-dpi-up"));
    QCOMPARE(mapping->slot({"input0/pointer", KEY_PROG1 + 1}, true), QString("btn-dpi-down"));
    QCOMPARE(mapping->slot({"input0/pointer", KEY_PROG1 + 2}, true), QString("btn-scroll-mode"));
    for (auto input = setup.inputs().begin(); input != setup.inputs().end(); ++input) {
        if (input->type == EV_KEY)
            QCOMPARE(mapping->slot(*input, true), input.key());
        else
            QCOMPARE(mapping->relativeSlot(input->source, input->code, input->direction), input.key());
    }
}

void MouseTest::fullSetupRetrySkipCancelAndConcurrentEdits() {
    SetupFlow setup;
    setup.start();
    QString error;
    QVERIFY(!setup.capture({"input0/pointer", BTN_LEFT}, &error));
    QVERIFY(!setup.next(false, &error));
    QVERIFY(setup.arm(&error));
    QVERIFY(!setup.capture({"input0/pointer", BTN_RIGHT}, &error));
    QVERIFY(setup.capture({"input0/pointer", BTN_LEFT}, &error));
    setup.retry();
    QVERIFY(!setup.captured());
    QVERIFY(!setup.armed());
    QVERIFY(setup.arm(&error));
    QVERIFY(setup.capture({"input0/pointer", BTN_LEFT}, &error));
    QVERIFY(setup.next(false, &error));
    while (setup.target() != "grid-1")
        QVERIFY(setup.next(true, &error));
    QVERIFY(setup.arm(&error));
    QVERIFY(setup.capture({"input2/keyboard", KEY_A}, &error));
    QVERIFY(setup.next(false, &error));
    QVERIFY(setup.arm(&error));
    QVERIFY(!setup.capture({"input2/keyboard", KEY_A}, &error));
    while (!setup.review())
        QVERIFY(setup.next(true, &error));
    QCOMPARE(setup.inputs().size(), 2);
    QVERIFY(!setup.arm(&error));

    QTemporaryDir dir;
    const auto path = dir.filePath("mouse.json");
    const QByteArray before = R"({"note":"preserve","devices":{"mmo-12":{"learned":{
        "grid-12":{"source":"old/keyboard","code":44}},"profiles":[{"bindings":{"grid-1":"ctrl+c"}}]}}})";
    QFile file(path);
    QVERIFY(file.open(QIODevice::WriteOnly));
    file.write(before);
    file.close();
    const QByteArray stale = "{}";
    QVERIFY(!saveLearnedButtons(path, setup.inputs(), &error, &stale));
    QVERIFY(error.contains("changed during setup"));
    QVERIFY2(saveLearnedButtons(path, setup.inputs(), &error, &before), qPrintable(error));
    QVERIFY(file.open(QIODevice::ReadOnly));
    const auto saved = file.readAll();
    file.close();
    const auto mapping = Mapping::parse(saved, &error);
    QVERIFY2(mapping, qPrintable(error));
    QCOMPARE(mapping->slot({"old/keyboard", KEY_Z}, false), QString("grid-12"));
    QVERIFY(saved.contains("preserve"));
    QCOMPARE(QDir(dir.filePath("backups")).entryList(QDir::Files).size(), 1);
    setup.cancel();
    QVERIFY(!setup.active());
    QVERIFY(setup.inputs().isEmpty());
    QVERIFY(file.open(QIODevice::ReadOnly));
    QCOMPARE(file.readAll(), saved);
}

void MouseTest::learningPreservesConfigurationAndBackups() {
    QTemporaryDir dir;
    QVERIFY(dir.isValid());
    const QString path = dir.filePath("mouse.json");
    const QByteArray before = R"({
          // this exact original text must survive in the backup
          "version":1, "note":"mine",
          "devices":{"generic":{"profiles":[{"bindings":{"btn-back":"alt+left"}}]},
            "mmo-12":{"dpi":1600,"profiles":[{"name":"global","bindings":{"grid-1":"ctrl+c"}}]}}})";
    QFile file(path);
    QVERIFY(file.open(QIODevice::WriteOnly));
    QCOMPARE(file.write(before), before.size());
    file.close();
    QVERIFY(file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner));
    const QString link = dir.filePath("linked.json");
    QVERIFY(QFile::link(path, link));
    QString error;
    QVERIFY2(saveLearnedButton(link, "grid-1", {"input1/keyboard", KEY_1}, &error), qPrintable(error));
    QVERIFY(QFileInfo(link).isSymbolicLink());
    QVERIFY(file.open(QIODevice::ReadOnly));
    const auto after = QJsonDocument::fromJson(file.readAll()).object();
    file.close();
    QCOMPARE(after.value("note").toString(), QString("mine"));
    const auto devices = after.value("devices").toObject();
    QCOMPARE(devices.value("generic").toObject().value("profiles").toArray().size(), 1);
    const auto mouse = devices.value("mmo-12").toObject();
    QCOMPARE(mouse.value("dpi").toInt(), 1600);
    QCOMPARE(mouse.value("profiles").toArray().first().toObject().value("bindings").toObject().value("grid-1").toString(),
             QString("ctrl+c"));
    QCOMPARE(mouse.value("learned").toObject().value("grid-1").toObject().value("code").toInt(), KEY_1);
    QCOMPARE(file.permissions() & (QFileDevice::ReadGroup | QFileDevice::ReadOther), QFileDevice::Permissions{});
    const auto backups = QDir(dir.filePath("backups")).entryList(QDir::Files);
    QCOMPARE(backups.size(), 1);
    QFile backup(dir.filePath("backups/" + backups.first()));
    QVERIFY(backup.open(QIODevice::ReadOnly));
    QCOMPARE(backup.readAll(), before);
    QVERIFY2(saveLearnedButton(path, "grid-2", {"input1/keyboard", KEY_2}, &error), qPrintable(error));
    QCOMPARE(QDir(dir.filePath("backups")).entryList(QDir::Files).size(), 2);
}

void MouseTest::learningRefusesInvalidChanges() {
    QTemporaryDir dir;
    const QString path = dir.filePath("mouse.json");
    QString error;
    QVERIFY(saveLearnedButton(path, "grid-1", {"input1/keyboard", KEY_1}, &error));
    QFile file(path);
    QVERIFY(file.open(QIODevice::ReadOnly));
    const auto before = file.readAll();
    file.close();
    QVERIFY(!saveLearnedButton(path, "grid-2", {"input1/keyboard", KEY_1}, &error));
    QVERIFY(error.contains("already belongs"));
    QVERIFY(!saveLearnedButton(path, "grid-2", {"input0/pointer", BTN_LEFT}, &error));
    QVERIFY(file.open(QIODevice::ReadOnly));
    QCOMPARE(file.readAll(), before);
}

void MouseTest::dbusReadinessAndFailureContract() {
    QTemporaryDir dir;
    Service service(dir.filePath("mouse.json"));
    auto bus = QDBusConnection::sessionBus();
    const QString name = "org.smplos.MouseRemapper.Test";
    const QString path = "/org/smplos/MouseRemapper1";
    QVERIFY(bus.registerService(name));
    QVERIFY(bus.registerObject(path, &service, QDBusConnection::ExportAllSlots | QDBusConnection::ExportAllSignals));
    auto client = QDBusConnection::connectToBus(QDBusConnection::SessionBus, "mouse-contract-client");
    QVERIFY(client.isConnected());
    auto call = QDBusMessage::createMethodCall(name, path, "org.smplos.MouseRemapper1", "Status");
    QDBusPendingCallWatcher statusCall(client.asyncCall(call));
    QTRY_VERIFY(statusCall.isFinished());
    QDBusPendingReply<QString> statusReply = statusCall;
    QVERIFY2(!statusReply.isError(), qPrintable(statusReply.error().message()));
    const auto status = QJsonDocument::fromJson(statusReply.value().toUtf8()).object();
    QCOMPARE(status.value("apiVersion").toInt(), 1);
    QVERIFY(!status.value("ready").toBool());
    QCOMPARE(status.value("configPath").toString(), dir.filePath("mouse.json"));
    QVERIFY(status.value("learned").isArray());
    call = QDBusMessage::createMethodCall(name, path, "org.smplos.MouseRemapper1", "SetIdentify");
    call.setArguments({true});
    QDBusPendingCallWatcher identifyCall(client.asyncCall(call));
    QTRY_VERIFY(identifyCall.isFinished());
    QDBusPendingReply<> identifyReply = identifyCall;
    QVERIFY(identifyReply.isError());
    QVERIFY(identifyReply.error().message().contains("not ready"));
    call = QDBusMessage::createMethodCall(name, path, "org.smplos.MouseRemapper1", "Learn");
    call.setArguments({"btn-left"});
    QDBusPendingCallWatcher learnCall(client.asyncCall(call));
    QTRY_VERIFY(learnCall.isFinished());
    QDBusPendingReply<> learnReply = learnCall;
    QVERIFY(learnReply.isError());
    QVERIFY(learnReply.error().message().contains("fixed codes"));
    bus.unregisterObject(path);
    bus.unregisterService(name);
    QDBusConnection::disconnectFromBus("mouse-contract-client");
}
void MouseTest::mouseUsesSharedKdenliveDispatcher() {
    auto server = QDBusConnection::connectToBus(QDBusConnection::SessionBus, "mouse-plugin-server");
    auto connection = QDBusConnection::connectToBus(QDBusConnection::SessionBus, "mouse-plugin-client");
    const QString service = "org.kde.kdenlive-mouse-test";
    MockKdenlive plugin;
    plugin.setStage(4);
    plugin.showEffectStack({});
    QVERIFY(plugin.registerOn(server));
    QVERIFY(server.registerService(service));
    KdenliveDBusClient client(connection);
    client.setServiceOverride(service);
    Router router(&client);
    QString error;
    const auto mapping = Mapping::parse(R"({"devices":{"mmo-12":{
      "learned":{"grid-1":{"source":"input1/keyboard","code":2},
                 "grid-2":{"source":"input1/keyboard","code":3},
                 "grid-3":{"source":"input1/keyboard","code":4}},
      "profiles":[
       {"name":"Kdenlive","match":{"class":"^org.kde.kdenlive$"},"kdenlive":true,
        "layers":[{"name":"Razor","when":{"tool":"razor"},
                   "bindings":{"grid-1":{"action":"select_tool"}}}],
        "bindings":{"grid-1":{"action":"monitor_pause","fallback":"space"},
                    "grid-2":{"control":"playhead.jog","scale":2},
                    "grid-3":{"request":"effect.add","params":{"id":"avfilter.gblur"}}}},
       {"name":"global","bindings":{"grid-1":"ctrl+c"}}]}}})",
                                        &error);
    QVERIFY2(mapping, qPrintable(error));
    router.setMapping(*mapping);
    QList<InputTriple> output;
    router.output = [&](bool, const InputTriple &event) { output << event; };
    router.setWindow({"org.kde.kdenlive", "Project", 4242, "0x123"});
    QTRY_VERIFY(client.isAvailable());
    QVERIFY(router.engine().kdenliveActive());
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QTRY_VERIFY(plugin.state().value("triggered").toStringList().contains("monitor_pause"));
    QVERIFY(output.isEmpty()); // API dispatch, never the configured fallback shortcut.
    const int position = plugin.state().value("position").toInt();
    QVERIFY(router.key({"input1/keyboard", KEY_2}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_2}, false, 0));
    QTRY_COMPARE(plugin.state().value("position").toInt(), position + 2);
    const auto effects = plugin.state().value("effectStack").toList().size();
    QVERIFY(router.key({"input1/keyboard", KEY_3}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_3}, false, 0));
    QTRY_COMPARE(plugin.state().value("effectStack").toList().size(), effects + 1);
    plugin.setContextValue("tool", "razor");
    QTRY_COMPARE(router.engine().resolve("grid-1")->layer, QStringLiteral("Razor"));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QTRY_VERIFY(plugin.state().value("triggered").toStringList().contains("select_tool"));
    QVERIFY(output.isEmpty());
    router.identify = true;
    const auto count = plugin.state().value("triggered").toStringList().size();
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QTest::qWait(30);
    QCOMPARE(plugin.state().value("triggered").toStringList().size(), count);
    router.identify = false;
    router.setWindow({"firefox", "", 9876, "0x456"});
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QVERIFY(!output.isEmpty()); // Other app uses Global, not the Kdenlive plugin.
    output.clear();
    server.unregisterObject(contract::kPath);
    server.unregisterService(service);
    router.setWindow({"org.kde.kdenlive", "", 4242, "0x123"});
    QTRY_VERIFY(client.isAbsent());
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QTest::qWait(30);
    QVERIFY(output.isEmpty()); // Missing API does not turn an action into a key.
    auto unsafeFallback = *mapping;
    unsafeFallback.profiles.first().keyFallback = true;
    router.setMapping(unsafeFallback); // Host policy also protects bypasses of JSON validation.
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 1));
    QVERIFY(router.key({"input1/keyboard", KEY_1}, false, 0));
    QTest::qWait(30);
    QVERIFY(output.isEmpty());
}

void MouseTest::focusLossUsesGlobalEvenForWildcardProfiles() {
    QString error;
    const auto mapping = Mapping::parse(R"({"devices":{"mmo-12":{"profiles":[
      {"name":"Wildcard","match":{"class":".*"},"bindings":{"btn-back":"a"}},
      {"name":"global","bindings":{"btn-back":"b"}}]}}})",
                                        &error);
    QVERIFY2(mapping, qPrintable(error));
    Router router;
    QList<InputTriple> output;
    router.output = [&](bool, const InputTriple &event) {
        if (event.type == EV_KEY)
            output << event;
    };
    router.setMapping(*mapping);
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QCOMPARE(output.first().code, KEY_B);
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
    output.clear();
    router.setWindow({"firefox", "", 123, "window"});
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QCOMPARE(output.first().code, KEY_A);
    router.setWindow({"firefox", "Updated title", 123, "window"});
    QCOMPARE(output.size(), 1); // A title-only update is not a focus change.
    router.setWindow({}, false);
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
    output.clear();
    router.setMapping(*mapping); // Reload while focus is unavailable must stay Global-only.
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QCOMPARE(output.first().code, KEY_B);
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
}

void MouseTest::identifyCancelsSharedDelayedSequences() {
    QString error;
    const auto mapping = Mapping::parse(R"({"devices":{"mmo-12":{"profiles":[
      {"bindings":{"btn-back":{"do":[{"command":["first"]},{"command":["later"]}],"delayMs":100}}}]}}})",
                                        &error);
    QVERIFY2(mapping, qPrintable(error));
    Router router;
    router.setMapping(*mapping);
    QStringList commands;
    router.command = [&](const Binding &binding) { commands << binding.argv.first(); };
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 1));
    QVERIFY(router.key({"input0/pointer", BTN_SIDE}, true, 0));
    QCOMPARE(commands, QStringList{"first"});
    router.releaseAll();
    router.identify = true;
    QTest::qWait(150);
    QCOMPARE(commands, QStringList{"first"});
}

QTEST_GUILESS_MAIN(MouseTest)
#include "tst_mouse.moc"
