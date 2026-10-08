// SPDX-License-Identifier: CC-BY-SA-3.0
// Same licence as the firmware it tests (firmware/LICENSE.upstream).
// Host tests for the pad firmware's input and keymap logic (firmware/src/padlogic.c).
//
// The firmware links the same file; here the platform hooks record what the pad
// would have sent, so decoding, dispatch, raw mode and layers can be checked
// without hardware. A small simulator runs the encoder interrupt every 250 us
// and the main loop every 1 ms, and can make each USB report block the main
// loop like the real endpoint does. Includes fast-spin timing tests and a
// seeded soak run of random physical input.

#include "padlogic.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures = 0;
static int checks = 0;

#define CHECK(cond)                                                              \
    do {                                                                         \
        ++checks;                                                                \
        if (!(cond)) {                                                           \
            ++failures;                                                          \
            fprintf(stderr, "%s:%d: CHECK failed: %s\n", __FILE__, __LINE__, #cond); \
        }                                                                        \
    } while (0)

#define CHECK_EQ(a, b)                                                           \
    do {                                                                         \
        long long va_ = (long long)(a), vb_ = (long long)(b);                    \
        ++checks;                                                                \
        if (va_ != vb_) {                                                        \
            ++failures;                                                          \
            fprintf(stderr, "%s:%d: CHECK_EQ failed: %s = %lld, %s = %lld\n",    \
                    __FILE__, __LINE__, #a, va_, #b, vb_);                       \
        }                                                                        \
    } while (0)

// ---------------------------------------------------------------------------------
// Recording platform
// ---------------------------------------------------------------------------------

enum { OUT_PRESS = 1, OUT_RELEASE, OUT_RAW };

typedef struct {
    int kind;
    uint16_t packed;
    uint8_t seq, slot, event, layer, count;
    uint16_t at;  // Pad.nowMs when it went out
} out_t;

#define OUT_MAX 8192
static out_t outs[OUT_MAX];
static int outCount = 0;
static uint16_t keymap[LAYER_COUNT][SLOT_COUNT];

// USB model: each report keeps the main loop busy this many ms (0 = free).
// The firmware polls at 1 ms (bInterval 1); 2.0.0 used 10 ms.
static int reportMs = 0;
static int blockedMs = 0;

uint16_t PAD_keymap(uint8_t layer, uint8_t slot)
{
    return keymap[layer][slot];
}

static void record(out_t o)
{
    o.at = Pad.nowMs;
    if (outCount < OUT_MAX) {
        outs[outCount] = o;
    }
    ++outCount;
}

static int reportsFor(uint16_t packed)
{
    // A key chord with modifiers goes out as two reports (modifiers, then key).
    return ((packed >> 13) == ACT_KEY && ((packed >> 8) & 0x1F)) ? 2 : 1;
}

void PAD_hwPress(uint16_t packed)
{
    out_t o = {OUT_PRESS, packed, 0, 0, 0, 0, 0, 0};
    record(o);
    blockedMs += reportMs * reportsFor(packed);
}

void PAD_hwRelease(uint16_t packed)
{
    out_t o = {OUT_RELEASE, packed, 0, 0, 0, 0, 0, 0};
    record(o);
    blockedMs += reportMs * reportsFor(packed);
}

void PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer, uint8_t count)
{
    out_t o = {OUT_RAW, 0, seq, slot, event, layer, count, 0};
    record(o);
    blockedMs += reportMs;
}

static int locked = 0;
void PAD_hwLock(void)
{
    ++locked;
}

void PAD_hwUnlock(void)
{
    --locked;
}

// ---------------------------------------------------------------------------------
// Physical pad and timing model
// ---------------------------------------------------------------------------------

#define TM_IDLE 0x2E

typedef struct {
    uint8_t gpioKey;                    // 1 = key 1 down
    uint8_t tm;                         // TM1650 register
    uint8_t a[KNOB_COUNT], b[KNOB_COUNT];
} phys_t;

static phys_t phys;
static long quarters = 0;               // simulated time in 250 us steps
static int missedMs = 0;                // main-loop passes a busy report took
// The firmware's millisecond clock: 8 bits, counted by the timer interrupt.
// The main loop passes the wrap-around difference to PAD_tick, as padfw.c.
static uint8_t msTicks = 0, lastTicks = 0;
static int watchRaw = 0;                // count main-loop passes that find raw mode off
static int rawOffPasses = 0;

static uint8_t pins(void)
{
    uint8_t p = 0;
    int k;
    for (k = 0; k < KNOB_COUNT; ++k) {
        p |= (uint8_t)((phys.b[k] ? 1 : 0) << (2 * k));
        p |= (uint8_t)((phys.a[k] ? 1 : 0) << (2 * k + 1));
    }
    return p;
}

static void physRest(void)
{
    int i;
    phys.gpioKey = 0;
    phys.tm = TM_IDLE;
    for (i = 0; i < KNOB_COUNT; ++i) {
        phys.a[i] = 1;
        phys.b[i] = 1;
    }
}

// One 250 us step: the timer interrupt, and every 4th step the main loop
// (unless a report is still keeping it busy).
static void quarter(void)
{
    CHECK_EQ(locked, 0);                // the interrupt never lands inside a lock here
    PAD_encoderIsr(pins());
    if (++quarters % 4) {
        return;
    }
    ++msTicks;
    if (blockedMs > 0) {
        --blockedMs;
        ++missedMs;
        return;
    }
    {
        uint8_t elapsed = msTicks;
        elapsed -= lastTicks;
        lastTicks = msTicks;
        CHECK_EQ(elapsed, 1 + missedMs);
        PAD_tick(elapsed);
        if (watchRaw && !PAD_rawActive(&Pad.raw)) {
            ++rawOffPasses;
        }
    }
    missedMs = 0;
    PAD_poll(phys.gpioKey, phys.tm);
}

static void quarters_(int n)
{
    while (n-- > 0) {
        quarter();
    }
}

// n milliseconds = n main-loop passes when nothing blocks.
static void poll(int ms)
{
    quarters_(4 * ms);
}

static int idle(void)
{
    return PAD_queued() == 0 && Pad.tapSlot == SLOT_NONE && blockedMs == 0;
}

// Let queued detents finish typing; returns the ms it took (-1 if never).
// Idle must hold over a few main-loop passes that really ran, so detents
// still in the interrupt's accumulator are collected too.
static int settle(int maxMs)
{
    int ms = 0, quiet = 0;
    while (quiet < 3) {
        if (ms++ >= maxMs) {
            return -1;
        }
        poll(1);
        quiet = (idle() && missedMs == 0) ? quiet + 1 : 0;
    }
    return ms - 3;
}

static void loadDefaults(void)
{
    int l, s;
    for (l = 0; l < LAYER_COUNT; ++l) {
        for (s = 0; s < SLOT_COUNT; ++s) {
            keymap[l][s] = PAD_defaultAction((uint8_t)l, (uint8_t)s);
        }
    }
}

static void reset(void)
{
    physRest();
    loadDefaults();
    reportMs = 0;
    blockedMs = 0;
    missedMs = 0;
    lastTicks = msTicks;                // the clock keeps running across tests, as on the pad
    PAD_init(0, phys.gpioKey, phys.tm, pins());
    outCount = 0;
}

// Elapsed time in pieces PAD_tick's 8-bit parameter can carry.
static void tickMs(int ms)
{
    while (ms > 0) {
        const uint8_t step = ms > 200 ? 200 : (uint8_t)ms;
        PAD_tick(step);
        ms -= step;
    }
}

static uint16_t key(uint8_t mods, uint8_t usage)
{
    uint16_t p = 0;
    PAD_encode(ACT_KEY, mods, usage, &p);
    return p;
}

// Encoder states, (A << 1) | B. Clockwise: 3 -> 1 -> 0 -> 2 -> 3.
static const uint8_t CW_PATH[4] = {1, 0, 2, 3};
static const uint8_t CCW_PATH[4] = {2, 0, 1, 3};

static void setEnc(int knob, uint8_t state)
{
    phys.a[knob] = (state >> 1) & 1;
    phys.b[knob] = state & 1;
}

static uint8_t encOf(int knob)
{
    return (uint8_t)((phys.a[knob] << 1) | phys.b[knob]);
}

// One detent, each of the 4 states held qPerState quarter-ms steps. With
// bounce > 0 the changing line chatters for that many steps at each edge.
static void turnQ(int knob, int cw, int qPerState, int bounce)
{
    int i, j;
    for (i = 0; i < 4; ++i) {
        const uint8_t from = encOf(knob), to = cw ? CW_PATH[i] : CCW_PATH[i];
        for (j = 0; j < bounce && j < qPerState - 1; ++j) {
            setEnc(knob, (j & 1) ? from : to);
            quarter();
        }
        setEnc(knob, to);
        quarters_(qPerState - (bounce < qPerState - 1 ? bounce : qPerState - 1));
    }
}

static void turn(int knob, int cw)
{
    turnQ(knob, cw, 4, 0);  // 1 ms per state
}

static int countKind(int kind)
{
    int i, n = 0;
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        n += outs[i].kind == kind;
    }
    return n;
}

