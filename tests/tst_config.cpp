// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"
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
        QCOMPARE(cfg->device.serial, QStringLiteral("key153"));
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
        QVERIFY(!parseConfig("{\"profiles\":[{\"name\":\"x\",\"bindings\":{\"key16\":\"a\"}}]}", {}, &err));
        QVERIFY(err.contains(QStringLiteral("key16")));
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
};

QTEST_GUILESS_MAIN(TestConfig)
#include "tst_config.moc"
