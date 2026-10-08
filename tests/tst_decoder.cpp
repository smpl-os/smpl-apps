// SPDX-License-Identifier: GPL-2.0-or-later
#include "decoder.h"

#include <QTest>
#include <linux/input-event-codes.h>

using namespace cs;

class TestDecoder : public QObject
{
    Q_OBJECT
private Q_SLOTS:
    void modifierAndKeyInOneFrame()
    {
        Decoder d;
        // The pad sends modifier bits first in the report, so evdev orders them first.
        QVERIFY(d.feed(EV_KEY, KEY_LEFTSHIFT, 1, 1).isEmpty());
        auto down = d.feed(EV_KEY, KEY_F16, 1, 2);
        QCOMPARE(down.size(), 1);
        QCOMPARE(down[0].chord, (KeyChord{Mod::Shift, KEY_F16}));
        QVERIFY(down[0].down);
        QVERIFY(d.feed(EV_SYN, SYN_REPORT, 0, 2).isEmpty());
        // Release: modifier goes up before the key; the chord must not change.
        QVERIFY(d.feed(EV_KEY, KEY_LEFTSHIFT, 0, 3).isEmpty());
        auto up = d.feed(EV_KEY, KEY_F16, 0, 4);
        QCOMPARE(up.size(), 1);
        QCOMPARE(up[0].chord, (KeyChord{Mod::Shift, KEY_F16}));
        QVERIFY(!up[0].down);
        QCOMPARE(d.heldModifiers(), quint8(0));
    }
    void autorepeatIgnored()
    {
        Decoder d;
        d.feed(EV_KEY, KEY_F14, 1, 0);
        QVERIFY(d.feed(EV_KEY, KEY_F14, 2, 1).isEmpty());
        QVERIFY(d.feed(EV_MSC, MSC_SCAN, 0x70069, 1).isEmpty());
    }
    void rightModifiersCountAsSame()
    {
        Decoder d;
        d.feed(EV_KEY, KEY_RIGHTCTRL, 1, 0);
        QCOMPARE(d.feed(EV_KEY, KEY_F19, 1, 0)[0].chord, (KeyChord{Mod::Ctrl, KEY_F19}));
    }
    void resetDropsState()
    {
        Decoder d;
        d.feed(EV_KEY, KEY_LEFTALT, 1, 0);
        d.reset();
        QCOMPARE(d.feed(EV_KEY, KEY_F14, 1, 0)[0].chord, (KeyChord{0, KEY_F14}));
    }
    void mapsToControls()
    {
        const auto map = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        // slot 1 = F14 = key1
        auto e = toPadEvent(map, ChordEvent{KeyChord{0, KEY_F14}, true, 0});
        QVERIFY(e);
        QCOMPARE(e->control, QStringLiteral("key1"));
        QCOMPARE(e->type, PadEvent::KeyDown);
        QCOMPARE(toPadEvent(map, ChordEvent{KeyChord{0, KEY_F14}, false, 0})->type, PadEvent::KeyUp);
        // slot 16 = ctrl+F17 = knob1 ccw, slot 18 = ctrl+F19 = knob1 cw, slot 17 = ctrl+F18 = knob1 press
        auto ccw = toPadEvent(map, ChordEvent{KeyChord{Mod::Ctrl, KEY_F17}, true, 0});
        QCOMPARE(ccw->control, QStringLiteral("knob1"));
        QCOMPARE(ccw->type, PadEvent::Turn);
        QCOMPARE(ccw->delta, -1);
        QCOMPARE(toPadEvent(map, ChordEvent{KeyChord{Mod::Ctrl, KEY_F19}, true, 0})->delta, 1);
        QVERIFY(!toPadEvent(map, ChordEvent{KeyChord{Mod::Ctrl, KEY_F19}, false, 0}));  // detent release
        QCOMPARE(toPadEvent(map, ChordEvent{KeyChord{Mod::Ctrl, KEY_F18}, true, 0})->type, PadEvent::PressDown);
        // slot 24 = alt+F19 = knob3 cw
        QCOMPARE(toPadEvent(map, ChordEvent{KeyChord{Mod::Alt, KEY_F19}, true, 0})->control, QStringLiteral("knob3"));
        QVERIFY(!toPadEvent(map, ChordEvent{KeyChord{0, KEY_A}, true, 0}));
    }