static int countPacked(int kind, uint16_t packed)
{
    int i, n = 0;
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        n += outs[i].kind == kind && outs[i].packed == packed;
    }
    return n;
}

static int countRaw(uint8_t slot, uint8_t event)
{
    int i, n = 0;
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        n += outs[i].kind == OUT_RAW && outs[i].slot == slot && outs[i].event == event;
    }
    return n;
}

// Detents in raw TAP events for a slot (counts summed).
static int rawDetents(uint8_t slot)
{
    int i, n = 0;
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        if (outs[i].kind == OUT_RAW && outs[i].slot == slot && outs[i].event == RAW_EVT_TAP) {
            n += outs[i].count;
        }
    }
    return n;
}

// ---------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------

static void test_keycode_map(void)
{
    // Rows as captured, holding the pad with the knobs on the right.
    static const uint8_t rows[3][5] = {
        {0x00, 0x44, 0x4C, 0x54, 0x5C},     // key 1 is GPIO P1.5
        {0x64, 0x45, 0x4D, 0x55, 0x5D},
        {0x65, 0x46, 0x4E, 0x56, 0x5E},
    };
    int seen[SLOT_COUNT] = {0};
    int r, c, code;

    for (r = 0; r < 3; ++r) {
        for (c = 0; c < 5; ++c) {
            if (r == 0 && c == 0) {
                continue;
            }
            CHECK_EQ(PAD_slotForKeycode(rows[r][c]), r * 5 + c);
        }
    }
    CHECK_EQ(PAD_slotForKeycode(0x67), SLOT_KNOB(0, KNOB_PRESS));
    CHECK_EQ(PAD_slotForKeycode(0x5F), SLOT_KNOB(1, KNOB_PRESS));
    CHECK_EQ(PAD_slotForKeycode(0x57), SLOT_KNOB(2, KNOB_PRESS));
    CHECK_EQ(SLOT_KNOB(0, KNOB_CCW), 15);
    CHECK_EQ(SLOT_KNOB(2, KNOB_CW), 23);

    // Every pressed code maps to at most one slot, and no two codes share one.
    for (code = 0; code < 256; ++code) {
        uint8_t slot = PAD_slotForKeycode((uint8_t)code);
        if (!(code & 0x40)) {
            CHECK_EQ(slot, SLOT_NONE);          // releases and idle never map
        }
        if (slot != SLOT_NONE) {
            CHECK(slot < SLOT_COUNT);
            CHECK(slot != SLOT_GPIO_KEY);
            ++seen[slot];
        }
    }
    for (r = 0; r < SLOT_COUNT; ++r) {
        const int isTurn = r >= 15 && (r - 15) % 3 != KNOB_PRESS;
        CHECK_EQ(seen[r], (r == SLOT_GPIO_KEY || isTurn) ? 0 : 1);
    }
    CHECK_EQ(PAD_slotForKeycode(TM_IDLE), SLOT_NONE);
    CHECK_EQ(PAD_slotForKeycode(0x04), SLOT_NONE);
    CHECK_EQ(PAD_slotForKeycode(0x66), SLOT_NONE);  // not fitted on this board
    CHECK_EQ(PAD_slotForKeycode(0x47), SLOT_NONE);
}

static void test_codec(void)
{
    uint16_t p = 0;
    uint8_t type, mod;
    uint16_t code;
    unsigned v;

    // Round trips through the compact form.
    struct {
        uint8_t type, mod;
        uint16_t code;
    } good[] = {
        {ACT_NONE, 0, 0},
        {ACT_KEY, 0x00, 0x69},
        {ACT_KEY, 0x02, 0x6E},
        {ACT_KEY, 0x0F, 0x04},
        {ACT_KEY, 0x20, 0x4F},                  // right shift
        {ACT_KEY, 0xF0, 0xE7},
        {ACT_KEY, 0x00, 0xE0},                  // a modifier on its own
        {ACT_CON, 0, 0xCD},
        {ACT_CON, 0, 0x3FF},
        {ACT_MOUSE, 0, (MS_BUTTON << 8) | 0x01},
        {ACT_MOUSE, 0, (MS_BUTTON << 8) | 0x1F},
        {ACT_MOUSE, 0, (MS_WHEEL << 8) | 0xFF},
        {ACT_MOUSE, 0, (MS_MOVE_Y << 8) | 0x80},
        {ACT_LAYER, 0, (LAYER_OP_TOGGLE << 8) | 0},
        {ACT_LAYER, 0, (LAYER_OP_MOMENTARY << 8) | 1},
        {ACT_LAYER, 0, (LAYER_OP_SET << 8) | 1},
    };
    for (v = 0; v < sizeof(good) / sizeof(good[0]); ++v) {
        CHECK(PAD_validAction(good[v].type, good[v].mod, good[v].code));
        CHECK(PAD_encode(good[v].type, good[v].mod, good[v].code, &p));
        CHECK(PAD_validPacked(p));
        PAD_decode(p, &type, &mod, &code);
        CHECK_EQ(type, good[v].type);
        CHECK_EQ(mod, good[v].mod);
        CHECK_EQ(code, good[v].code);
    }

    struct {
        uint8_t type, mod;
        uint16_t code;
    } bad[] = {
        {ACT_NONE, 1, 0},
        {ACT_NONE, 0, 1},
        {ACT_KEY, 0, 0},
        {ACT_KEY, 0, 0xE8},                     // past the keyboard page
        {ACT_KEY, 0, 0x100},
        {ACT_KEY, 0x12, 0x04},                  // left ctrl + right ctrl
        {ACT_CON, 1, 0xCD},
        {ACT_CON, 0, 0},
        {ACT_CON, 0, 0x400},
        {ACT_MOUSE, 1, (MS_WHEEL << 8) | 1},
        {ACT_MOUSE, 0, (MS_BUTTON << 8) | 0},
        {ACT_MOUSE, 0, (MS_BUTTON << 8) | 0x20},
        {ACT_MOUSE, 0, (MS_WHEEL << 8) | 0},
        {ACT_MOUSE, 0, (5 << 8) | 1},
        {ACT_LAYER, 1, 0},
        {ACT_LAYER, 0, 2},                      // only layers 0 and 1
        {ACT_LAYER, 0, (3 << 8) | 0},
        {5, 0, 1},
        {7, 0, 1},
        {255, 0, 1},
    };
    for (v = 0; v < sizeof(bad) / sizeof(bad[0]); ++v) {
        p = 0xBEEF;
        CHECK(!PAD_validAction(bad[v].type, bad[v].mod, bad[v].code));
        CHECK(!PAD_encode(bad[v].type, bad[v].mod, bad[v].code, &p));
        CHECK_EQ(p, 0xBEEF);                    // untouched on failure
    }

    // Stored words: exactly the encodable ones are valid; junk bits are not.
    {
        unsigned valid = 0;
        for (v = 0; v <= 0xFFFF; ++v) {
            if (PAD_validPacked((uint16_t)v)) {
                uint16_t again = 0;
                ++valid;
                PAD_decode((uint16_t)v, &type, &mod, &code);
                CHECK(type == ACT_NONE ? v == 0 : PAD_encode(type, mod, code, &again) && again == v);
            }
        }
        CHECK(valid > 1000);
    }
    CHECK(!PAD_validPacked(0xFFFF));            // erased flash
    CHECK(!PAD_validPacked(0x1000));            // ACT_NONE with stray bits
    CHECK(!PAD_validPacked((uint16_t)((ACT_CON << 13) | (1 << 10) | 0xCD)));
    CHECK(!PAD_validPacked((uint16_t)((ACT_LAYER << 13) | (1 << 12))));
}

