// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"
#include "featurelist.h"
#include "learn.h"
#include "configwatcher.h"

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
        QCOMPARE(cfg->profiles.size(), 3);
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
        QCOMPARE(kd->layers.at(0).bindings.value(QStringLiteral("knob2.shift.turn")).options.value(QStringLiteral("step")).toString(), QStringLiteral("fine"));
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
        QCOMPARE(kd->bindings.value(QStringLiteral("key1")).keys.value(0).key, KEY_I);
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
        QCOMPARE(c->cheatsheet.opacity, 0.85);
        QCOMPARE(c->cheatsheet.autoHideMs, 0);
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
        QCOMPARE(c->cheatsheet.autoHideMs, 4000);
        QCOMPARE(c->cheatsheet.position, QStringLiteral("top-right"));
        for (const char *bad : {R"({"cheatsheet":{"opacity":0},"profiles":[]})", R"({"cheatsheet":{"opacity":1.5},"profiles":[]})",
                                R"({"cheatsheet":{"autoHideMs":-1},"profiles":[]})", R"({"cheatsheet":{"position":"middle"},"profiles":[]})",
                                R"({"cheatsheet":{"color":"red"},"profiles":[]})", R"({"cheatsheet":true,"profiles":[]})"}) {
            QVERIFY2(!parseConfig(bad, {}, &err), bad);
            QVERIFY(err.startsWith(QStringLiteral("cheatsheet")));
        }
    }
};

QTEST_GUILESS_MAIN(TestConfig)
#include "tst_config.moc"