    void twoKeysHeldTogether()
    {
        // evdev as hid-input reports the pad's keyboard reports (modifier field
        // first, then the key array). key1 = F14 held while key8 = Shift+F15
        // is tapped twice: exact pairs, key1 released last.
        const auto map = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        Decoder d;
        QStringList got;
        auto feed = [&](int code, int value) {
            for (const ChordEvent &c : d.feed(EV_KEY, code, value, 0)) {
                if (const auto e = toPadEvent(map, c)) {
                    got << e->describe();
                }
            }
        };
        feed(KEY_F14, 1);                                  // [F14]
        for (int i = 0; i < 2; ++i) {
            feed(KEY_LEFTSHIFT, 1);                        // [Shift F14 F15]
            feed(KEY_F15, 1);
            feed(KEY_LEFTSHIFT, 0);                        // [F14] (key1 needs no Shift)
            feed(KEY_F15, 0);
        }
        feed(KEY_F14, 0);                                  // []
        QCOMPARE(got, (QStringList{QStringLiteral("key1 down"), QStringLiteral("key8 down"), QStringLiteral("key8 up"), QStringLiteral("key8 down"),
                                   QStringLiteral("key8 up"), QStringLiteral("key1 up")}));

        // The documented evdev limit: key7 = Shift+F14 shares F14 with key1.
        // The firmware cannot press F14 twice, so held together they cannot
        // be told apart (raw mode can).
        got.clear();
        d.reset();
        feed(KEY_F14, 1);                                  // key1: [F14]
        feed(KEY_LEFTSHIFT, 1);                            // key7: [Shift F14], F14 already down
        feed(KEY_LEFTSHIFT, 0);                            // key7 up: the firmware drops F14 and Shift
        feed(KEY_F14, 0);
        QCOMPARE(got, (QStringList{QStringLiteral("key1 down"), QStringLiteral("key1 up")}));
    }

    // Held-key layers in evdev mode: the held chord's modifier stays in the
    // pad's report, so a knob turned meanwhile carries it too. key8 = Shift+F15
    // held, knob2 ccw = Alt+F14 arrives as Shift+Alt+F14: still knob2 ccw.
    void heldModifierDoesNotHideTheTurn()
    {
        const auto map = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        Decoder d;
        QStringList got;
        auto feed = [&](int code, int value) {
            for (const ChordEvent &c : d.feed(EV_KEY, code, value, 0)) {
                if (const auto e = toPadEvent(map, c)) {
                    got << e->describe();
                }
            }
        };
        feed(KEY_LEFTSHIFT, 1);                            // key8: [Shift F15]
        feed(KEY_F15, 1);
        feed(KEY_LEFTALT, 1);                              // knob2 ccw: [Shift Alt F15 F14]
        feed(KEY_F14, 1);
        feed(KEY_F14, 0);                                  // the firmware keeps Shift for key8
        feed(KEY_LEFTALT, 0);
        feed(KEY_LEFTCTRL, 1);                             // knob1 cw: Ctrl+F19 -> Shift+Ctrl+F19
        feed(KEY_F19, 1);
        feed(KEY_F19, 0);
        feed(KEY_LEFTCTRL, 0);
        feed(KEY_F15, 0);
        feed(KEY_LEFTSHIFT, 0);
        QCOMPARE(got, (QStringList{QStringLiteral("key8 down"), QStringLiteral("knob2 turn -1"), QStringLiteral("knob1 turn +1"), QStringLiteral("key8 up")}));
        // A chord that maps as it is keeps its meaning: key1 (F14) held, then
        // knob1 ccw (Ctrl+F17) is Ctrl+F17.
        got.clear();
        feed(KEY_F14, 1);
        feed(KEY_LEFTCTRL, 1);
        feed(KEY_F17, 1);
        QCOMPARE(got, (QStringList{QStringLiteral("key1 down"), QStringLiteral("knob1 turn -1")}));
    }

    // A node that closes (unplug) or drops events releases what was down, as
    // inferred releases: holds end, deferred taps do not fire.
    void releaseAllIsSynthetic()
    {
        const auto map = HardwareMap::fromScheme(ch552::Numbering::KeysThenKnobs);
        Decoder d;
        d.feed(EV_KEY, KEY_F14, 1, 0);                     // key1
        d.feed(EV_KEY, KEY_LEFTCTRL, 1, 0);
        d.feed(EV_KEY, KEY_F18, 1, 0);                     // knob1 press
        const auto ups = d.releaseAll();
        QCOMPARE(ups.size(), 2);
        QStringList got;
        for (const ChordEvent &c : ups) {
            QVERIFY(!c.down);
            QVERIFY(c.synthetic);
            const auto e = toPadEvent(map, c);
            QVERIFY(e);
            QVERIFY(e->synthetic);
            got << e->describe();
        }
        got.sort();
        QCOMPARE(got, (QStringList{QStringLiteral("key1 up"), QStringLiteral("knob1 release")}));
        QVERIFY(d.releaseAll().isEmpty());
        QCOMPARE(d.heldModifiers(), quint8(0));
    }
};

QTEST_GUILESS_MAIN(TestDecoder)
#include "tst_decoder.moc"