static void test_crc(void)
{
    static const uint8_t check[] = "123456789";
    uint8_t zeros[96] = {0};
    CHECK_EQ(PAD_crc8(check, 9), 0xFB);         // poly 0x07, init 0xFF, no reflection
    CHECK_EQ(PAD_crc8(check, 0), 0xFF);
    CHECK_EQ(PAD_crc8(zeros, 96), 0x7D);
    zeros[50] = 1;
    CHECK(PAD_crc8(zeros, 96) != 0x7D);
}

static void test_defaults(void)
{
    int l, s, t;
    uint8_t type, mod;
    uint16_t code;
    static const uint8_t mods[4] = {0x00, 0x02, 0x01, 0x04};    // none, LShift, LCtrl, LAlt

    for (l = 0; l < LAYER_COUNT; ++l) {
        for (s = 0; s < SLOT_COUNT; ++s) {
            CHECK(PAD_validPacked(PAD_defaultAction((uint8_t)l, (uint8_t)s)));
            CHECK(PAD_defaultAction((uint8_t)l, (uint8_t)s) != 0);
            for (t = 0; t < s; ++t) {
                CHECK(PAD_defaultAction((uint8_t)l, (uint8_t)s) != PAD_defaultAction((uint8_t)l, (uint8_t)t));
            }
        }
    }
    // Layer 0 is exactly the daemon's scheme (src/proto/scheme.cpp).
    for (s = 0; s < SLOT_COUNT; ++s) {
        PAD_decode(PAD_defaultAction(0, (uint8_t)s), &type, &mod, &code);
        CHECK_EQ(type, ACT_KEY);
        CHECK_EQ(mod, mods[s / 6]);
        CHECK_EQ(code, 0x69 + s % 6);
    }
    // Layer 1 can always get back to layer 0.
    {
        int ways = 0;
        for (s = 0; s < SLOT_COUNT; ++s) {
            PAD_decode(PAD_defaultAction(1, (uint8_t)s), &type, &mod, &code);
            ways += type == ACT_LAYER && ((code >> 8) == LAYER_OP_TOGGLE || code == ((LAYER_OP_SET << 8) | 0));
        }
        CHECK(ways >= 1);
    }
    CHECK_EQ(PAD_defaultAction(2, 0), 0);
    CHECK_EQ(PAD_defaultAction(0, SLOT_COUNT), 0);
}

// Feeds one knob's states straight into the interrupt decoder (the other knobs at rest).
static int encFeed(int knob, const uint8_t *states, int n, int held)
{
    int i, steps = 0;
    for (i = 0; i < n; ++i) {
        uint8_t p = 0x3F & (uint8_t)~(3 << (2 * knob));
        p |= (uint8_t)(states[i] << (2 * knob));
        PAD_encoderSetHeld(held ? (uint8_t)(1 << knob) : 0);
        PAD_encoderIsr(p);
    }
    {
        const uint16_t t = PAD_encoderTake((uint8_t)knob);
        steps = ENC_TAKE_CW(t) - ENC_TAKE_CCW(t);
    }
    CHECK_EQ(PAD_encoderTake((uint8_t)knob) & 0x7F7F, 0);  // taking clears
    return steps;
}

static void test_encoder(void)
{
    int k;
    for (k = 0; k < KNOB_COUNT; ++k) {
        static const uint8_t cw[] = {1, 0, 2, 3};
        static const uint8_t ccw[] = {2, 0, 1, 3};
        static const uint8_t cw3[] = {1, 0, 2};
        static const uint8_t rest[] = {3};
        reset();
        // One clean detent each way, complete on reaching rest.
        CHECK_EQ(encFeed(k, cw3, 3, 0), 0);
        CHECK_EQ(encFeed(k, rest, 1, 0), 1);
        CHECK_EQ(encFeed(k, ccw, 4, 0), -1);
        CHECK_EQ(encFeed(k, cw, 4, 0) + encFeed(k, cw, 4, 0), 2);
        // Repeated samples of the same state change nothing.
        CHECK_EQ(encFeed(k, rest, 1, 0), 0);
    }

    // Knob press: B low and back, alone, with contact bounce, with A chatter.
    reset();
    {
        static const uint8_t press[] = {2, 3};
        static const uint8_t bouncy[] = {2, 3, 2, 3, 2, 2, 2, 3, 2, 3};
        static const uint8_t pressA[] = {2, 0, 2, 0, 2, 3};
        CHECK_EQ(encFeed(0, press, 2, 0), 0);
        CHECK_EQ(encFeed(0, bouncy, 10, 0), 0);
        CHECK_EQ(encFeed(0, pressA, 6, 0), 0);
    }
    // Rotating while the switch is held: nothing, and nothing left over after.
    {
        static const uint8_t heldTurn[] = {2, 0, 2, 0, 1, 0, 2, 3};
        static const uint8_t after[] = {1, 0, 2, 3};
        CHECK_EQ(encFeed(1, heldTurn, 8, 1), 0);
        CHECK_EQ(encFeed(1, after, 4, 0), 1);  // the next real detent counts in full
    }
    // Press, a half turn while held, the line returns with the switch still held.
    {
        static const uint8_t press[] = {2};
        static const uint8_t heldHalf[] = {0, 1, 3};
        CHECK_EQ(encFeed(0, press, 1, 0) + encFeed(0, heldHalf, 3, 1), 0);
    }
    // B low before the TM1650 reports the switch, released after; and the reverse.
    {
        static const uint8_t down[] = {2};
        static const uint8_t up[] = {3};
        CHECK_EQ(encFeed(2, down, 1, 0) + encFeed(2, up, 1, 1), 0);
        CHECK_EQ(encFeed(2, down, 1, 1) + encFeed(2, up, 1, 0), 0);
    }
    // Bounce at the leading edge still gives exactly one step.
    {
        static const uint8_t edge[] = {1, 3, 1, 3, 1, 0, 1, 0, 2, 0, 2, 3, 2, 3};
        CHECK_EQ(encFeed(0, edge, 14, 0), 1);
    }
    // One skipped state (fast spin) still counts, in the right direction.
    {
        static const uint8_t skipMid[] = {1, 2, 3};
        static const uint8_t skipFirst[] = {0, 2, 3};
        static const uint8_t skipLastCcw[] = {2, 0, 3};
        CHECK_EQ(encFeed(0, skipMid, 3, 0), 1);
        CHECK_EQ(encFeed(0, skipFirst, 3, 0), 1);
        CHECK_EQ(encFeed(0, skipLastCcw, 3, 0), -1);
    }
    // The rest state itself missed between two detents: both still count.
    // (2.0.0 only counted on reaching rest and lost one of these.)
    {
        static const uint8_t noRest[] = {1, 0, 2, 1, 0, 2, 3};
        static const uint8_t noRestCcw[] = {2, 0, 1, 2, 0, 1, 3};
        static const uint8_t threeNoRest[] = {1, 0, 2, 1, 0, 2, 1, 0, 2, 3};
        CHECK_EQ(encFeed(1, noRest, 7, 0), 2);
        CHECK_EQ(encFeed(1, noRestCcw, 7, 0), -2);
        CHECK_EQ(encFeed(1, threeNoRest, 10, 0), 3);
    }
    // Two states missed carries no direction: dropped, never reversed; counted.
    reset();
    {
        static const uint8_t jump[] = {0, 3};
        padstats_t st;
        CHECK_EQ(encFeed(0, jump, 2, 0), 0);
        PAD_getStats(&st, 1);
        CHECK_EQ(st.illegal[0], 2);
        CHECK_EQ(st.illegal[1], 0);
        PAD_getStats(&st, 0);
        CHECK_EQ(st.illegal[0], 0);  // cleared
    }
    // Starting a detent and going back is not a step.
    {
        static const uint8_t back[] = {1, 0, 1, 3, 2, 0, 2, 3};
        CHECK_EQ(encFeed(0, back, 8, 0), 0);
    }
    // Long runs: exact, and the accumulator saturates instead of wrapping.
    reset();
    {
        static const uint8_t cw[] = {1, 0, 2, 3};
        int i, total = 0;
        padstats_t st;
        for (i = 0; i < 250; ++i) {
            total += encFeed(2, cw, 4, 0);
        }
        CHECK_EQ(total, 250);
        for (i = 0; i < 150; ++i) {   // nobody takes them
            uint8_t j;
            for (j = 0; j < 4; ++j) {
                PAD_encoderIsr((uint8_t)(0x0F | (cw[j] << 4)));
            }
        }
        CHECK_EQ(ENC_TAKE_CW(PAD_encoderTake(2)), ENC_ACC_MAX);
        PAD_getStats(&st, 0);
        CHECK_EQ(st.overruns, 50);
        CHECK_EQ(st.cw[2], 350);
        CHECK_EQ(st.ccw[2], 0);
    }
    // Starting away from rest (power-on mid-detent) does not invent a step.
    {
        static const uint8_t settleRest[] = {3};
        reset();
        PAD_encoderReset(0x00);
        CHECK_EQ(encFeed(0, settleRest, 1, 0), 0);
    }
}

