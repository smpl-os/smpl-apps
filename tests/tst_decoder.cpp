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
};

QTEST_GUILESS_MAIN(TestDecoder)
#include "tst_decoder.moc"
