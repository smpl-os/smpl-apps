// SPDX-License-Identifier: GPL-2.0-or-later
#include "hardwaremap.h"
#include "learn.h"

#include <QJsonDocument>
#include <QTemporaryDir>
#include <QTest>
#include <linux/input-event-codes.h>

using namespace cs;

class TestHardwareMap : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void keyNames()
    {
        QCOMPARE(keyCodeFromName(QStringLiteral("F14")), KEY_F14);
        QCOMPARE(keyCodeFromName(QStringLiteral("key_f14")), KEY_F14);
        QCOMPARE(keyCodeFromName(QStringLiteral("kpasterisk")), KEY_KPASTERISK);
        QCOMPARE(keyCodeFromName(QStringLiteral("nope")), -1);
        auto c = parseChord(QStringLiteral("Ctrl+Shift+z"));
        QVERIFY(c);
        QCOMPARE(c->mods, quint8(Mod::Ctrl | Mod::Shift));
        QCOMPARE(c->key, KEY_Z);
        QCOMPARE(chordName(*c), QStringLiteral("ctrl+shift+Z"));
        QCOMPARE(parseChord(QStringLiteral("super+1"))->mods, quint8(Mod::Meta));
        QCOMPARE(parseChord(QStringLiteral("shift"))->key, KEY_LEFTSHIFT);
        QString err;
        QVERIFY(!parseChord(QStringLiteral("hyper+a"), &err));
        QVERIFY(err.contains(QStringLiteral("hyper")));
        auto seq = parseChordSequence(QStringLiteral("ctrl+k x"));
        QCOMPARE(seq->size(), 2);
    }
    void defaultMapCoversAllControls()
    {
        const auto m = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        QCOMPARE(m.size(), 24);
        QCOMPARE(m.lookup(KeyChord{0, KEY_F14})->name(), QStringLiteral("key1"));
        QCOMPARE(m.lookup(KeyChord{Mod::Shift, KEY_F19})->name(), QStringLiteral("key12"));
        QCOMPARE(m.lookup(KeyChord{Mod::Ctrl, KEY_F16})->name(), QStringLiteral("key15"));
        QCOMPARE(m.lookup(KeyChord{Mod::Ctrl, KEY_F17})->name(), QStringLiteral("knob1.ccw"));
        QCOMPARE(m.lookup(KeyChord{Mod::Alt, KEY_F19})->name(), QStringLiteral("knob3.cw"));
        const auto b = HardwareMap::fromScheme(ch552::Numbering::VendorTwelve);
        QCOMPARE(b.lookup(KeyChord{Mod::Ctrl, KEY_F14})->name(), QStringLiteral("knob1.ccw"));
        QCOMPARE(b.lookup(KeyChord{Mod::Alt, KEY_F19})->name(), QStringLiteral("key15"));
    }
    void jsonRoundTrip()
    {
        const auto m = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        const auto back = HardwareMap::fromJson(m.toJson());
        QVERIFY(back);
        QCOMPARE(back->size(), 24);
        for (const auto &c : m.chords()) {
            QCOMPARE(*back->lookup(c), *m.lookup(c));
        }
        QString err;
        QVERIFY(!HardwareMap::fromJson(QJsonObject{}, &err));
        QVERIFY(!HardwareMap::fromJson(QJsonDocument::fromJson("{\"chords\":{\"F14\":\"knob1\"}}").object(), &err));
        QVERIFY(!HardwareMap::fromJson(QJsonDocument::fromJson("{\"chords\":{\"F14\":\"key1.cw\"}}").object(), &err));
        QVERIFY(HardwareMap::fromJson(QJsonDocument::fromJson("{\"chords\":{\"F14\":\"knob2.press\"}}").object(), &err));
    }
    void learnEvaluation()
    {
        const auto targets = learnTargets(*builtinBoardProfile(QStringLiteral("sy181-15k3e")));
        QCOMPARE(targets.size(), 24);
        QHash<int, KeyChord> captured;
        const auto a = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        // Simulate a pad that follows numbering A exactly.
        for (int i = 0; i < targets.size(); ++i) {
            for (const auto &c : a.chords()) {
                if (*a.lookup(c) == PadTarget{targets[i].control, targets[i].role}) {
                    captured.insert(i, c);
                }
            }
        }
        auto r = evaluateLearn(targets, captured);
        QCOMPARE(r.captured, 24);
        QCOMPARE(r.numbering, QStringLiteral("keys-then-knobs"));
        QVERIFY(r.complete());
        // Swap two knob directions: custom numbering, still complete.
        std::swap(captured[15], captured[16]);
        r = evaluateLearn(targets, captured);
        QCOMPARE(r.numbering, QStringLiteral("custom"));
        QVERIFY(r.complete());
        // A duplicate and a foreign chord are reported.
        captured[0] = captured[1];
        captured[2] = KeyChord{0, KEY_A};
        r = evaluateLearn(targets, captured);
        QCOMPARE(r.duplicates.size(), 1);
        QCOMPARE(r.foreign.size(), 1);
        QVERIFY(!r.complete());

        QTemporaryDir dir;
        captured.remove(0);
        captured.remove(2);
        r = evaluateLearn(targets, captured);
        QString err;
        const QString path = dir.filePath(QStringLiteral("hw.json"));
        QVERIFY(writeHardwareMap(r, path, &err));
        QVERIFY(writeHardwareMap(r, path, &err));  // second write backs up the first
        QCOMPARE(QDir(dir.path()).entryList({QStringLiteral("hw.json.bak-*")}).size(), 1);
    }
};

QTEST_GUILESS_MAIN(TestHardwareMap)
#include "tst_hardwaremap.moc"