static void test_debounce(void)
{
    debounce_t d;
    PAD_debounceInit(&d, 0);
    CHECK_EQ(PAD_debounce(&d, 1), 0);
    CHECK_EQ(PAD_debounce(&d, 1), 0);
    CHECK_EQ(PAD_debounce(&d, 1), 1);
    CHECK_EQ(d.stable, 1);
    CHECK_EQ(PAD_debounce(&d, 1), 0);
    // A one- or two-sample glitch never gets through.
    CHECK_EQ(PAD_debounce(&d, 0), 0);
    CHECK_EQ(PAD_debounce(&d, 1), 0);
    CHECK_EQ(PAD_debounce(&d, 0), 0);
    CHECK_EQ(PAD_debounce(&d, 0), 0);
    CHECK_EQ(PAD_debounce(&d, 1), 0);
    CHECK_EQ(d.stable, 1);
    // Alternating values never settle.
    PAD_debounceInit(&d, 0x2E);
    {
        int i, changes = 0;
        for (i = 0; i < 100; ++i) {
            changes += PAD_debounce(&d, (i & 1) ? 0x44 : 0x4C);
        }
        CHECK_EQ(changes, 0);
    }
}

static void test_keys(void)
{
    reset();
    // Key 1 is on the GPIO path.
    phys.gpioKey = 1;
    poll(2);
    CHECK_EQ(outCount, 0);                      // not yet debounced
    poll(1);
    CHECK_EQ(outCount, 1);
    CHECK_EQ(outs[0].kind, OUT_PRESS);
    CHECK_EQ(outs[0].packed, key(0, 0x69));
    poll(50);                                   // held: nothing repeats
    CHECK_EQ(outCount, 1);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(outCount, 2);
    CHECK_EQ(outs[1].kind, OUT_RELEASE);
    CHECK_EQ(outs[1].packed, key(0, 0x69));

    // A TM1650 key, then its release code.
    reset();
    phys.tm = 0x44;
    poll(3);
    phys.tm = 0x04;
    poll(3);
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(outCount, 2);
    CHECK_EQ(outs[0].kind, OUT_PRESS);
    CHECK_EQ(outs[0].packed, key(0, 0x6A));     // slot 1
    CHECK_EQ(outs[1].kind, OUT_RELEASE);
    CHECK_EQ(outs[1].packed, key(0, 0x6A));

    // Bottom-right key (slot 14) is ctrl+F16 on layer 0.
    reset();
    phys.tm = 0x5E;
    poll(3);
    CHECK_EQ(outCount, 1);
    CHECK_EQ(outs[0].packed, key(0x01, 0x6B));

    // Rolling from one TM1650 key to another releases the first.
    reset();
    phys.tm = 0x44;
    poll(3);
    phys.tm = 0x4C;
    poll(3);
    phys.tm = 0x0C;
    poll(3);
    CHECK_EQ(outCount, 4);
    CHECK_EQ(outs[1].kind, OUT_RELEASE);
    CHECK_EQ(outs[1].packed, key(0, 0x6A));
    CHECK_EQ(outs[2].kind, OUT_PRESS);
    CHECK_EQ(outs[2].packed, key(0, 0x6B));
    CHECK_EQ(outs[3].kind, OUT_RELEASE);

    // Key 1 and a TM1650 key really are independent.
    reset();
    phys.gpioKey = 1;
    phys.tm = 0x5D;
    poll(3);
    CHECK_EQ(countKind(OUT_PRESS), 2);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(countKind(OUT_RELEASE), 1);
    CHECK_EQ(countPacked(OUT_RELEASE, key(0, 0x69)), 1);
    phys.tm = 0x1D;
    poll(3);
    CHECK_EQ(countPacked(OUT_RELEASE, key(0x02, 0x6C)), 1);  // slot 9

    // Bouncing key: one press, one release.
    reset();
    {
        static const uint8_t bounce[] = {0x44, 0x04, 0x44, 0x04, 0x44, 0x44, 0x44, 0x44,
                                         0x04, 0x44, 0x04, 0x04, 0x04, 0x04};
        unsigned i;
        for (i = 0; i < sizeof(bounce); ++i) {
            phys.tm = bounce[i];
            poll(1);
        }
    }
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(countKind(OUT_RELEASE), 1);

    // Unknown codes do nothing, but do end a held key.
    reset();
    phys.tm = 0x47;
    poll(5);
    CHECK_EQ(outCount, 0);

    // Held at power-on: silent until released and pressed again.
    physRest();
    loadDefaults();
    phys.tm = 0x4C;
    phys.gpioKey = 1;
    PAD_init(0, phys.gpioKey, phys.tm, pins());
    outCount = 0;
    poll(10);
    phys.tm = TM_IDLE;
    phys.gpioKey = 0;
    poll(5);
    CHECK_EQ(outCount, 0);
    phys.tm = 0x4C;
    poll(3);
    CHECK_EQ(countKind(OUT_PRESS), 1);
}

