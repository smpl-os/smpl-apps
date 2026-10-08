// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"
#include "featurelist.h"
#include "learn.h"
#include "configwatcher.h"

#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QSaveFile>
#include <QSignalSpy>
#include <QTemporaryDir>
#include <QTest>
#include <linux/input-event-codes.h>

using namespace cs;

class TestConfig : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void jsoncStripping()
    {
        const QByteArray in = "{\n"
            "            // comment\n"
            "            \"a\": \"http://x//y\", /* block */\n"
            "            \"b\": [1, 2, ],\n"
            "            \"c\": \"quote \\\" // not a comment\",\n"
            "        }";
        const auto out = stripJsonComments(in);
        QJsonParseError e;
        const auto doc = QJsonDocument::fromJson(out, &e);
        QCOMPARE(e.error, QJsonParseError::NoError);
        QCOMPARE(doc.object().value(QStringLiteral("a")).toString(), QStringLiteral("http://x//y"));
        QCOMPARE(doc.object().value(QStringLiteral("b")).toArray().size(), 2);
        QCOMPARE(doc.object().value(QStringLiteral("c")).toString(), QStringLiteral("quote \" // not a comment"));
    }
    void exampleConfigParses()
    {
        QTemporaryDir home;  // keep a learned map in the real home out of this test
        qputenv("HOME", home.path().toLocal8Bit());
        QString err;
        auto cfg = loadConfig(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"), &err);
        QVERIFY2(cfg, qPrintable(err));
        QCOMPARE(cfg->profiles.size(), 4);  // kdenlive, fl-studio, brave, global
        QVERIFY(cfg->device.serial.isEmpty());  // any 1189:8890 pad
        QCOMPARE(cfg->device.input, QStringLiteral("auto"));
        QCOMPARE(cfg->settings.coalesceMs, 8);
        const Profile *kd = cfg->profileFor(QStringLiteral("org.kde.kdenlive"), QStringLiteral("x"));
        QCOMPARE(kd->name, QStringLiteral("kdenlive"));
        QVERIFY(kd->kdenlive);
        QVERIFY(!kd->keyFallback);  // API only by default
        QStringList layers;
        for (const auto &l : kd->layers) {
            layers << l.name;
        }
        QCOMPARE(layers, (QStringList{QStringLiteral("color-wheels"), QStringLiteral("effect-parameter"), QStringLiteral("track-video"), QStringLiteral("track-mixer"),
                                      QStringLiteral("trim"), QStringLiteral("clip-monitor"), QStringLiteral("project-monitor"), QStringLiteral("timeline")}));
        QCOMPARE(kd->layers.at(3).when.value(QStringLiteral("$mode.page")).toString(), QStringLiteral("track"));
        QCOMPARE(kd->layers.at(0).bindings.value(QStringLiteral("knob2.turn")).options.value(QStringLiteral("wheel")).toString(), QStringLiteral("gamma"));
        QCOMPARE(kd->layers.at(0).bindings.value(QStringLiteral("knob2.press")).kind, Binding::Cycle);
        QVERIFY(!kd->layers.at(0).bindings.contains(QStringLiteral("knob2.shift.turn")));  // the firmware ignores turns while pressed
        QCOMPARE(kd->bindings.value(QStringLiteral("key13")).kind, Binding::Cycle);
        QVERIFY(cfg->warnings.isEmpty());
        QCOMPARE(cfg->profileFor(QStringLiteral("org.kde.kdenlive.automation-preview"), {})->name, QStringLiteral("kdenlive"));
        QCOMPARE(cfg->profileFor(QStringLiteral("firefox"), {})->name, QStringLiteral("global"));
        const Binding jog = kd->bindings.value(QStringLiteral("knob1.turn"));
        QCOMPARE(jog.kind, Binding::Control);
        QCOMPARE(jog.name, QStringLiteral("playhead.jog"));
        QCOMPARE(jog.keys.size(), 2);
        QCOMPARE(kd->bindings.value(QStringLiteral("knob2.turn")).name, QStringLiteral("timeline.zoom"));
        QCOMPARE(kd->bindings.value(QStringLiteral("knob3.turn")).name, QStringLiteral("timeline.track"));
        QCOMPARE(kd->bindings.value(QStringLiteral("key1")).kind, Binding::Cheatsheet);  // held: the overlay
        QCOMPARE(kd->bindings.value(QStringLiteral("key2")).name, QStringLiteral("mark_in"));
        QCOMPARE(kd->bindings.value(QStringLiteral("key2")).keys.value(0).key, KEY_I);
        QCOMPARE(cfg->profileFor(QStringLiteral("brave-browser"), {})->name, QStringLiteral("brave"));
        QCOMPARE(kd->modes.value(QStringLiteral("liftAxis")).size(), 4);
        // The learned map is absent in a clean checkout: default numbering applies.
        QCOMPARE(cfg->hardware.size(), 24);
    }
    void bindingForms()
    {
        QString err;
        QCOMPARE(parseBinding(QJsonValue(QStringLiteral("ctrl+z")), &err)->kind, Binding::Keys);
        QCOMPARE(parseBinding(QJsonValue(QStringLiteral("action:mark_in")), &err)->name, QStringLiteral("mark_in"));
        QCOMPARE(parseBinding(QJsonValue(QStringLiteral("cycle:axis")), &err)->kind, Binding::Cycle);
        QCOMPARE(parseBinding(QJsonValue(QStringLiteral("none")), &err)->kind, Binding::None);
        QCOMPARE(parseBinding(QJsonValue(), &err)->kind, Binding::None);
        auto cmd = parseBinding(QJsonDocument::fromJson("{\"command\":[\"notify-send\",\"hi\"]}").object(), &err);
        QCOMPARE(cmd->argv, (QStringList{QStringLiteral("notify-send"), QStringLiteral("hi")}));
        auto req = parseBinding(QJsonDocument::fromJson("{\"request\":\"colorwheel.reset\",\"params\":{\"wheel\":\"lift\"}}").object(), &err);
        QCOMPARE(req->options.value(QStringLiteral("wheel")).toString(), QStringLiteral("lift"));
        QVERIFY(!parseBinding(QJsonDocument::fromJson("{\"command\":[]}").object(), &err));
        QVERIFY(!parseBinding(QJsonDocument::fromJson("{\"foo\":1}").object(), &err));
        QVERIFY(!parseBinding(QJsonValue(QStringLiteral("ctrl+nokey")), &err));
    }
    void rejectsBadConfigs()
    {
        QString err;
        QVERIFY(!parseConfig("{", {}, &err));
        QVERIFY(parseConfig("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"key16\":\"a\"}}]}", {}, &err));  // 16-key pads
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"key17\":\"a\"}}]}", {}, &err));
        QVERIFY(err.contains(QStringLiteral("key17")));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"knob1\":{\"spin\":\"a\"}}}]}", {}, &err));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"x\",\"match\":{\"class\":\"(\"}}]}", {}, &err));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"x\",\"modes\":{\"m\":[]}}]}", {}, &err));
        QVERIFY(parseConfig("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"knob1.cw\":\"right\",\"knob2\":{\"press\":\"a\"}}}]}", {}, &err));
        QVERIFY(parseConfig("{\"hardware\":\"default:vendor-twelve\"}", {}, &err));
        // Only the pad may be configured: another id would grab another keyboard.
        QVERIFY(!parseConfig("{\"device\":{\"vendor\":\"0c45\",\"product\":\"760a\"}}", {}, &err));
        QVERIFY(err.contains(QStringLiteral("refused")));
        QVERIFY(parseConfig("{\"device\":{\"vendor\":\"1189\",\"product\":\"8890\",\"serial\":\"x\"}}", {}, &err));
    }
    void shiftSlotsAccelAndKeyFallback()
    {
        QString err;
        auto c = parseConfig("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"keyFallback\":true,\"bindings\":{"
                             "\"knob1\":{\"turn\":{\"control\":\"playhead.jog\",\"accel\":2.5},\"shift\":{\"turn\":{\"control\":\"timeline.scroll\"},\"cw\":\"right\"}},"
                             "\"knob2.shift.ccw\":\"left\"}}]}",
                             {}, &err);
        QVERIFY2(c, qPrintable(err));
        const Profile &p = c->profiles.first();
        QVERIFY(p.keyFallback);
        QCOMPARE(p.bindings.value(QStringLiteral("knob1.shift.turn")).name, QStringLiteral("timeline.scroll"));
        QCOMPARE(p.bindings.value(QStringLiteral("knob1.shift.cw")).kind, Binding::Keys);
        QCOMPARE(p.bindings.value(QStringLiteral("knob2.shift.ccw")).kind, Binding::Keys);
        QCOMPARE(p.bindings.value(QStringLiteral("knob1.turn")).accel, 2.5);
        QCOMPARE(p.bindings.value(QStringLiteral("knob1.turn")).scale, 1.0);
        auto d = parseConfig("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true}]}", {}, &err);
        QVERIFY(!d->profiles.first().keyFallback);  // default: API only
        // Shift has no press, keys have no shift, accel/scale are bounded.
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"k\",\"bindings\":{\"knob1\":{\"shift\":{\"press\":\"a\"}}}}]}", {}, &err));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"k\",\"bindings\":{\"key1.shift.turn\":\"a\"}}]}", {}, &err));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"k\",\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"playhead.jog\",\"accel\":-1}}}}]}", {}, &err));
        QVERIFY(err.contains(QStringLiteral("accel")));
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"k\",\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"playhead.jog\",\"scale\":0}}}}]}", {}, &err));
    }

    // Validation: references that can never work are errors naming the place;
    // names a newer Kdenlive might offer are warnings.
    void validationErrorsAndWarnings()
    {
        QString err;
        auto bad = [&](const char *json, const QString &needle) {
            const bool rejected = !parseConfig(json, {}, &err);
            return rejected && err.contains(needle);
        };
        QVERIFY2(bad("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"bindings\":{\"key1\":{\"cycle\":\"nope\"}}}]}",
                     QStringLiteral("profile k key1: cycles undefined mode 'nope'")), qPrintable(err));
        QVERIFY2(bad("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"$nope\"}}}}]}",
                     QStringLiteral("undefined mode 'nope'")), qPrintable(err));
        QVERIFY2(bad("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"layers\":[{\"name\":\"L\",\"when\":{\"focus\":\"timeline\"},"
                     "\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"param.nudge\",\"options\":{\"step\":\"$nope\"}}}}}]}]}",
                     QStringLiteral("profile k layer L knob1.turn: option step uses undefined mode 'nope'")), qPrintable(err));
        QVERIFY2(bad("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"layers\":[{\"name\":\"L\",\"when\":{\"$mode.page\":\"x\"}}]}]}",
                     QStringLiteral("layer L: condition uses undefined mode 'page'")), qPrintable(err));
        // Modes from the global profile and context references are fine.
        auto ok = parseConfig("{\"profiles\":[{\"name\":\"k\",\"match\":{\"class\":\"k\"},\"kdenlive\":true,\"bindings\":{\"key1\":{\"cycle\":\"g\"},"
                              "\"key2\":{\"request\":\"track.set\",\"params\":{\"what\":\"mute\",\"value\":\"$!ctx:timeline.track.muted\"}}}},"
                              "{\"name\":\"global\",\"modes\":{\"g\":[\"a\",\"b\"]}}]}",
                              {}, &err);
        QVERIFY2(ok, qPrintable(err));
        QVERIFY(ok->warnings.isEmpty());
        // Warnings: unknown names, Kdenlive-only bindings elsewhere, unconditional layers, misplaced keyFallback.
        auto w = parseConfig("{\"profiles\":[{\"name\":\"k\",\"kdenlive\":true,\"layers\":[{\"name\":\"always\",\"bindings\":{}}],"
                             "\"bindings\":{\"knob1\":{\"turn\":{\"control\":\"playhead.warp\"}},\"key1\":{\"request\":\"clip.explode\"}}},"
                             "{\"name\":\"app\",\"match\":{\"class\":\"a\"},\"keyFallback\":true,\"bindings\":{\"key1\":{\"action\":\"mark_in\"}}}]}",
                             {}, &err);
        QVERIFY2(w, qPrintable(err));
        const QString all = w->warnings.join(QLatin1Char('\n'));
        QVERIFY2(all.contains(QStringLiteral("profile k knob1.turn: unknown control 'playhead.warp'")), qPrintable(all));
        QVERIFY2(all.contains(QStringLiteral("profile k key1: unknown command 'clip.explode'")), qPrintable(all));
        QVERIFY2(all.contains(QStringLiteral("profile app key1: action bindings only work in a profile with \"kdenlive\": true")), qPrintable(all));
        QVERIFY2(all.contains(QStringLiteral("layer always: no \"when\" condition")), qPrintable(all));
        QVERIFY2(all.contains(QStringLiteral("profile app: \"keyFallback\" only applies")), qPrintable(all));
        QCOMPARE(w->warnings.size(), 5);
    }

    // Hot reload: valid edits (in place or by rename) reload, a broken file
    // is reported and never replaces the running config, unchanged saves are quiet.
    void watcherReloadsValidEditsOnly()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("config.jsonc"));
        auto write = [&](const QByteArray &text, bool byRename) {
            if (byRename) {
                QSaveFile f(path);
                QVERIFY(f.open(QIODevice::WriteOnly));
                f.write(text);
                QVERIFY(f.commit());
            } else {
                QFile f(path);
                QVERIFY(f.open(QIODevice::WriteOnly | QIODevice::Truncate));
                f.write(text);
            }
        };
        write("{\"profiles\":[{\"name\":\"one\"}]}", false);
        ConfigWatcher w(path);
        w.setDebounceMs(50);
        QSignalSpy ok(&w, &ConfigWatcher::reloaded);
        QSignalSpy bad(&w, &ConfigWatcher::failed);
        w.start();
        write("{\"profiles\":[{\"name\":\"two\"}]}", false);
        QTRY_COMPARE_WITH_TIMEOUT(ok.size(), 1, 5000);
        QCOMPARE(ok.last().first().value<Config>().profiles.first().name, QStringLiteral("two"));
        write("{\"profiles\":[{\"name\":\"k\",\"bindings\":{\"key1\":{\"cycle\":\"nope\"}}}]}", false);
        QTRY_COMPARE_WITH_TIMEOUT(bad.size(), 1, 5000);
        QVERIFY2(bad.last().first().toString().contains(QStringLiteral("undefined mode 'nope'")), qPrintable(bad.last().first().toString()));
        QVERIFY(bad.last().first().toString().contains(QStringLiteral("keeping the running config")));
        QCOMPARE(ok.size(), 1);
        write("{\"profiles\":[{\"name\":\"three\"}]}", true);  // editors that save by rename
        QTRY_COMPARE_WITH_TIMEOUT(ok.size(), 2, 5000);
        QCOMPARE(ok.last().first().value<Config>().profiles.first().name, QStringLiteral("three"));
        write("{\"profiles\":[{\"name\":\"four\"}]}", false);  // still watched after the rename
        QTRY_COMPARE_WITH_TIMEOUT(ok.size(), 3, 5000);
        write("{\"profiles\":[{\"name\":\"four\"}]}", false);  // same bytes: no reload
        QTest::qWait(300);
        QCOMPARE(ok.size(), 3);
        QCOMPARE(bad.size(), 1);
    }

    void conditions()
    {
        const QVariantMap ctx{{QStringLiteral("focus"), QStringLiteral("effectStack")},
                              {QStringLiteral("playing"), false},
                              {QStringLiteral("effect"), QVariantMap{{QStringLiteral("id"), QStringLiteral("lift_gamma_gain")}}},
                              {QStringLiteral("param"), QVariantMap{{QStringLiteral("name"), QStringLiteral("lift_r")}, {QStringLiteral("min"), 0}}}};
        QVERIFY(conditionMatches({}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("focus"), QStringLiteral("effectStack")}}, ctx));
        QVERIFY(!conditionMatches({{QStringLiteral("focus"), QStringLiteral("timeline")}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("effect.id"), QStringLiteral("/^lift_/")}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("focus"), QVariantList{QStringLiteral("timeline"), QStringLiteral("effectStack")}}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("focus"), QStringLiteral("!timeline")}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("colorWheel"), false}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("param"), true}}, ctx));
        QVERIFY(!conditionMatches({{QStringLiteral("playing"), true}}, ctx));
        QVERIFY(conditionMatches({{QStringLiteral("param.min"), 0}}, ctx));
        QVERIFY(!conditionMatches({{QStringLiteral("missing.path"), QStringLiteral("x")}}, ctx));
        QVERIFY(!conditionMatches({{QStringLiteral("param.name"), QStringLiteral("/.+/")}}, QVariantMap{}));
        // Lists (MR2 colorWheels): an empty array is falsy, a populated one truthy.
        const QVariantMap none{{QStringLiteral("colorWheels"), QVariantList{}}, {QStringLiteral("axes"), QStringList{}}};
        const QVariantMap three{{QStringLiteral("colorWheels"), QVariantList{QVariantMap{{QStringLiteral("wheel"), QStringLiteral("lift")}}}},
                                {QStringLiteral("axes"), QStringList{QStringLiteral("value")}}};
        QVERIFY(!conditionMatches({{QStringLiteral("colorWheels"), true}}, none));
        QVERIFY(conditionMatches({{QStringLiteral("colorWheels"), false}}, none));
        QVERIFY(!conditionMatches({{QStringLiteral("axes"), true}}, none));
        QVERIFY(conditionMatches({{QStringLiteral("colorWheels"), true}}, three));
        QVERIFY(conditionMatches({{QStringLiteral("axes"), true}}, three));
        QVERIFY(!conditionMatches({{QStringLiteral("colorWheels"), true}}, ctx));  // absent
    }

    void configIssues()
    {
        // Real messages from the parser and the checker, split for an editor.
        auto issueOf = [](const char *json) {
            QString err;
            auto c = parseConfig(json, {}, &err);
            if (c) {
                return describeConfigIssue(c->warnings.value(0));
            }
            return describeConfigIssue(err);
        };
        ConfigIssue i = issueOf(R"({"profiles":[{"name":"Kdenlive edit","bindings":{"key3":{"bogus":1}}}]})");
        QCOMPARE(i.profile, QStringLiteral("Kdenlive edit"));
        QCOMPARE(i.slot, QStringLiteral("key3"));
        QVERIFY(i.layer.isEmpty());
        QVERIFY(i.message.contains(QStringLiteral("binding object needs")));

        i = issueOf(R"({"profiles":[{"name":"x","bindings":{"key99":"a"}}]})");
        QCOMPARE(i.profile, QStringLiteral("x"));
        QCOMPARE(i.slot, QStringLiteral("key99"));

        i = issueOf(R"({"profiles":[{"name":"x","bindings":{"knob2":{"shift":{"cw":7}}}}]})");
        QCOMPARE(i.slot, QStringLiteral("knob2.shift.cw"));

        i = issueOf(R"({"profiles":[{"name":"k","kdenlive":true,"layers":[{"name":"wheels","when":{"a":1},"bindings":{"knob1.cw":{"keys":7}}}]}]})");
        QCOMPARE(i.profile, QStringLiteral("k"));
        QCOMPARE(i.layer, QStringLiteral("wheels"));
        QCOMPARE(i.slot, QStringLiteral("knob1.cw"));

        i = issueOf(R"({"profiles":[{"name":"k","bindings":{"key2":{"cycle":"nomode"}}}]})");  // checkConfig error
        QCOMPARE(i.profile, QStringLiteral("k"));
        QCOMPARE(i.slot, QStringLiteral("key2"));

        i = issueOf(R"({"profiles":[{"name":"g","bindings":{"knob1.cw":{"control":"nope.x"}}}]})");  // a warning
        QCOMPARE(i.profile, QStringLiteral("g"));
        QCOMPARE(i.slot, QStringLiteral("knob1.cw"));

        i = issueOf("{\"profiles\":[{\"name\":\"m\",\"match\":{\"class\":\"[\"}}]}");  // bad regex
        QCOMPARE(i.profile, QStringLiteral("m"));
        QVERIFY(i.slot.isEmpty());

        i = issueOf("{oops");
        QVERIFY(i.profile.isEmpty());
        QVERIFY(i.message.startsWith(QStringLiteral("JSON error")));
        const QJsonObject j = i.toJson();
        QVERIFY(j.value(QStringLiteral("profile")).isNull());
        QVERIFY(j.value(QStringLiteral("slot")).isNull());
        QCOMPARE(j.value(QStringLiteral("message")).toString(), i.message);
    }

    void mouseBindings()
    {
        QString err;
        auto c = parseConfig(R"({"profiles":[{"name":"g","bindings":{"key1":{"mouse":"left"},"knob2.cw":{"mouse":"wheel-down"},"knob2":{"press":{"mouse":"middle"}}}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        const Binding b = c->profiles.first().bindings.value(QStringLiteral("key1"));
        QCOMPARE(b.kind, Binding::Mouse);
        QCOMPARE(b.name, QStringLiteral("left"));
        QCOMPARE(b.describe(), QStringLiteral("mouse:left"));
        QCOMPARE(c->profiles.first().bindings.value(QStringLiteral("knob2.press")).name, QStringLiteral("middle"));
        QVERIFY(!parseConfig(R"({"profiles":[{"name":"g","bindings":{"key1":{"mouse":"scroll"}}}]})", {}, &err));
        QVERIFY(err.contains(QStringLiteral("unknown mouse action")));
        QCOMPARE(describeConfigIssue(err).slot, QStringLiteral("key1"));
        QVERIFY(!parseConfig(R"({"profiles":[{"name":"g","bindings":{"key1":{"mouse":7}}}]})", {}, &err));
    }

    void layouts()
    {
        QString err;
        // Precedence: config "layout" > firmware board > hardware map > default.
        // Nothing known: the default (15 keys, 3 knobs = the measured board).
        auto c = parseConfig(R"({"profiles":[]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QVERIFY(!c->layout);
        BoardProfile l = effectiveLayout(*c);
        QCOMPARE(l.id, QStringLiteral("sy181-15k3e"));
        QCOMPARE(l.source, QStringLiteral("default"));
        // The firmware's board wins over that.
        l = effectiveLayout(*c, QStringLiteral("generic-3k1e"));
        QCOMPARE(l.id, QStringLiteral("generic-3k1e"));
        QCOMPARE(l.source, QStringLiteral("firmware"));
        QCOMPARE(effectiveLayout(*c, QStringLiteral("no-such-board")).source, QStringLiteral("default"));
        // A hardware map that names this pad's inputs (learned, or a scheme set on purpose).
        c = parseConfig(R"({"hardware":"default:vendor-twelve","profiles":[]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(effectiveLayout(*c).source, QStringLiteral("hardware-map"));
        QCOMPARE(effectiveLayout(*c, QStringLiteral("sy181-15k3e")).source, QStringLiteral("firmware"));
        // A missing learned map is still the default.
        c = parseConfig(R"({"hardware":"/nonexistent/hardware-map.json","profiles":[]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(effectiveLayout(*c).source, QStringLiteral("default"));

        // A board profile by id: the user's override wins, also over the firmware.
        c = parseConfig(R"({"layout":"generic-12k2e","profiles":[{"name":"g","bindings":{"key13":"a","knob3.cw":"b","key2":"c"}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->layout->keys.size(), 12);
        l = effectiveLayout(*c);
        QCOMPARE(l.source, QStringLiteral("config"));
        QCOMPARE(l.knobs.size(), 2);
        l = effectiveLayout(*c, QStringLiteral("sy181-15k3e"));
        QCOMPARE(l.id, QStringLiteral("generic-12k2e"));
        QCOMPARE(l.source, QStringLiteral("config"));
        // ... reported with the firmware's layout, and a warning on another slot count.
        auto fw = builtinBoardProfile(QStringLiteral("sy181-15k3e"));
        fw->source = QStringLiteral("firmware");
        QJsonObject report = layoutReport(l, fw);
        QCOMPARE(report.value(QStringLiteral("source")).toString(), QStringLiteral("config"));
        QCOMPARE(report.value(QStringLiteral("firmwareLayout")).toObject().value(QStringLiteral("id")).toString(), QStringLiteral("sy181-15k3e"));
        QCOMPARE(report.value(QStringLiteral("firmwareSlots")).toInt(), 24);
        QCOMPARE(report.value(QStringLiteral("matchesFirmware")).toBool(), false);
        const QString warn = layoutMismatchWarning(l, fw);
        QVERIFY2(warn.startsWith(QLatin1String("layout: the config's generic-12k2e (12 keys, 2 knobs = 18 slots) overrides the firmware's sy181-15k3e, which has 24 slots")), qPrintable(warn));
        const ConfigIssue issue = describeConfigIssue(warn);
        QVERIFY(issue.profile.isEmpty() && issue.layer.isEmpty() && issue.slot.isEmpty());
        QCOMPARE(layoutMismatchWarning(l, fw, 18), QString());           // GET_INFO's count decides
        QCOMPARE(layoutReport(l, fw, 18).value(QStringLiteral("matchesFirmware")).toBool(), true);
        QCOMPARE(layoutMismatchWarning(l, std::nullopt), QString());     // no firmware information
        QVERIFY(layoutReport(l, std::nullopt).value(QStringLiteral("firmwareLayout")).isNull());
        QVERIFY(layoutReport(l, std::nullopt).value(QStringLiteral("matchesFirmware")).isNull());
        QCOMPARE(layoutMismatchWarning(*fw, fw), QString());             // not an override
        QCOMPARE(layoutReport(*fw, fw).value(QStringLiteral("matchesFirmware")).toBool(), true);
        QVERIFY(!layoutMismatchWarning(l, std::nullopt, 24).isEmpty()); // GET_INFO alone is enough
        QCOMPARE(c->warnings.size(), 2);  // key13 and knob3 are not on this pad
        QCOMPARE(describeConfigIssue(c->warnings.value(0)).profile, QStringLiteral("g"));
        QVERIFY(!parseConfig(R"({"layout":"nope","profiles":[]})", {}, &err));
        QVERIFY(err.contains(QStringLiteral("generic-3k1e")));

        // A custom grid.
        c = parseConfig(R"({"layout":{"keys":16,"knobs":0,"columns":4},"profiles":[{"name":"g","layers":[{"name":"L","when":{"a":1},"bindings":{"knob1.cw":"x"}}],"bindings":{"key16":"a"}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->layout->rows, 4);
        QCOMPARE(c->layout->slotCount(), 16);
        QCOMPARE(c->warnings.size(), 1);
        const ConfigIssue i = describeConfigIssue(c->warnings.value(0));
        QCOMPARE(i.layer, QStringLiteral("L"));
        QCOMPARE(i.slot, QStringLiteral("knob1.cw"));
        for (const char *bad : {R"({"layout":{"keys":17}})", R"({"layout":{"keys":3,"knobs":4}})", R"({"layout":{"keys":0,"knobs":0}})",
                                R"({"layout":{"keys":4,"columns":9}})", R"({"layout":7})"}) {
            QVERIFY2(!parseConfig(bad, {}, &err), bad);
            QVERIFY(err.startsWith(QStringLiteral("layout")));
        }
        QVERIFY(parseConfig(R"({"layout":{"knobs":1,"keys":0}})", {}, &err));

        // The learn walk follows the layout.
        const auto targets = learnTargets(*builtinBoardProfile(QStringLiteral("generic-3k1e")));
        QCOMPARE(targets.size(), 3 + 3);
        QCOMPARE(targets.at(3).control, QStringLiteral("knob1"));
    }

    void deviceInput()
    {
        QString err;
        auto c = parseConfig(R"({"profiles":[]})", {}, &err);
        QCOMPARE(c->device.input, QStringLiteral("auto"));
        QVERIFY(c->device.serial.isEmpty());
        c = parseConfig(R"({"device":{"serial":"","input":"evdev"},"profiles":[]})", {}, &err);
        QCOMPARE(c->device.input, QStringLiteral("evdev"));
        QVERIFY(!parseConfig(R"({"device":{"input":"usb"},"profiles":[]})", {}, &err));
        QVERIFY(err.contains(QStringLiteral("device.input")));
    }

    void featuresAreAccepted()
    {
        // What features --json advertises must be accepted by the parser.
        const QJsonObject f = featuresJson();
        QString err;
        for (const auto &k : f.value(QStringLiteral("keyNames")).toArray()) {
            QVERIFY2(parseChord(k.toString(), &err), qPrintable(k.toString()));
            QVERIFY2(parseChord(QStringLiteral("ctrl+") + k.toString().toLower(), &err), qPrintable(k.toString()));
        }
        for (const auto &m : f.value(QStringLiteral("modifierNames")).toArray()) {
            QVERIFY2(parseChord(m.toString() + QStringLiteral("+F14"), &err), qPrintable(m.toString()));
        }
        for (const auto &m : f.value(QStringLiteral("mouseNames")).toArray()) {
            QVERIFY(parseBinding(QJsonObject{{QStringLiteral("mouse"), m}}, &err));
        }
        for (const auto &k : f.value(QStringLiteral("bindingKinds")).toArray()) {
            const QString example = k.toObject().value(QStringLiteral("example")).toString().section(QStringLiteral(" | "), -1);
            const QJsonDocument d = QJsonDocument::fromJson(QByteArray("[") + example.toUtf8() + "]");
            QVERIFY2(d.isArray(), qPrintable(example));
            QVERIFY2(parseBinding(d.array().at(0), &err), qPrintable(example + QStringLiteral(": ") + err));
        }
    }

    void cheatsheetBindingAndOptions()
    {
        QString err;
        auto c = parseConfig(R"({"profiles":[{"name":"g","bindings":{"key15":{"cheatsheet":"toggle","label":"Help"},"knob2":{"press":{"cheatsheet":"hold"}}}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        const Binding t = c->profiles.first().bindings.value(QStringLiteral("key15"));
        QCOMPARE(t.kind, Binding::Cheatsheet);
        QCOMPARE(t.name, QStringLiteral("toggle"));
        QCOMPARE(t.label, QStringLiteral("Help"));
        QCOMPARE(t.describe(), QStringLiteral("cheatsheet:toggle"));
        QCOMPARE(c->profiles.first().bindings.value(QStringLiteral("knob2.press")).name, QStringLiteral("hold"));
        QVERIFY(c->warnings.isEmpty());
        // Defaults.
        QCOMPARE(c->cheatsheet.opacity, 0.35);
        // features.cheatsheet.defaults is what an unset option means.
        const QJsonObject defaults = featuresJson().value(QStringLiteral("cheatsheet")).toObject().value(QStringLiteral("defaults")).toObject();
        QCOMPARE(defaults, (QJsonObject{{QStringLiteral("opacity"), c->cheatsheet.opacity},
                                        {QStringLiteral("autoHideMs"), c->cheatsheet.effectiveAutoHideMs()},
                                        {QStringLiteral("position"), c->cheatsheet.position}}));
        QVERIFY(!c->cheatsheet.autoHideMs);  // unset: hides itself after 8 s
        QCOMPARE(c->cheatsheet.effectiveAutoHideMs(), 8000);
        QCOMPARE(c->cheatsheet.position, QStringLiteral("center"));

        QVERIFY(!parseConfig(R"({"profiles":[{"name":"g","bindings":{"key1":{"cheatsheet":"always"}}}]})", {}, &err));
        QVERIFY(err.contains(QStringLiteral("toggle")));
        // Only on something pressed.
        for (const char *bad : {R"({"profiles":[{"name":"g","bindings":{"knob1.cw":{"cheatsheet":"toggle"}}}]})",
                                R"({"profiles":[{"name":"g","bindings":{"knob1":{"turn":{"cheatsheet":"toggle"}}}}]})",
                                R"({"profiles":[{"name":"g","layers":[{"name":"L","when":{"a":1},"bindings":{"knob3.ccw":{"cheatsheet":"hold"}}}]}]})"}) {
            QVERIFY2(!parseConfig(bad, {}, &err), bad);
            QVERIFY(err.contains(QStringLiteral("key or a knob press")));
            QVERIFY(!describeConfigIssue(err).slot.isEmpty());
        }
        // hold on a knob whose press waits for release: a warning.
        c = parseConfig(R"({"profiles":[{"name":"g","bindings":{"knob1":{"press":{"cheatsheet":"hold"},"shift":{"cw":"a"}}}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->warnings.size(), 1);
        QVERIFY(c->warnings.first().contains(QStringLiteral("acts as \"toggle\"")));
        QCOMPARE(describeConfigIssue(c->warnings.first()).slot, QStringLiteral("knob1.press"));

        // Options.
        c = parseConfig(R"({"cheatsheet":{"opacity":0.5,"autoHideMs":4000,"position":"top-right"},"profiles":[]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->cheatsheet.opacity, 0.5);
        QCOMPARE(c->cheatsheet.autoHideMs, std::optional<int>(4000));
        QCOMPARE(c->cheatsheet.effectiveAutoHideMs(), 4000);
        QCOMPARE(c->cheatsheet.position, QStringLiteral("top-right"));
        c = parseConfig(R"({"cheatsheet":{"autoHideMs":0},"profiles":[]})", {}, &err);  // explicitly: until hidden
        QVERIFY2(c, qPrintable(err));
        QCOMPARE(c->cheatsheet.autoHideMs, std::optional<int>(0));
        QCOMPARE(c->cheatsheet.effectiveAutoHideMs(), 0);
        for (const char *bad : {R"({"cheatsheet":{"opacity":0},"profiles":[]})", R"({"cheatsheet":{"opacity":1.5},"profiles":[]})",
                                R"({"cheatsheet":{"autoHideMs":-1},"profiles":[]})", R"({"cheatsheet":{"position":"middle"},"profiles":[]})",
                                R"({"cheatsheet":{"autoHideMs":"8s"},"profiles":[]})", R"({"cheatsheet":{"autoHideMs":600001},"profiles":[]})",
                                R"({"cheatsheet":{"autoHideMs":null},"profiles":[]})",
                                R"({"cheatsheet":{"color":"red"},"profiles":[]})", R"({"cheatsheet":true,"profiles":[]})"}) {
            QVERIFY2(!parseConfig(bad, {}, &err), bad);
            QVERIFY(err.startsWith(QStringLiteral("cheatsheet")));
        }
    }

    void bindingFields()
    {
        QString err;
        auto one = [&](const QByteArray &json) { return parseBinding(QJsonDocument::fromJson("[" + json + "]").array().at(0), &err); };
        // icon: a Tabler outline name (unknown names pass), or "none".
        for (const char *ok : {"player-play", "brand-github", "chart-dots-3", "none", "some-future-icon"}) {
            const auto b = one(QByteArray(R"({"keys": "a", "icon": ")") + ok + "\"}");
            QVERIFY2(b, ok);
            QCOMPARE(b->icon, QLatin1String(ok));
        }
        for (const char *bad : {"Player-play", "app:grafium", "-x", "x-", "a b", "", "a--b"}) {
            QVERIFY2(!one(QByteArray(R"({"keys": "a", "icon": ")") + bad + "\"}"), bad);
            QVERIFY(err.contains(QStringLiteral("icon")));
        }
        QVERIFY(!one(R"({"keys": "a", "icon": 3})"));
        QVERIFY(one(R"({"keys": "a"})")->icon.isEmpty());  // automatic
        // ifInstalled: one name or a list.
        auto b = one(R"({"command": ["gtk-launch", "grafium"], "ifInstalled": "grafium"})");
        QVERIFY2(b, qPrintable(err));
        QCOMPARE(b->ifInstalled, QStringList{QStringLiteral("grafium")});
        b = one(R"({"command": ["gtk-launch", "grafium"], "ifInstalled": ["gtk-launch", "grafium"]})");
        QCOMPARE(b->ifInstalled, (QStringList{QStringLiteral("gtk-launch"), QStringLiteral("grafium")}));
        for (const char *bad : {R"({"keys": "a", "ifInstalled": 1})", R"({"keys": "a", "ifInstalled": ""})", R"({"keys": "a", "ifInstalled": ["x", 2]})",
                                R"({"keys": "a", "ifInstalled": "two words"})"}) {
            QVERIFY2(!one(bad), bad);
            QVERIFY(err.contains(QStringLiteral("ifInstalled")));
        }
        // "~" in a command line is the home directory.
        b = one(R"({"command": ["xdg-open", "~", "~/Videos", "a~b", "~user"]})");
        QCOMPARE(b->argv, (QStringList{QStringLiteral("xdg-open"), QDir::homePath(), QDir::homePath() + QStringLiteral("/Videos"), QStringLiteral("a~b"),
                                       QStringLiteral("~user")}));
    }

    void exampleConfigUsesEveryInputOnce()
    {
        // The shipped example: the cheatsheet on key1 in every profile, and
        // launchers that only apply when their app is installed.
        QFile f(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"));
        QVERIFY(f.open(QIODevice::ReadOnly));
        QString err;
        const auto c = parseConfig(f.readAll(), QStringLiteral(CS_SOURCE_DIR "/data"), &err);
        QVERIFY2(c, qPrintable(err));
        QVERIFY2(c->warnings.isEmpty(), qPrintable(c->warnings.join(QLatin1Char('\n'))));
        const Profile *global = c->globalProfile();
        QVERIFY(global);
        for (const Profile &p : c->profiles) {
            // Its own key1, or the global one through the fall-through (Brave).
            const Binding k1 = p.bindings.value(QStringLiteral("key1"), global->bindings.value(QStringLiteral("key1")));
            QVERIFY2(k1.kind == Binding::Cheatsheet && k1.name == QLatin1String("hold"), qPrintable(p.name));
        }
        int launchers = 0;
        for (const Binding &b : global->bindings) {
            if (b.kind == Binding::Command) {
                ++launchers;
                QVERIFY2(!b.ifInstalled.isEmpty(), qPrintable(b.argv.join(QLatin1Char(' '))));
            }
        }
        QCOMPARE(launchers, 7);
        QVERIFY(!c->cheatsheet.autoHideMs);
    }

    void heldLayers()
    {
        QString err;
        auto c = parseConfig(R"({"profiles": [{"name": "g",
            "layers": [{"name": "a", "when": {"held": "key1"}, "bindings": {"knob1.cw": "a"}},
                       {"name": "b", "when": {"held": ["key14", " key15 "]}, "bindings": {"knob1.cw": "b"}},
                       {"name": "c", "when": {"held": "key13+knob3+key13", "$mode.m": "x"}, "bindings": {"key2": "c"}}],
            "modes": {"m": ["x"]}, "bindings": {"key1": "a"}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        const Profile &p = c->profiles.first();
        QCOMPARE(p.layers[0].held, (QList<QStringList>{{QStringLiteral("key1")}}));
        QCOMPARE(p.layers[1].held, (QList<QStringList>{{QStringLiteral("key14")}, {QStringLiteral("key15")}}));
        QCOMPARE(p.layers[2].held, (QList<QStringList>{{QStringLiteral("key13"), QStringLiteral("knob3")}}));
        QVERIFY(!p.layers[0].when.contains(QStringLiteral("held")));
        QCOMPARE(p.layers[2].when.value(QStringLiteral("$mode.m")).toString(), QStringLiteral("x"));
        QCOMPARE(p.heldControls, (QSet<QString>{QStringLiteral("key1"), QStringLiteral("key13"), QStringLiteral("key14"), QStringLiteral("key15"), QStringLiteral("knob3")}));
        QVERIFY2(c->warnings.isEmpty(), qPrintable(c->warnings.join(QLatin1Char('\n'))));  // a held layer has a condition
        QVERIFY(p.layers[2].heldMatches({QStringLiteral("key13"), QStringLiteral("knob3"), QStringLiteral("key1")}));
        QVERIFY(!p.layers[2].heldMatches({QStringLiteral("key13")}));
        QVERIFY(p.layers[1].heldMatches({QStringLiteral("key15")}));
        QVERIFY(!p.layers[1].heldMatches({}));

        for (const char *bad : {R"("held": "key0")", R"("held": "knob4")", R"("held": "key17")", R"("held": [])", R"("held": 3)",
                                R"("held": ["key1", 2])", R"("held": "key1+")", R"("held": "F14")"}) {
            const QByteArray text = QByteArray(R"({"profiles": [{"name": "g", "layers": [{"name": "L", "when": {)") + bad + R"(}, "bindings": {"key2": "a"}}]}]})";
            QVERIFY2(!parseConfig(text, {}, &err), bad);
            QVERIFY2(err.startsWith(QStringLiteral("profile g layer L: ")) && err.contains(QStringLiteral("held")), qPrintable(err));
        }
    }

    // What the measured board cannot deliver, and evdev chord collisions.
    void boardWarningsForThePad()
    {
        QString err;
        auto c = parseConfig(R"({"device": {"input": "evdev"}, "profiles": [{"name": "g",
            "layers": [{"name": "ws", "when": {"held": ["key1", "key16"]}, "bindings": {"knob1": {"turn": "a"}, "knob2": {"ccw": "b", "cw": "c"}, "key7": "d"}}],
            "bindings": {"knob1": {"shift": {"turn": "x"}}, "knob3": {"shift": {"cw": "y", "ccw": "z"}}}}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        const BoardProfile pad = *builtinBoardProfile(QStringLiteral("sy181-15k3e"));
        QVERIFY(!pad.turnsWhilePressed);
        QVERIFY(pad.knobs[0].pressPinsEncoder && pad.knobs[1].pressPinsEncoder && !pad.knobs[2].pressPinsEncoder);
        QStringList w = boardWarnings(*c, pad);
        w.sort();
        QCOMPARE(w.size(), 5);
        QVERIFY2(w[0].startsWith(QStringLiteral("profile g knob1.shift.turn: never fires: pressing knob1 holds one of its encoder lines low")), qPrintable(w[0]));
        QVERIFY2(w[1].startsWith(QStringLiteral("profile g knob3.shift.")) && w[1].contains(QStringLiteral("firmware ignores turns while a knob is pressed")), qPrintable(w[1]));
        // key1 = F14 and knob2 ccw = Alt+F14; key1 and key7 = Shift+F14.
        QVERIFY2(w[2].startsWith(QStringLiteral("profile g layer ws: \"held\": the sy181-15k3e layout has no key16")), qPrintable(w[2]));
        QVERIFY2(w[3].startsWith(QStringLiteral("profile g layer ws: key7: with \"input\": \"evdev\", key1 held and key7 share the pad's F14 key")), qPrintable(w[3]));
        QVERIFY2(w[4].startsWith(QStringLiteral("profile g layer ws: knob2.ccw: with \"input\": \"evdev\", key1 held and knob2.ccw share")), qPrintable(w[4]));
        for (const QString &x : std::as_const(w)) {
            const ConfigIssue i = describeConfigIssue(x);
            QCOMPARE(i.profile, QStringLiteral("g"));
        }
        QCOMPARE(describeConfigIssue(w[0]).slot, QStringLiteral("knob1.shift.turn"));
        QCOMPARE(describeConfigIssue(w[4]).layer, QStringLiteral("ws"));
        QCOMPARE(describeConfigIssue(w[4]).slot, QStringLiteral("knob2.ccw"));
        // Raw input (auto) has no collisions; a board that turns while pressed has no shift warnings.
        c->device.input = QStringLiteral("auto");
        QCOMPARE(boardWarnings(*c, pad).size(), 3);
        BoardProfile other = gridProfile(QStringLiteral("generic-16k3e"), QStringLiteral("x"), 16, 3, 4);
        QCOMPARE(boardWarnings(*c, other), QStringList{});
        // One at a time (the TM1650 matrix), and evdev modifiers that carry over.
        c = parseConfig(R"({"device": {"input": "evdev"}, "profiles": [{"name": "g", "layers": [
            {"name": "a", "when": {"held": "key13+knob3"}, "bindings": {"knob1": {"turn": "a"}}},
            {"name": "b", "when": {"held": "key13"}, "bindings": {"key5": "x", "knob2.press": "y", "knob1": {"turn": "w"}}},
            {"name": "c", "when": {"held": "key1+knob3"}, "bindings": {"knob1.cw": "v"}}]}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        QVERIFY(pad.oneAtATime.contains(QStringLiteral("key2")) && pad.oneAtATime.contains(QStringLiteral("knob3")) && !pad.oneAtATime.contains(QStringLiteral("key1")));
        w = boardWarnings(*c, pad);
        QCOMPARE(w.size(), 4);
        QVERIFY2(w[0].startsWith(QStringLiteral("profile g layer a: \"held\": key13+knob3 can never be held together")), qPrintable(w[0]));
        QVERIFY2(w[1].startsWith(QStringLiteral("profile g layer b: ")) && w[1].contains(QStringLiteral("never fires: pressing")), qPrintable(w[1]));
        QVERIFY2(w[2].startsWith(QStringLiteral("profile g layer b: ")) && w[2].contains(QStringLiteral("reports key13 released")), qPrintable(w[2]));
        // knob3 = Alt+F18 held: key1 (F14) arrives as Alt+F14, which is knob2 ccw.
        QVERIFY2(w[3].startsWith(QStringLiteral("profile g layer c: key1: with \"input\": \"evdev\", key1 while knob3 is held reads as knob2.ccw (the held key's alt")), qPrintable(w[3]));
        QCOMPARE(describeConfigIssue(w[3]).slot, QStringLiteral("key1"));
        c->device.input = QStringLiteral("raw");
        QCOMPARE(boardWarnings(*c, pad).size(), 3);  // the matrix limits stay
        // Held together on one F-key in evdev: key1 = F14, key13 = Ctrl+F14.
        c = parseConfig(R"({"device": {"input": "evdev"}, "profiles": [{"name": "g", "layers": [
            {"name": "d", "when": {"held": "key1+key13"}, "bindings": {"knob1": {"turn": "a"}}}]}]})", {}, &err);
        QVERIFY2(c, qPrintable(err));
        w = boardWarnings(*c, pad);
        QCOMPARE(w.size(), 1);
        QVERIFY2(w[0].startsWith(QStringLiteral("profile g layer d: \"held\": with \"input\": \"evdev\", key1 and key13 share the pad's F14 key")), qPrintable(w[0]));

        // The shipped example asks for nothing the pad cannot do.
        QFile ex(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"));
        QVERIFY(ex.open(QIODevice::ReadOnly));
        auto exc = parseConfig(ex.readAll(), {}, &err);
        QVERIFY2(exc, qPrintable(err));
        QCOMPARE(boardWarnings(*exc, pad), QStringList{});
        exc->device.input = QStringLiteral("evdev");
        QCOMPARE(boardWarnings(*exc, pad), QStringList{});
    }
};

QTEST_GUILESS_MAIN(TestConfig)
#include "tst_config.moc"
