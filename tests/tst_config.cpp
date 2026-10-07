// SPDX-License-Identifier: GPL-2.0-or-later
#include "config.h"

#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
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
        QCOMPARE(kd->layers.size(), 4);
        QCOMPARE(cfg->profileFor(QStringLiteral("org.kde.kdenlive.automation-preview"), {})->name, QStringLiteral("kdenlive"));
        QCOMPARE(cfg->profileFor(QStringLiteral("firefox"), {})->name, QStringLiteral("global"));
        const Binding jog = kd->bindings.value(QStringLiteral("knob1.turn"));
        QCOMPARE(jog.kind, Binding::Control);
        QCOMPARE(jog.name, QStringLiteral("playhead.jog"));
        QCOMPARE(jog.keys.size(), 2);
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
    }
};

QTEST_GUILESS_MAIN(TestConfig)
#include "tst_config.moc"