static void test_knobs(void)
{
    // Top knob press pulls its B line low as well; only the press must come out.
    reset();
    phys.b[0] = 0;
    poll(1);
    phys.tm = 0x67;
    poll(5);
    phys.b[0] = 1;
    poll(1);
    phys.tm = 0x27;
    poll(5);
    CHECK_EQ(outCount, 2);
    CHECK_EQ(outs[0].packed, key(0x01, 0x6D));  // slot 16 = ctrl+F18
    CHECK_EQ(outs[1].kind, OUT_RELEASE);

    // Same with the B line bouncing and released before the switch report ends.
    reset();
    {
        int i;
        for (i = 0; i < 6; ++i) {
            phys.b[1] = (uint8_t)(i & 1);
            quarters_(2);
        }
        phys.b[1] = 0;
        phys.tm = 0x5F;
        poll(4);
        phys.b[1] = 1;
        quarters_(1);
        phys.b[1] = 0;
        quarters_(1);
        phys.b[1] = 1;
        poll(2);
        phys.tm = TM_IDLE;
        poll(4);
    }
    settle(100);
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6A)), 1);   // slot 19 = alt+F15
    CHECK_EQ(countKind(OUT_RELEASE), 1);

    // Turns are taps: press, at least PAD_TAP_GAP_MS, release, on the right slot.
    reset();
    turn(0, 1);
    CHECK(settle(100) >= 0);
    CHECK_EQ(outCount, 2);
    CHECK_EQ(outs[0].kind, OUT_PRESS);
    CHECK_EQ(outs[0].packed, key(0x01, 0x6E));  // slot 17 = top cw = ctrl+F19
    CHECK_EQ(outs[1].kind, OUT_RELEASE);
    CHECK_EQ(outs[1].packed, key(0x01, 0x6E));
    CHECK((uint16_t)(outs[1].at - outs[0].at) >= PAD_TAP_GAP_MS);
    reset();
    turn(1, 0);
    turn(2, 1);
    turn(2, 0);
    settle(100);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x69)), 1);   // slot 18 = middle ccw
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6E)), 1);   // slot 23 = bottom cw
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6C)), 1);   // slot 21 = bottom ccw
    CHECK_EQ(countKind(OUT_RELEASE), 3);

    // Keys keep working while detents are being typed.
    reset();
    {
        int i;
        for (i = 0; i < 10; ++i) {
            turnQ(0, 1, 2, 0);
        }
        phys.tm = 0x44;
        poll(4);
        CHECK_EQ(countPacked(OUT_PRESS, key(0, 0x6A)), 1);   // within 4 ms, not after the taps
        phys.tm = TM_IDLE;
        CHECK(settle(500) >= 0);
        CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6E)), 10);
    }

    // Turning while pressed does nothing on the turn slots.
    reset();
    phys.b[0] = 0;
    phys.tm = 0x67;
    poll(4);
    phys.a[0] = 0;
    poll(2);
    phys.a[0] = 1;
    poll(2);
    phys.a[0] = 0;
    poll(2);
    phys.a[0] = 1;
    poll(2);
    phys.b[0] = 1;
    phys.tm = TM_IDLE;
    poll(4);
    settle(100);
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6D)), 1);

    // Mouse wheel detents are one-shot (no release), one per detent.
    reset();
    PAD_setLayer(1);
    turn(1, 1);
    turn(1, 1);
    settle(100);
    CHECK_EQ(outCount, 2);
    CHECK_EQ(countKind(OUT_RELEASE), 0);
    {
        uint8_t type, mod;
        uint16_t code;
        PAD_decode(outs[0].packed, &type, &mod, &code);
        CHECK_EQ(type, ACT_MOUSE);
        CHECK_EQ(code, (MS_WHEEL << 8) | 0x01);
    }

    // An unbound detent is consumed silently.
    reset();
    keymap[0][SLOT_KNOB(0, KNOB_CW)] = 0;
    turn(0, 1);
    turn(0, 0);
    settle(100);
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(outs[0].packed, key(0x01, 0x6C));  // slot 15 = top ccw

    // The tap queue keeps runs in turning order; long runs merge, and only more
    // direction changes than it has runs for are dropped (and counted).
    reset();
    PAD_turn(0, 1, 100);
    PAD_turn(0, 1, 50);
    PAD_turn(0, 1, 120);  // 270 in one direction: two runs
    CHECK_EQ(PAD_queued(), 270);
    CHECK_EQ(Pad.runLen, 2);
    {
        int i;
        for (i = 0; i < PAD_QUEUE_RUNS; ++i) {
            PAD_turn(1, (uint8_t)(i & 1), 1);
        }
    }
    {
        padstats_t st;
        PAD_getStats(&st, 0);
        // 14 new runs fit; the 15th (ccw) is dropped; the 16th (cw) still
        // joins the cw run at the tail.
        CHECK_EQ(st.queueDrops, 1);
        CHECK_EQ(st.maxQueue, 270 + PAD_QUEUE_RUNS - 1);
    }
    CHECK(settle(20000) >= 0);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6E)), 270);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6B)), 8);  // middle cw
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x69)), 7);  // middle ccw
    // ... in the order turned: all the top knob first, then middle alternating.
    {
        int i, firstMiddle = -1, ok = 1;
        for (i = 0; i < outCount && i < OUT_MAX; ++i) {
            if (outs[i].kind != OUT_PRESS) {
                continue;
            }
            if (outs[i].packed != key(0x01, 0x6E) && firstMiddle < 0) {
                firstMiddle = i;
            }
            if (firstMiddle >= 0 && outs[i].packed == key(0x01, 0x6E)) {
                ok = 0;
            }
        }
        CHECK(ok);
    }
}

// The defect seen on hardware with 2.0.0: fast spins lost about half the detents.
// Here: realistic speeds and bounce, the USB report pace of 2.0.1 (1 ms) and of
// 2.0.0 (10 ms), keymap and raw mode. Every detent must come out, exactly once.
static uint32_t rng = 0x12345678u;
static uint32_t rnd(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

static void spin(int knob, int cw, int detents, int qMin, int qMax, int bounce)
{
    int i;
    for (i = 0; i < detents; ++i) {
        const int q = qMin + (int)(rnd() % (uint32_t)(qMax - qMin + 1));
        turnQ(knob, cw, q, bounce);
    }
}

static void test_fast_spin(void)
{
    // quarter-ms per state: 15 = 20 detents in 300 ms (3.75 ms per transition),
    // down to 2 = 0.5 ms per transition (a very hard flick).
    static const int speeds[][3] = {{15, 15, 2}, {12, 18, 3}, {8, 8, 2}, {5, 7, 1}, {3, 4, 1}, {2, 2, 0}};
    static const int paces[] = {0, 1, 10};
    unsigned s, p;
    int knob, raw;
    for (s = 0; s < sizeof(speeds) / sizeof(speeds[0]); ++s) {
        for (p = 0; p < sizeof(paces) / sizeof(paces[0]); ++p) {
            for (raw = 0; raw < 2; ++raw) {
                for (knob = 0; knob < KNOB_COUNT; ++knob) {
                    const int cw = (int)((s + p + (unsigned)knob) & 1);
                    const uint8_t slot = (uint8_t)SLOT_KNOB(knob, cw ? KNOB_CW : KNOB_CCW);
                    padstats_t st;
                    int took;
                    reset();
                    reportMs = paces[p];
                    if (raw) {
                        PAD_setRaw(10000);
                    }
                    spin(knob, cw, 20, speeds[s][0], speeds[s][1], speeds[s][2]);
                    took = settle(10000);
                    CHECK(took >= 0);
                    PAD_getStats(&st, 0);
                    CHECK_EQ(st.overruns + st.queueDrops, 0);
                    CHECK_EQ(cw ? st.cw[knob] : st.ccw[knob], 20);
                    CHECK_EQ(cw ? st.ccw[knob] : st.cw[knob], 0);
                    if (raw) {
                        CHECK_EQ(rawDetents(slot), 20);
                        CHECK(countKind(OUT_RAW) <= 20);
                        CHECK_EQ(countKind(OUT_RAW), outCount);
                    } else {
                        CHECK_EQ(countPacked(OUT_PRESS, keymap[0][slot]), 20);
                        CHECK_EQ(countPacked(OUT_RELEASE, keymap[0][slot]), 20);
                        CHECK_EQ(outCount, 40);
                        // At the 1 ms report pace typing keeps up with the
                        // 300 ms spin: the last detent goes out soon after.
                        if (paces[p] == 1 && s == 0) {
                            CHECK(took < 60);
                        }
                    }
                }
            }
        }
    }

    // Raw mode coalesces: a slow main loop sends fewer, larger reports.
    reset();
    PAD_setRaw(10000);
    reportMs = 10;
    spin(2, 1, 40, 2, 2, 0);
    settle(1000);
    CHECK_EQ(rawDetents(SLOT_KNOB(2, KNOB_CW)), 40);
    CHECK(countKind(OUT_RAW) < 40);

    // Back and forth quickly: every detent, in order, in keymap and raw mode.
    for (raw = 0; raw < 2; ++raw) {
        int i;
        reset();
        reportMs = 1;
        if (raw) {
            PAD_setRaw(10000);
        }
        for (i = 0; i < 6; ++i) {
            spin(0, i & 1, 1 + i, 2, 3, 0);
        }
        settle(2000);
        if (raw) {
            CHECK_EQ(rawDetents(SLOT_KNOB(0, KNOB_CCW)), 1 + 3 + 5);
            CHECK_EQ(rawDetents(SLOT_KNOB(0, KNOB_CW)), 2 + 4 + 6);
        } else {
            int n = 0, runs = 0;
            uint16_t prev = 0;
            CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(0, KNOB_CCW)]), 1 + 3 + 5);
            CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(0, KNOB_CW)]), 2 + 4 + 6);
            for (i = 0; i < outCount; ++i) {
                if (outs[i].kind == OUT_PRESS) {
                    runs += outs[i].packed != prev;
                    prev = outs[i].packed;
                    ++n;
                }
            }
            CHECK_EQ(n, 21);
            CHECK_EQ(runs, 6);  // typed in the order turned
        }
    }

    // A knob press in the middle of a fast spin: the press, and every detent
    // before and after it.
    reset();
    spin(1, 1, 8, 3, 4, 1);
    phys.b[1] = 0;
    quarters_(2);
    phys.tm = 0x5F;
    poll(6);
    phys.b[1] = 1;
    quarters_(3);
    phys.tm = TM_IDLE;
    poll(5);
    spin(1, 1, 8, 3, 4, 1);
    settle(2000);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(1, KNOB_CW)]), 16);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(1, KNOB_PRESS)]), 1);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(1, KNOB_CCW)]), 0);

    // All three knobs at once, fast, with the old 10 ms report pace.
    reset();
    reportMs = 10;
    {
        int i;
        for (i = 0; i < 4 * 15; ++i) {
            int k;
            for (k = 0; k < KNOB_COUNT; ++k) {
                setEnc(k, (k == 1 ? CCW_PATH : CW_PATH)[i % 4]);
            }
            quarters_(3);
        }
    }
    CHECK(settle(10000) >= 0);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(0, KNOB_CW)]), 15);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(1, KNOB_CCW)]), 15);
    CHECK_EQ(countPacked(OUT_PRESS, keymap[0][SLOT_KNOB(2, KNOB_CW)]), 15);
}

