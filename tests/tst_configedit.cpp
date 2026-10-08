// SPDX-License-Identifier: GPL-2.0-or-later
// `control-surfaced set/get` core: the in-place JSONC editor, option values,
// and set/get on a file (validated, backed up, written atomically).
#include "config.h"
#include "configedit.h"
#include "configstore.h"

#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QTemporaryDir>
#include <QTest>

using namespace cs;

namespace {
QByteArray readFile(const QString &path)
{
    QFile f(path);
    return f.open(QIODevice::ReadOnly) ? f.readAll() : QByteArray();
}
void writeFile(const QString &path, const QByteArray &data)
{
    QFile f(path);
    QVERIFY(f.open(QIODevice::WriteOnly));
    f.write(data);
}
QByteArray set(const QByteArray &text, const QString &path, const QJsonValue &v)
{
    QString err;
    const auto out = setJsoncValue(text, path.split(QLatin1Char('.')), v, &err);
    if (!out) {
        qWarning() << "setJsoncValue failed:" << err;
        return {};
    }
    return *out;
}
QByteArray example()
{
    return readFile(QStringLiteral(CS_SOURCE_DIR "/data/config.example.jsonc"));
}
} // namespace

class TestConfigEdit : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void replaceKeepsEverythingElse()
    {
        const QByteArray in = "// head\n{\n    \"device\": { \"serial\": \"\", \"input\": \"auto\" },  // pad\n"
                              "    /* \"input\": \"no\" */ \"x\": 1\n}\n";
        QCOMPARE(set(in, QStringLiteral("device.input"), QStringLiteral("raw")),
                 QByteArray("// head\n{\n    \"device\": { \"serial\": \"\", \"input\": \"raw\" },  // pad\n"
                            "    /* \"input\": \"no\" */ \"x\": 1\n}\n"));
        // An object value replaced by a scalar, and the other way round.
        QCOMPARE(set("{\"a\": {\"b\": [1, {\"c\": 2}]}}", QStringLiteral("a"), 5), QByteArray("{\"a\": 5}"));
        QCOMPARE(jsoncValue(set("{\"a\": 5}", QStringLiteral("a"), QJsonObject{{QStringLiteral("w"), true}}), {QStringLiteral("a"), QStringLiteral("w")}),
                 QJsonValue(true));
        // Duplicate keys: the last one is what the parser uses, so it is the one changed.
        QCOMPARE(set("{\"a\": 1, \"a\": 2}", QStringLiteral("a"), 3), QByteArray("{\"a\": 1, \"a\": 3}"));
    }

    void stringsAndCommentsAreNotStructure()
    {
        const QByteArray in = "{\n    \"match\": \"a{b:c}\\\"}\", // \"input\": {\n    \"device\": { /* } */ \"input\": \"auto\" }\n}\n";
        const QByteArray out = set(in, QStringLiteral("device.input"), QStringLiteral("evdev"));
        QCOMPARE(out, QByteArray(in).replace("\"auto\"", "\"evdev\""));
        QCOMPARE(jsoncValue(out, {QStringLiteral("match")}), QJsonValue(QStringLiteral("a{b:c}\"}")));
    }

    void insertMultiLine()
    {
        // A comment after the last member stays on its line.
        QCOMPARE(set("{\n    \"settings\": {\n        \"a\": 1,\n        \"b\": 2  // bee\n    }\n}\n", QStringLiteral("settings.c"), 3),
                 QByteArray("{\n    \"settings\": {\n        \"a\": 1,\n        \"b\": 2,  // bee\n        \"c\": 3\n    }\n}\n"));
        // A trailing comma style is kept.
        QCOMPARE(set("{\n    \"a\": 1,\n    \"b\": 2, // bee\n}\n", QStringLiteral("c"), 3),
                 QByteArray("{\n    \"a\": 1,\n    \"b\": 2, // bee\n    \"c\": 3,\n}\n"));
        // A block comment running over the line end is not split.
        QCOMPARE(set("{\n    \"b\": 2 /* one\n    two */\n}\n", QStringLiteral("c"), 3),
                 QByteArray("{\n    \"b\": 2, /* one\n    two */\n    \"c\": 3\n}\n"));
        // The object closes on the last member's line.
        QCOMPARE(set("{\n    \"a\": 1,\n    \"b\": 2 }", QStringLiteral("c"), 3), QByteArray("{\n    \"a\": 1,\n    \"b\": 2,\n    \"c\": 3 }"));
    }

    void insertInlineAndEmpty()
    {
        QCOMPARE(set("{ \"device\": { \"serial\": \"\" } }", QStringLiteral("device.input"), QStringLiteral("raw")),
                 QByteArray("{ \"device\": { \"serial\": \"\", \"input\": \"raw\" } }"));
        QCOMPARE(set("{ \"device\": { \"serial\": \"\", } }", QStringLiteral("device.input"), QStringLiteral("raw")),
                 QByteArray("{ \"device\": { \"serial\": \"\", \"input\": \"raw\", } }"));
        QCOMPARE(set("{\"device\": {}}", QStringLiteral("device.input"), QStringLiteral("raw")), QByteArray("{\"device\": { \"input\": \"raw\" }}"));
        QCOMPARE(set("{}", QStringLiteral("device.input"), QStringLiteral("raw")), QByteArray("{ \"device\": { \"input\": \"raw\" } }"));
    }

    void insertMissingParent()
    {
        const QByteArray in = "{\n    \"profiles\": []\n}\n";
        const QByteArray out = set(in, QStringLiteral("cheatsheet.opacity"), 0.5);
        QCOMPARE(out, QByteArray("{\n    \"profiles\": [],\n    \"cheatsheet\": { \"opacity\": 0.5 }\n}\n"));
        QCOMPARE(jsoncValue(out, {QStringLiteral("cheatsheet"), QStringLiteral("opacity")}), QJsonValue(0.5));
    }

    // Review findings: an object holding only comments keeps them; CRLF files
    // stay CRLF; a UTF-8 BOM is accepted as the parser does.
    void commentOnlyObjects()
    {
        QCOMPARE(set("{\n    \"settings\": {\n        // \"accelFactor\": 3,   too fast\n        // \"keyRateHz\": 240\n    },\n    \"profiles\": []\n}\n",
                     QStringLiteral("settings.keyRateHz"), 200),
                 QByteArray("{\n    \"settings\": {\n        // \"accelFactor\": 3,   too fast\n        // \"keyRateHz\": 240\n        \"keyRateHz\": 200\n    },\n"
                            "    \"profiles\": []\n}\n"));
        QCOMPARE(set("{\n  // my notes\n}", QStringLiteral("device.input"), QStringLiteral("raw")),
                 QByteArray("{\n  // my notes\n  \"device\": { \"input\": \"raw\" }\n}"));
        QCOMPARE(set("{ \"d\": { /* note */ } }", QStringLiteral("d.input"), QStringLiteral("raw")), QByteArray("{ \"d\": { /* note */ \"input\": \"raw\" } }"));
        QCOMPARE(set("{\n    \"d\": { /* note */\n    }\n}", QStringLiteral("d.k"), 1), QByteArray("{\n    \"d\": { /* note */\n        \"k\": 1\n    }\n}"));
        for (const QByteArray &t : {QByteArray("{\n  // my notes\n}"), QByteArray("{ \"d\": { /* note */ } }")}) {
            QVERIFY(QJsonDocument::fromJson(stripJsonComments(set(t, QStringLiteral("d.k"), 1))).isObject());
        }
    }

    void lineEndingsAndBom()
    {
        QCOMPARE(set("{\r\n    \"b\": 2\r\n}\r\n", QStringLiteral("c"), 3), QByteArray("{\r\n    \"b\": 2,\r\n    \"c\": 3\r\n}\r\n"));
        QCOMPARE(set("{\r\n    \"b\": 2 // x\r\n}\r\n", QStringLiteral("c"), 3), QByteArray("{\r\n    \"b\": 2, // x\r\n    \"c\": 3\r\n}\r\n"));
        QCOMPARE(set("{\r\n    // only\r\n}\r\n", QStringLiteral("c"), 3), QByteArray("{\r\n    // only\r\n    \"c\": 3\r\n}\r\n"));
        const QByteArray bom = "\xEF\xBB\xBF{ \"device\": { \"input\": \"auto\" } }";
        QCOMPARE(set(bom, QStringLiteral("device.input"), QStringLiteral("raw")), QByteArray(bom).replace("auto", "raw"));
        QCOMPARE(jsoncValue(bom, {QStringLiteral("device"), QStringLiteral("input")}), QJsonValue(QStringLiteral("auto")));
    }

    // cheatsheet.eww may be an object ({window, config, ...}): set switches its
    // "enabled" and keeps the rest.
    void ewwObjectIsKept()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("config.jsonc"));
        const QByteArray text = "{ \"profiles\": [],\n  \"cheatsheet\": { \"opacity\": 0.5,\n    // my own eww setup\n"
                                "    \"eww\": { \"window\": \"my-sheet\", \"config\": \"~/dots/eww\" } } }\n";
        writeFile(path, text);
        QCOMPARE(getOption(path, QStringLiteral("cheatsheet.eww")).value(QStringLiteral("effective")), QJsonValue(true));
        OptionChange r = setOption(path, QStringLiteral("cheatsheet.eww"), QStringLiteral("true"));
        QVERIFY(r.ok);
        QVERIFY(!r.changed);  // enabled defaults to true
        r = setOption(path, QStringLiteral("cheatsheet.eww"), QStringLiteral("false"));
        QVERIFY2(r.ok, qPrintable(r.message));
        QVERIFY(r.changed);
        QCOMPARE(r.oldValue, QJsonValue(true));
        QCOMPARE(readFile(path), QByteArray(text).replace("\"config\": \"~/dots/eww\" }", "\"config\": \"~/dots/eww\", \"enabled\": false }"));
        QCOMPARE(getOption(path, QStringLiteral("cheatsheet.eww")).value(QStringLiteral("effective")), QJsonValue(false));
        r = setOption(path, QStringLiteral("cheatsheet.eww"), QStringLiteral("true"));
        QVERIFY(r.changed);
        QCOMPARE(jsoncValue(readFile(path), {QStringLiteral("cheatsheet"), QStringLiteral("eww"), QStringLiteral("window")}), QJsonValue(QStringLiteral("my-sheet")));
        QCOMPARE(jsoncValue(readFile(path), {QStringLiteral("cheatsheet"), QStringLiteral("eww"), QStringLiteral("enabled")}), QJsonValue(true));
        QVERIFY(readFile(path).contains("// my own eww setup"));
    }

    void errors()
    {
        QString err;
        QVERIFY(!setJsoncValue("{\"cheatsheet\": true}", {QStringLiteral("cheatsheet"), QStringLiteral("opacity")}, 0.5, &err));
        QCOMPARE(err, QStringLiteral("cheatsheet is not an object"));
        QVERIFY(!setJsoncValue("[1]", {QStringLiteral("a")}, 1, &err));
        QCOMPARE(err, QStringLiteral("the config is not a JSON object"));
        QVERIFY(!setJsoncValue("{\"a\": ", {QStringLiteral("a")}, 1, &err));
        QVERIFY(!setJsoncValue("{}", {}, 1, &err));
        QVERIFY(jsoncValue("{\"a\": 1}", {QStringLiteral("b")}).isUndefined());
        QVERIFY(jsoncValue("not json", {QStringLiteral("a")}).isUndefined());
    }

    void optionValues()
    {
        QString err;
        const auto input = optionSpec(QStringLiteral("input"));
        QVERIFY(input);
        QCOMPARE(*parseOptionValue(*input, QStringLiteral("keymap"), &err), QJsonValue(QStringLiteral("evdev")));
        QCOMPARE(*parseOptionValue(*input, QStringLiteral("Automatic"), &err), QJsonValue(QStringLiteral("auto")));
        QCOMPARE(*parseOptionValue(*input, QStringLiteral("RAW"), &err), QJsonValue(QStringLiteral("raw")));
        QVERIFY(!parseOptionValue(*input, QStringLiteral("fast"), &err));
        QVERIFY(err.contains(QStringLiteral("auto, evdev, raw")));
        const auto labels = input->toJson().value(QStringLiteral("labels")).toObject();
        QCOMPARE(labels.value(QStringLiteral("auto")).toString(), QStringLiteral("Automatic"));
        QCOMPARE(labels.value(QStringLiteral("evdev")).toString(), QStringLiteral("Keymap (compatible)"));
        QCOMPARE(labels.value(QStringLiteral("raw")).toString(), QStringLiteral("Raw (fastest)"));

        const auto opacity = optionSpec(QStringLiteral("cheatsheet.opacity"));
        QCOMPARE(*parseOptionValue(*opacity, QStringLiteral("0.5"), &err), QJsonValue(0.5));
        QVERIFY(!parseOptionValue(*opacity, QStringLiteral("2"), &err));
        QVERIFY(!parseOptionValue(*opacity, QStringLiteral("nan"), &err));
        const auto hide = optionSpec(QStringLiteral("cheatsheet.autoHideMs"));
        QCOMPARE(*parseOptionValue(*hide, QStringLiteral("0"), &err), QJsonValue(0));
        QVERIFY(!parseOptionValue(*hide, QStringLiteral("1.5"), &err));
        const auto eww = optionSpec(QStringLiteral("cheatsheet.eww"));
        QCOMPARE(*parseOptionValue(*eww, QStringLiteral("true"), &err), QJsonValue(true));
        QVERIFY(!parseOptionValue(*eww, QStringLiteral("yes please"), &err));
        QCOMPARE(*parseOptionValue(*optionSpec(QStringLiteral("serial")), QString(), &err), QJsonValue(QString()));
        QVERIFY(!optionSpec(QStringLiteral("profiles")));
    }

    // Every option set on the shipped example changes that one value, and the
    // result still validates.
    void everyOptionOnTheExample()
    {
        const QByteArray ex = example();
        QVERIFY(!ex.isEmpty());
        const QHash<QString, QString> sample{
            {QStringLiteral("input"), QStringLiteral("raw")},          {QStringLiteral("serial"), QStringLiteral("key153")},
            {QStringLiteral("cheatsheet.opacity"), QStringLiteral("0.6")}, {QStringLiteral("cheatsheet.autoHideMs"), QStringLiteral("0")},
            {QStringLiteral("cheatsheet.position"), QStringLiteral("top")}, {QStringLiteral("cheatsheet.eww"), QStringLiteral("false")},
            {QStringLiteral("settings.accelFactor"), QStringLiteral("2")}, {QStringLiteral("settings.accelWindowMs"), QStringLiteral("50")},
            {QStringLiteral("settings.keyRateHz"), QStringLiteral("200")},
        };
        QTemporaryDir dir;
        const ConfigStore store(dir.filePath(QStringLiteral("config.jsonc")));
        for (const OptionSpec &spec : settableOptions()) {
            QVERIFY2(sample.contains(spec.key), qPrintable(spec.key));
            QString err;
            const auto v = parseOptionValue(spec, sample.value(spec.key), &err);
            QVERIFY2(v, qPrintable(err));
            const QByteArray out = set(ex, spec.path.join(QLatin1Char('.')), *v);
            QVERIFY2(!out.isEmpty(), qPrintable(spec.key));
            QCOMPARE(jsoncValue(out, spec.path), *v);
            const auto check = store.validate(out);
            QVERIFY2(check.ok, qPrintable(spec.key + QLatin1Char(' ') + check.errors.join(QLatin1Char(';'))));
            // One line changed (and at most one added).
            QList<QByteArray> removed = ex.split('\n'), added;
            for (const QByteArray &l : out.split('\n')) {
                if (!removed.removeOne(l)) {
                    added << l;
                }
            }
            QVERIFY2(removed.size() <= 1 && added.size() <= 2, qPrintable(spec.key));
        }
    }

    // The example a person opens: short header, profiles first, then the
    // advanced settings with the input mode in plain sight.
    void exampleIsOrdered()
    {
        const QByteArray ex = example();
        const qsizetype profiles = ex.indexOf("\"profiles\"");
        const qsizetype advanced = ex.indexOf("// ---- Advanced");
        const qsizetype device = ex.indexOf("\"device\"");
        QVERIFY(profiles > 0 && advanced > profiles && device > advanced);
        QVERIFY(ex.indexOf("\"cheatsheet\": {") > advanced && ex.indexOf("\"settings\"") > advanced);
        QVERIFY(ex.left(profiles).count('\n') <= 15);  // the header stays short
        QVERIFY(ex.contains("control-surfaced set input"));
        QVERIFY(ex.contains("docs/config-reference.md"));
        QCOMPARE(jsoncValue(ex, {QStringLiteral("device"), QStringLiteral("input")}), QJsonValue(QStringLiteral("auto")));
        QVERIFY(!ex.contains("\"shift\""));  // the firmware ignores turns while a knob is held
        QVERIFY(QFile::exists(QStringLiteral(CS_SOURCE_DIR "/docs/config-reference.md")));
    }

    void setOptionOnFile()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("config.jsonc"));
        writeFile(path, example());
        const QString before = ConfigStore::hashOf(example());

        OptionChange r = setOption(path, QStringLiteral("input"), QStringLiteral("keymap"));
        QVERIFY2(r.ok, qPrintable(r.message));
        QVERIFY(r.changed);
        QCOMPARE(r.oldValue, QJsonValue(QStringLiteral("auto")));
        QCOMPARE(r.newValue, QJsonValue(QStringLiteral("evdev")));
        QCOMPARE(r.hash, ConfigStore::hashOf(readFile(path)));
        QVERIFY(!r.backup.isEmpty());
        QCOMPARE(ConfigStore::hashOf(readFile(r.backup)), before);
        QCOMPARE(jsoncValue(readFile(path), {QStringLiteral("device"), QStringLiteral("input")}), QJsonValue(QStringLiteral("evdev")));
        QCOMPARE(r.toJson().value(QStringLiteral("new")).toString(), QStringLiteral("evdev"));

        // Already so: nothing written.
        const QString hash = r.hash;
        r = setOption(path, QStringLiteral("input"), QStringLiteral("evdev"));
        QVERIFY(r.ok);
        QVERIFY(!r.changed);
        QCOMPARE(ConfigStore::hashOf(readFile(path)), hash);

        r = setOption(path, QStringLiteral("input"), QStringLiteral("fast"));
        QVERIFY(!r.ok);
        QCOMPARE(r.code, QStringLiteral("invalid-value"));
        r = setOption(path, QStringLiteral("profiles"), QStringLiteral("x"));
        QCOMPARE(r.code, QStringLiteral("unknown-option"));
        QVERIFY(r.message.contains(QStringLiteral("input")));
        QCOMPARE(ConfigStore::hashOf(readFile(path)), hash);

        // A file the option cannot go into is left alone.
        writeFile(path, "{ \"profiles\": [], \"cheatsheet\": true }");
        r = setOption(path, QStringLiteral("cheatsheet.opacity"), QStringLiteral("0.5"));
        QCOMPARE(r.code, QStringLiteral("invalid-config"));
        QCOMPARE(readFile(path), QByteArray("{ \"profiles\": [], \"cheatsheet\": true }"));

        const QJsonObject g = getOption(path, QStringLiteral("cheatsheet.opacity"));
        QVERIFY(g.value(QStringLiteral("ok")).toBool());
        QVERIFY(g.value(QStringLiteral("value")).isNull());
        QCOMPARE(g.value(QStringLiteral("default")).toDouble(), CheatsheetOptions().opacity);
        QCOMPARE(g.value(QStringLiteral("effective")).toDouble(), CheatsheetOptions().opacity);
        QVERIFY(!getOption(path, QStringLiteral("nope")).value(QStringLiteral("ok")).toBool());
    }

    void setOptionCreatesTheFile()
    {
        QTemporaryDir dir;
        const QString path = dir.filePath(QStringLiteral("config.jsonc"));
        const OptionChange r = setOption(path, QStringLiteral("input"), QStringLiteral("raw"));
        QVERIFY2(r.ok, qPrintable(r.message));
        QVERIFY(r.changed);
        QVERIFY(r.backup.isEmpty());
        QCOMPARE(jsoncValue(readFile(path), {QStringLiteral("device"), QStringLiteral("input")}), QJsonValue(QStringLiteral("raw")));
        const QJsonObject g = getOption(path, QStringLiteral("input"));
        QCOMPARE(g.value(QStringLiteral("value")).toString(), QStringLiteral("raw"));
        QCOMPARE(g.value(QStringLiteral("default")).toString(), QStringLiteral("auto"));
        QCOMPARE(g.value(QStringLiteral("file")).toString(), path);
    }
};

QTEST_GUILESS_MAIN(TestConfigEdit)
#include "tst_configedit.moc"