static void test_layers(void)
{
    uint16_t momentary = 0;

    // Layer 1 standalone: knob 3 press toggles back to layer 0 without output.
    reset();
    PAD_setLayer(1);
    phys.tm = 0x44;
    poll(3);
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(outs[0].packed, key(0, 0x69));     // layer 1 slot 1 = F14
    outCount = 0;
    phys.tm = 0x57;
    poll(3);
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(outCount, 0);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 0);
    phys.tm = 0x57;
    poll(3);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6D)), 1);   // layer 0 slot 22

    // A key held across a layer change releases what it pressed.
    reset();
    phys.tm = 0x44;
    poll(3);
    PAD_setLayer(1);
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(countPacked(OUT_RELEASE, key(0, 0x6A)), 1);

    // Momentary layer on key 1 (GPIO) shifts the knobs while held.
    reset();
    CHECK(PAD_encode(ACT_LAYER, 0, (LAYER_OP_MOMENTARY << 8) | 1, &momentary));
    keymap[0][0] = momentary;
    phys.gpioKey = 1;
    poll(3);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
    turn(0, 1);                                 // layer 1 top cw = volume up
    settle(100);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 0);
    turn(0, 1);
    settle(100);
    {
        uint16_t volUp = 0;
        PAD_encode(ACT_CON, 0, 0xE9, &volUp);
        CHECK_EQ(countPacked(OUT_PRESS, volUp), 1);
        CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6E)), 1);
    }

    // A momentary layer bound to a detent does nothing (press and release at once).
    reset();
    keymap[0][SLOT_KNOB(2, KNOB_CW)] = momentary;
    turn(2, 1);
    settle(100);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 0);
    CHECK_EQ(outCount, 0);

    // Set and out-of-range.
    PAD_setLayer(1);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
    PAD_setLayer(2);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
}

static void test_raw_mode(void)
{
    int i;

    // Raw events replace keymap output entirely.
    reset();
    CHECK(PAD_setRaw(1000));
    CHECK(PAD_rawActive(&Pad.raw));
    phys.tm = 0x44;
    poll(3);
    phys.tm = TM_IDLE;
    poll(3);
    turn(0, 0);
    settle(100);
    CHECK_EQ(countKind(OUT_PRESS), 0);
    CHECK_EQ(countKind(OUT_RELEASE), 0);
    CHECK_EQ(countKind(OUT_RAW), 3);
    CHECK_EQ(outs[0].slot, 1);
    CHECK_EQ(outs[0].event, RAW_EVT_DOWN);
    CHECK_EQ(outs[1].event, RAW_EVT_UP);
    CHECK_EQ(outs[2].slot, SLOT_KNOB(0, KNOB_CCW));
    CHECK_EQ(outs[2].event, RAW_EVT_TAP);
    CHECK_EQ(outs[0].seq, 1);
    CHECK_EQ(outs[1].seq, 2);
    CHECK_EQ(outs[2].seq, 3);
    CHECK_EQ(outs[2].layer, 0);
    CHECK_EQ(outs[2].count, 1);
    CHECK_EQ(outs[0].count, 1);

    // Knob press in raw mode: DOWN/UP on the press slot, no turn.
    outCount = 0;
    phys.b[2] = 0;
    phys.tm = 0x57;
    poll(4);
    phys.b[2] = 1;
    phys.tm = TM_IDLE;
    poll(4);
    CHECK_EQ(countRaw(SLOT_KNOB(2, KNOB_PRESS), RAW_EVT_DOWN), 1);
    CHECK_EQ(countRaw(SLOT_KNOB(2, KNOB_PRESS), RAW_EVT_UP), 1);
    CHECK_EQ(outCount, 2);

    // Heartbeats keep it alive; silence ends it.
    reset();
    CHECK(PAD_setRaw(1000));
    for (i = 0; i < 20; ++i) {
        tickMs(900);
        CHECK(PAD_rawActive(&Pad.raw));
        CHECK(PAD_setRaw(1000));
    }
    tickMs(999);
    CHECK(PAD_rawActive(&Pad.raw));
    PAD_tick(1);
    CHECK(!PAD_rawActive(&Pad.raw));
    tickMs(5000);
    CHECK(!PAD_rawActive(&Pad.raw));

    // Out-of-range requests are refused and change nothing; 0 turns it off.
    CHECK(PAD_setRaw(RAW_TIMEOUT_MAX_MS));
    CHECK(!PAD_setRaw(RAW_TIMEOUT_MAX_MS + 1));
    CHECK(PAD_rawActive(&Pad.raw));
    CHECK(PAD_setRaw(0));
    CHECK(!PAD_rawActive(&Pad.raw));

    // Entering raw mode releases what the keymap is holding; that key's later
    // release is silent.
    reset();
    phys.gpioKey = 1;
    poll(3);
    CHECK_EQ(countKind(OUT_PRESS), 1);
    PAD_setRaw(1000);
    CHECK_EQ(countKind(OUT_RELEASE), 1);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(countKind(OUT_RAW), 0);
    CHECK_EQ(countKind(OUT_RELEASE), 1);

    // Heartbeat lost with a key down: no raw UP later, no keymap release either,
    // and the next press goes through the keymap.
    reset();
    PAD_setRaw(500);
    phys.tm = 0x45;
    poll(3);
    CHECK_EQ(countRaw(6, RAW_EVT_DOWN), 1);
    tickMs(600);
    CHECK(!PAD_rawActive(&Pad.raw));
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(countKind(OUT_RAW), 1);
    CHECK_EQ(countKind(OUT_RELEASE), 0);
    phys.tm = 0x45;
    poll(3);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x02, 0x69)), 1);   // slot 6

    // A key held through expiry and a new raw session: its release has no DOWN
    // in the new session, so no UP goes out either.
    reset();
    PAD_setRaw(500);
    phys.tm = 0x4D;
    poll(3);
    tickMs(600);
    PAD_setRaw(500);
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(countRaw(7, RAW_EVT_DOWN), 1);
    CHECK_EQ(countRaw(7, RAW_EVT_UP), 0);
    reset();
    PAD_setRaw(500);
    phys.gpioKey = 1;
    poll(3);
    PAD_setRaw(0);
    PAD_setRaw(500);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(countRaw(0, RAW_EVT_UP), 0);

    // Raw mode switched off explicitly behaves the same.
    reset();
    PAD_setRaw(500);
    phys.gpioKey = 1;
    poll(3);
    PAD_setRaw(0);
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(countKind(OUT_RAW), 1);
    CHECK_EQ(countKind(OUT_RELEASE), 0);

    // Sequence numbers wrap without skipping.
    reset();
    PAD_setRaw(10000);
    for (i = 0; i < 300; ++i) {
        turn(1, i & 1);
        poll(1);
    }
    CHECK_EQ(countKind(OUT_RAW), 300);
    for (i = 1; i < 300 && i < OUT_MAX; ++i) {
        CHECK_EQ((uint8_t)(outs[i - 1].seq + 1), outs[i].seq);
    }

    // Raw events carry the active layer.
    reset();
    PAD_setLayer(1);
    PAD_setRaw(1000);
    turn(0, 1);
    poll(1);
    CHECK_EQ(outs[0].layer, 1);

    // Entering raw mode drops detents still waiting to be typed and releases
    // the one being typed.
    reset();
    spin(0, 1, 6, 2, 2, 0);
    poll(1);
    PAD_setRaw(1000);
    settle(100);
    CHECK_EQ(countKind(OUT_PRESS), countKind(OUT_RELEASE));
    CHECK(countKind(OUT_PRESS) < 6);
    CHECK_EQ(PAD_queued(), 0);
    CHECK_EQ(Pad.tapSlot, SLOT_NONE);
}

// ---------------------------------------------------------------------------------
// Soak: random physical input with bounce, checked against what was performed.
// ---------------------------------------------------------------------------------

// Settle at a state, chattering between the previous state and it first.
static void moveEnc(int knob, uint8_t from, uint8_t to)
{
    int bounces = (int)(rnd() % 4), i;
    for (i = 0; i < bounces; ++i) {
        setEnc(knob, to);
        quarter();
        setEnc(knob, from);
        quarter();
    }
    setEnc(knob, to);
    quarters_(2 + (int)(rnd() % 12));
}

static void setTmBouncy(uint8_t code, uint8_t prev)
{
    int bounces = (int)(rnd() % 3), i;
    for (i = 0; i < bounces; ++i) {
        phys.tm = code;
        poll(1);
        phys.tm = prev;
        poll(1);
    }
    phys.tm = code;
    poll(15 + (int)(rnd() % 26));  // a human press or release: 15-40 ms
}

static void tally(int raw, int *gotDown, int *gotUp, int *gotTap)
{
    int j;
    for (j = 0; j < outCount && j < OUT_MAX; ++j) {
        const out_t *o = &outs[j];
        if (o->kind == OUT_RAW) {
            CHECK(raw);
            if (o->event == RAW_EVT_DOWN) ++gotDown[o->slot];
            if (o->event == RAW_EVT_UP) ++gotUp[o->slot];
            if (o->event == RAW_EVT_TAP) gotTap[o->slot] += o->count;
        } else {
            int slot;
            CHECK(!raw);
            for (slot = 0; slot < SLOT_COUNT; ++slot) {
                if (keymap[0][slot] == o->packed) {
                    break;
                }
            }
            CHECK(slot < SLOT_COUNT);
            if (slot < SLOT_COUNT) {
                const int isTurn = slot >= 15 && (slot - 15) % 3 != KNOB_PRESS;
                if (o->kind == OUT_PRESS) {
                    if (isTurn) ++gotTap[slot];
                    else ++gotDown[slot];
                } else if (!isTurn) {
                    ++gotUp[slot];
                }
            }
        }
    }
    CHECK(outCount <= OUT_MAX);
    outCount = 0;
}

// The daemon's pattern for 10 s of simulated time: heartbeats every 500 ms
// (timeout 1500), the 8-bit millisecond clock wrapping ~39 times, USB-busy
// passes that make PAD_tick see 2..n ms at once. Raw mode must never drop
// (2.0.1 dropped it at every wrap), and every DOWN gets its UP.
static long beatQ = 0;
static void beatIfDue(void)
{
    if (quarters - beatQ >= 4 * 500) {
        CHECK(PAD_setRaw(1500));
        beatQ = quarters;
    }
}

static void runFor(int ms)
{
    while (ms-- > 0) {
        poll(1);
        beatIfDue();
    }
}

static void holdTm(uint8_t code, int ms)
{
    phys.tm = code;
    runFor(ms);
    phys.tm = TM_IDLE;
    runFor(5);
}

static void test_raw_long_run(void)
{
    int i, k, slot;
    int taps[KNOB_COUNT * 2] = {0};
    uint8_t seq;

    reset();
    reportMs = 1;                               // each raw report keeps the loop busy 1 ms
    CHECK(PAD_setRaw(1500));
    beatQ = quarters;
    watchRaw = 1;
    rawOffPasses = 0;
    const long start = quarters;

    // Holds from 0.1 s to 5 s, key 1 (GPIO) and matrix keys.
    phys.gpioKey = 1; runFor(100); phys.gpioKey = 0; runFor(10);
    phys.gpioKey = 1; runFor(1000); phys.gpioKey = 0; runFor(10);
    holdTm(0x44, 100);                          // key2 (slot 1)
    holdTm(0x4C, 2500);                         // key3 (slot 2)
    phys.gpioKey = 1; runFor(5000); phys.gpioKey = 0; runFor(10);
    // Key 1 and a matrix key at once (the TM1650 itself reports one key at a time).
    phys.gpioKey = 1; runFor(50);
    phys.tm = 0x45; runFor(300);                // key7 (slot 6) while key 1 is down
    phys.tm = TM_IDLE; runFor(50);
    phys.gpioKey = 0; runFor(10);
    // Fast spins both ways on every knob, with heartbeats in between.
    for (k = 0; k < KNOB_COUNT; ++k) {
        spin(k, 1, 20, 3, 4, 1);                // 20 detents in ~70 ms
        beatIfDue();
        spin(k, 0, 20, 15, 15, 0);              // 20 detents in 300 ms
        beatIfDue();
        runFor(20);
    }
    runFor(10000 - (int)((quarters - start) / 4) > 0 ? 10000 - (int)((quarters - start) / 4) : 0);
    watchRaw = 0;

    CHECK((quarters - start) / 4 >= 10000);     // >= 39 wraps of the 8-bit clock
    CHECK_EQ(rawOffPasses, 0);
    CHECK(PAD_rawActive(&Pad.raw));
    CHECK_EQ(Pad.stats.rawEntries, 1);
    CHECK_EQ(Pad.stats.rawExpiries, 0);
    CHECK_EQ(Pad.rawEpoch, 1);
    CHECK_EQ(countKind(OUT_PRESS), 0);          // nothing went through the keymap
    // Exact DOWN/UP pairs.
    CHECK_EQ(countRaw(0, RAW_EVT_DOWN), 4);
    CHECK_EQ(countRaw(0, RAW_EVT_UP), 4);
    CHECK_EQ(countRaw(1, RAW_EVT_DOWN), 1);
    CHECK_EQ(countRaw(1, RAW_EVT_UP), 1);
    CHECK_EQ(countRaw(2, RAW_EVT_DOWN), 1);
    CHECK_EQ(countRaw(2, RAW_EVT_UP), 1);
    CHECK_EQ(countRaw(6, RAW_EVT_DOWN), 1);
    CHECK_EQ(countRaw(6, RAW_EVT_UP), 1);
    CHECK_EQ(Pad.rawHeld[0] | Pad.rawHeld[1] | Pad.rawHeld[2], 0);
    // Exact detents, and the running totals the host reconciles against.
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        if (outs[i].kind == OUT_RAW && outs[i].event == RAW_EVT_TAP) {
            slot = outs[i].slot - 15;
            taps[(slot / 3) * 2 + (slot % 3 == KNOB_CW ? 0 : 1)] += outs[i].count;
        }
    }
    for (k = 0; k < KNOB_COUNT; ++k) {
        CHECK_EQ(taps[2 * k], 20);
        CHECK_EQ(taps[2 * k + 1], 20);
        CHECK_EQ(Pad.knobRaw[2 * k], 20);
        CHECK_EQ(Pad.knobRaw[2 * k + 1], 20);
    }
    // Sequence numbers without a gap.
    seq = 0;
    for (i = 0; i < outCount && i < OUT_MAX; ++i) {
        if (outs[i].kind == OUT_RAW) {
            CHECK_EQ(outs[i].seq, (uint8_t)(seq + 1));
            seq = outs[i].seq;
        }
    }
    CHECK_EQ(Pad.rawSeq, seq);

    // Heartbeats stop: raw mode ends once, 1500..1502 ms after the last one,
    // even with the clock wrapping meanwhile.
    reportMs = 0;
    {
        const long last = beatQ;
        long endedAt = -1;
        for (i = 0; i < 3000 && endedAt < 0; ++i) {
            poll(1);
            if (!PAD_rawActive(&Pad.raw)) {
                endedAt = (quarters - last) / 4;
            }
        }
        CHECK(endedAt >= 1500 && endedAt <= 1502);
    }
    CHECK_EQ(Pad.stats.rawExpiries, 1);
    CHECK_EQ(Pad.rawEpoch, 1);                  // the epoch moves on the next start
    CHECK(PAD_setRaw(1500));
    CHECK_EQ(Pad.rawEpoch, 2);
    CHECK_EQ(Pad.stats.rawEntries, 2);
    CHECK(PAD_setRaw(0));
    CHECK_EQ(Pad.stats.rawStops, 1);

    // What 2.0.1 did: SDCC passed 0xFF00 + the 8-bit difference at every wrap
    // of the clock, and any such value ends raw mode at once.
    {
        rawmode_t r;
        CHECK(PAD_rawRequest(&r, 1500));
        CHECK_EQ(PAD_rawTick(&r, 0xFF03), 1);
        CHECK(!PAD_rawActive(&r));
    }
}

// Raw-session snapshot fields the host compares its events with.
static void test_raw_snapshot_state(void)
{
    reset();
    CHECK_EQ(Pad.rawEpoch, 0);
    CHECK(PAD_setRaw(1000));
    phys.gpioKey = 1;
    poll(3);
    phys.tm = 0x44;                             // key2 (slot 1)
    poll(3);
    CHECK_EQ(Pad.rawHeld[0], 0x03);             // slots 0 and 1 reported down
    phys.tm = TM_IDLE;
    poll(3);
    CHECK_EQ(Pad.rawHeld[0], 0x01);
    turn(1, 1);
    turn(1, 1);
    turn(2, 0);
    poll(2);
    CHECK_EQ(Pad.knobRaw[2], 2);                // knob2 cw
    CHECK_EQ(Pad.knobRaw[5], 1);                // knob3 ccw
    CHECK_EQ(Pad.rawSeq, (uint8_t)countKind(OUT_RAW));
    // Detents in keymap mode are not raw detents.
    CHECK(PAD_setRaw(0));
    CHECK_EQ(Pad.rawHeld[0], 0);
    turn(1, 1);
    settle(100);
    CHECK_EQ(Pad.knobRaw[2], 2);
}

static void test_soak(void)
{
    static const uint8_t tmKeys[14] = {0x44, 0x4C, 0x54, 0x5C, 0x64, 0x45, 0x4D,
                                       0x55, 0x5D, 0x65, 0x46, 0x4E, 0x56, 0x5E};
    static const uint8_t knobCodes[3] = {0x67, 0x5F, 0x57};
    int expectDown[SLOT_COUNT], expectTap[SLOT_COUNT];
    int gotDown[SLOT_COUNT], gotUp[SLOT_COUNT], gotTap[SLOT_COUNT];
    int round, i, rawRounds = 0;

    for (round = 0; round < 4; ++round) {
        const int raw = round & 1;
        reset();
        reportMs = round >= 2 ? 1 : 0;
        memset(gotDown, 0, sizeof gotDown);
        memset(gotUp, 0, sizeof gotUp);
        memset(gotTap, 0, sizeof gotTap);
        memset(expectDown, 0, sizeof expectDown);
        memset(expectTap, 0, sizeof expectTap);
        if (raw) {
            PAD_setRaw(10000);
            ++rawRounds;
        }
        for (i = 0; i < 8000; ++i) {
            const uint32_t what = rnd() % 11;
            if (raw) {
                PAD_setRaw(10000);              // heartbeat every operation
            }
            if (what < 3) {
                const int k = (int)(rnd() % 14);
                setTmBouncy(tmKeys[k], TM_IDLE);
                ++expectDown[k + 1];
                setTmBouncy((uint8_t)(tmKeys[k] & ~0x40), tmKeys[k]);
                setTmBouncy(TM_IDLE, (uint8_t)(tmKeys[k] & ~0x40));
            } else if (what == 3) {
                phys.gpioKey = 1;
                poll(15 + (int)(rnd() % 26));
                ++expectDown[0];
                phys.gpioKey = 0;
                poll(15 + (int)(rnd() % 26));
            } else if (what < 6) {
                // Knob press: B line drops first, the TM1650 reports later.
                const int k = (int)(rnd() % 3);
                moveEnc(k, 3, 2);
                setTmBouncy(knobCodes[k], TM_IDLE);
                ++expectDown[SLOT_KNOB(k, KNOB_PRESS)];
                if (rnd() & 1) {
                    moveEnc(k, 2, 3);
                    setTmBouncy(TM_IDLE, knobCodes[k]);
                } else {
                    setTmBouncy(TM_IDLE, knobCodes[k]);
                    moveEnc(k, 2, 3);
                }
            } else if (what < 10) {
                const int k = (int)(rnd() % 3);
                const int cw = (int)(rnd() & 1);
                const uint8_t *path = cw ? CW_PATH : CCW_PATH;
                uint8_t from = 3;
                int s;
                for (s = 0; s < 4; ++s) {
                    moveEnc(k, from, path[s]);
                    from = path[s];
                }
                ++expectTap[SLOT_KNOB(k, cw ? KNOB_CW : KNOB_CCW)];
            } else {
                // A fast flick: several detents at 0.5-1.5 ms per state.
                const int k = (int)(rnd() % 3);
                const int cw = (int)(rnd() & 1);
                const int n = 3 + (int)(rnd() % 12);
                spin(k, cw, n, 2, 6, (int)(rnd() % 2));
                expectTap[SLOT_KNOB(k, cw ? KNOB_CW : KNOB_CCW)] += n;
            }
            if (i % 50 == 49) {
                CHECK(settle(5000) >= 0);
                tally(raw, gotDown, gotUp, gotTap);
            }
        }
        CHECK(settle(5000) >= 0);
        tally(raw, gotDown, gotUp, gotTap);
        for (i = 0; i < SLOT_COUNT; ++i) {
            CHECK_EQ(gotDown[i], expectDown[i]);
            CHECK_EQ(gotUp[i], expectDown[i]);
            CHECK_EQ(gotTap[i], expectTap[i]);
        }
        for (i = 0; i < SLOT_COUNT; ++i) {
            CHECK_EQ(Pad.held[i], 0);
        }
        CHECK_EQ(Pad.rawHeld[0] | Pad.rawHeld[1] | Pad.rawHeld[2], 0);
        {
            padstats_t st;
            PAD_getStats(&st, 0);
            CHECK_EQ(st.overruns + st.queueDrops, 0);
        }
    }
    CHECK_EQ(rawRounds, 2);
}

int main(void)
{
    test_keycode_map();
    test_codec();
    test_crc();
    test_defaults();
    test_encoder();
    test_debounce();
    test_keys();
    test_knobs();
    test_fast_spin();
    test_layers();
    test_raw_mode();
    test_raw_long_run();
    test_raw_snapshot_state();
    test_soak();
    printf("tst_fw_logic: %d checks, %d failures\n", checks, failures);
    return failures ? 1 : 0;
}
