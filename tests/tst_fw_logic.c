// SPDX-License-Identifier: CC-BY-SA-3.0
// Same licence as the firmware it tests (firmware/LICENSE.upstream).
// Host tests for the pad firmware's input and keymap logic (firmware/src/padlogic.c).
//
// The firmware links the same file; here the platform hooks record what the pad
// would have sent, so decoding, dispatch, raw mode and layers can be checked
// without hardware. Includes a seeded soak run of random physical input.

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

enum { OUT_PRESS = 1, OUT_RELEASE, OUT_WAIT, OUT_RAW };

typedef struct {
    int kind;
    uint16_t packed;
    uint8_t seq, slot, event, layer;
} out_t;

#define OUT_MAX 4096
static out_t outs[OUT_MAX];
static int outCount = 0;
static uint16_t keymap[LAYER_COUNT][SLOT_COUNT];

uint16_t PAD_keymap(uint8_t layer, uint8_t slot)
{
    return keymap[layer][slot];
}

static void record(out_t o)
{
    if (outCount < OUT_MAX) {
        outs[outCount] = o;
    }
    ++outCount;
}

void PAD_hwPress(uint16_t packed)
{
    out_t o = {OUT_PRESS, packed, 0, 0, 0, 0};
    record(o);
}

void PAD_hwRelease(uint16_t packed)
{
    out_t o = {OUT_RELEASE, packed, 0, 0, 0, 0};
    record(o);
}

void PAD_hwWait(uint8_t ms)
{
    out_t o = {OUT_WAIT, ms, 0, 0, 0, 0};
    record(o);
}

void PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer)
{
    out_t o = {OUT_RAW, 0, seq, slot, event, layer};
    record(o);
}

// ---------------------------------------------------------------------------------
// Physical pad model
// ---------------------------------------------------------------------------------

#define TM_IDLE 0x2E

typedef struct {
    uint8_t gpioKey;                    // 1 = key 1 down
    uint8_t tm;                         // TM1650 register
    uint8_t a[KNOB_COUNT], b[KNOB_COUNT];
} phys_t;

static phys_t phys;

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

static void poll(int times)
{
    while (times-- > 0) {
        PAD_poll(phys.gpioKey, phys.tm, phys.a, phys.b);
    }
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
    PAD_init(0, phys.gpioKey, phys.tm, phys.a, phys.b);
    outCount = 0;
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

static void turn(int knob, int cw, int samplesPerState)
{
    int i;
    for (i = 0; i < 4; ++i) {
        setEnc(knob, cw ? CW_PATH[i] : CCW_PATH[i]);
        poll(samplesPerState);
    }
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

static int8_t encRun(encstate_t *e, const uint8_t *states, int n, uint8_t held, int *steps)
{
    int i;
    int8_t last = 0;
    for (i = 0; i < n; ++i) {
        int8_t s = PAD_encoderUpdate(e, (states[i] >> 1) & 1, states[i] & 1, held);
        if (s) {
            *steps += s;
            last = s;
        }
    }
    return last;
}

static void test_encoder(void)
{
    encstate_t e;
    int steps;

    // One clean detent each way, reported on reaching rest.
    {
        static const uint8_t cw[] = {1, 0, 2, 3};
        static const uint8_t ccw[] = {2, 0, 1, 3};
        PAD_encoderInit(&e, 1, 1);
        CHECK_EQ(PAD_encoderUpdate(&e, 0, 1, 0), 0);
        CHECK_EQ(PAD_encoderUpdate(&e, 0, 0, 0), 0);
        CHECK_EQ(PAD_encoderUpdate(&e, 1, 0, 0), 0);
        CHECK_EQ(PAD_encoderUpdate(&e, 1, 1, 0), 1);
        steps = 0;
        encRun(&e, ccw, 4, 0, &steps);
        CHECK_EQ(steps, -1);
        steps = 0;
        encRun(&e, cw, 4, 0, &steps);
        encRun(&e, cw, 4, 0, &steps);
        CHECK_EQ(steps, 2);
    }
    // Repeated samples of the same state change nothing.
    PAD_encoderInit(&e, 1, 1);
    CHECK_EQ(PAD_encoderUpdate(&e, 1, 1, 0), 0);
    CHECK_EQ(e.count, 0);

    // Knob press: B low and back, alone and with contact bounce.
    {
        static const uint8_t press[] = {2, 3};
        static const uint8_t bouncy[] = {2, 3, 2, 3, 2, 2, 2, 3, 2, 3};
        static const uint8_t pressA[] = {2, 0, 2, 0, 2, 3};      // A chatters too
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, press, 2, 0, &steps);
        encRun(&e, bouncy, 10, 0, &steps);
        encRun(&e, pressA, 6, 0, &steps);
        CHECK_EQ(steps, 0);
    }
    // Rotating while the switch is held: nothing, and nothing left over after.
    {
        static const uint8_t heldTurn[] = {2, 0, 2, 0, 1, 0, 2};
        static const uint8_t back[] = {3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, heldTurn, 7, 1, &steps);
        encRun(&e, back, 1, 1, &steps);
        CHECK_EQ(steps, 0);
        CHECK_EQ(e.count, 0);
    }
    // Press, part of a turn while held, then the line returns with the switch
    // still reported: the half-turn must not complete into a step.
    {
        static const uint8_t press[] = {2};
        static const uint8_t heldHalf[] = {0, 1, 3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, press, 1, 0, &steps);
        encRun(&e, heldHalf, 3, 1, &steps);
        CHECK_EQ(steps, 0);
    }
    // Press seen on B before the TM1650 reports it, released after.
    {
        static const uint8_t down[] = {2};
        static const uint8_t up[] = {3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, down, 1, 0, &steps);
        encRun(&e, up, 1, 1, &steps);
        CHECK_EQ(steps, 0);
        // ... or the switch report ends first.
        encRun(&e, down, 1, 1, &steps);
        encRun(&e, up, 1, 0, &steps);
        CHECK_EQ(steps, 0);
    }
    // Bounce at the leading edge still gives exactly one step.
    {
        static const uint8_t edge[] = {1, 3, 1, 3, 1, 0, 1, 0, 2, 0, 2, 3, 2, 3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, edge, 14, 0, &steps);
        CHECK_EQ(steps, 1);
    }
    // One skipped intermediate state (fast spin) still counts, in the right direction.
    {
        static const uint8_t skipMid[] = {1, 2, 3};             // 0 missed
        static const uint8_t skipFirst[] = {0, 2, 3};           // 1 missed
        static const uint8_t skipLastCcw[] = {2, 0, 3};         // 1 missed
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, skipMid, 3, 0, &steps);
        CHECK_EQ(steps, 1);
        steps = 0;
        encRun(&e, skipFirst, 3, 0, &steps);
        CHECK_EQ(steps, 1);
        steps = 0;
        encRun(&e, skipLastCcw, 3, 0, &steps);
        CHECK_EQ(steps, -1);
    }
    // Two states missed carries no direction: dropped, never reversed.
    {
        static const uint8_t jump[] = {0, 3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, jump, 2, 0, &steps);
        CHECK_EQ(steps, 0);
    }
    // Starting a detent and going back is not a step.
    {
        static const uint8_t back[] = {1, 0, 1, 3, 2, 0, 2, 3};
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        encRun(&e, back, 8, 0, &steps);
        CHECK_EQ(steps, 0);
    }
    // Long chatter stays bounded and still resolves.
    {
        int i;
        PAD_encoderInit(&e, 1, 1);
        steps = 0;
        for (i = 0; i < 1000; ++i) {
            static const uint8_t cycle[] = {1, 0, 2, 3};
            int8_t s = PAD_encoderUpdate(&e, (cycle[i % 4] >> 1) & 1, cycle[i % 4] & 1, 0);
            steps += s;
            CHECK(e.count <= 8 && e.count >= -8);
        }
        CHECK_EQ(steps, 250);
    }
    // Starting away from rest (power-on mid-detent) does not invent a step.
    {
        static const uint8_t settle[] = {3};
        PAD_encoderInit(&e, 0, 0);
        steps = 0;
        encRun(&e, settle, 1, 0, &steps);
        CHECK_EQ(steps, 0);
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
    PAD_init(0, phys.gpioKey, phys.tm, phys.a, phys.b);
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
            poll(1);
        }
        phys.b[1] = 0;
        phys.tm = 0x5F;
        poll(4);
        phys.b[1] = 1;
        poll(1);
        phys.b[1] = 0;
        poll(1);
        phys.b[1] = 1;
        poll(2);
        phys.tm = TM_IDLE;
        poll(4);
    }
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6A)), 1);   // slot 19 = alt+F15
    CHECK_EQ(countKind(OUT_RELEASE), 1);

    // Turns are taps: press, gap, release, on the right slot.
    reset();
    turn(0, 1, 2);
    CHECK_EQ(outCount, 3);
    CHECK_EQ(outs[0].kind, OUT_PRESS);
    CHECK_EQ(outs[0].packed, key(0x01, 0x6E));  // slot 17 = top cw = ctrl+F19
    CHECK_EQ(outs[1].kind, OUT_WAIT);
    CHECK_EQ(outs[1].packed, PAD_TAP_GAP_MS);
    CHECK_EQ(outs[2].kind, OUT_RELEASE);
    reset();
    turn(1, 0, 2);
    turn(2, 1, 2);
    turn(2, 0, 2);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x69)), 1);   // slot 18 = middle ccw
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6E)), 1);   // slot 23 = bottom cw
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6C)), 1);   // slot 21 = bottom ccw

    // Fast spin: one sample per state, 40 detents in a row.
    reset();
    {
        int i;
        for (i = 0; i < 40; ++i) {
            turn(1, 1, 1);
        }
    }
    CHECK_EQ(countPacked(OUT_PRESS, key(0x04, 0x6B)), 40);  // slot 20 = middle cw
    CHECK_EQ(countKind(OUT_PRESS), 40);

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
    CHECK_EQ(countKind(OUT_PRESS), 1);
    CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6D)), 1);

    // Mouse wheel detents are one-shot, buttons are taps.
    reset();
    PAD_setLayer(1);
    turn(1, 1, 2);
    CHECK_EQ(outCount, 1);
    CHECK_EQ(outs[0].kind, OUT_PRESS);
    {
        uint8_t type, mod;
        uint16_t code;
        PAD_decode(outs[0].packed, &type, &mod, &code);
        CHECK_EQ(type, ACT_MOUSE);
        CHECK_EQ(code, (MS_WHEEL << 8) | 0x01);
    }
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
    turn(0, 1, 2);                              // layer 1 top cw = volume up
    phys.gpioKey = 0;
    poll(3);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 0);
    turn(0, 1, 2);
    {
        uint16_t volUp = 0;
        PAD_encode(ACT_CON, 0, 0xE9, &volUp);
        CHECK_EQ(countPacked(OUT_PRESS, volUp), 1);
        CHECK_EQ(countPacked(OUT_PRESS, key(0x01, 0x6E)), 1);
    }

    // A momentary layer bound to a detent does nothing (press and release at once).
    reset();
    keymap[0][SLOT_KNOB(2, KNOB_CW)] = momentary;
    turn(2, 1, 2);
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
    turn(0, 0, 2);
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
        PAD_tick(900);
        CHECK(PAD_rawActive(&Pad.raw));
        CHECK(PAD_setRaw(1000));
    }
    PAD_tick(999);
    CHECK(PAD_rawActive(&Pad.raw));
    PAD_tick(1);
    CHECK(!PAD_rawActive(&Pad.raw));
    PAD_tick(5000);
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
    PAD_tick(600);
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
    PAD_tick(600);
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
    PAD_setRaw(1000);
    for (i = 0; i < 300; ++i) {
        turn(1, i & 1, 1);
    }
    CHECK_EQ(countKind(OUT_RAW), 300);
    for (i = 1; i < 300 && i < OUT_MAX; ++i) {
        CHECK_EQ((uint8_t)(outs[i - 1].seq + 1), outs[i].seq);
    }

    // Raw events carry the active layer.
    reset();
    PAD_setLayer(1);
    PAD_setRaw(1000);
    turn(0, 1, 1);
    CHECK_EQ(outs[0].layer, 1);
}

// ---------------------------------------------------------------------------------
// Soak: random physical input with bounce, checked against what was performed.
// ---------------------------------------------------------------------------------

static uint32_t rng = 0x12345678u;
static uint32_t rnd(void)
{
    rng ^= rng << 13;
    rng ^= rng >> 17;
    rng ^= rng << 5;
    return rng;
}

// Settle at a state, chattering between the previous state and it first.
static void moveEnc(int knob, uint8_t from, uint8_t to)
{
    int bounces = (int)(rnd() % 4), i;
    for (i = 0; i < bounces; ++i) {
        setEnc(knob, to);
        poll(1);
        setEnc(knob, from);
        poll(1);
    }
    setEnc(knob, to);
    poll(1 + (int)(rnd() % 3));
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
    poll(4 + (int)(rnd() % 4));
}

static void test_soak(void)
{
    static const uint8_t tmKeys[14] = {0x44, 0x4C, 0x54, 0x5C, 0x64, 0x45, 0x4D,
                                       0x55, 0x5D, 0x65, 0x46, 0x4E, 0x56, 0x5E};
    static const uint8_t knobCodes[3] = {0x67, 0x5F, 0x57};
    int expectDown[SLOT_COUNT] = {0}, expectTap[SLOT_COUNT] = {0};
    int gotDown[SLOT_COUNT] = {0}, gotUp[SLOT_COUNT] = {0}, gotTap[SLOT_COUNT] = {0};
    int round, i, rawRounds = 0;

    for (round = 0; round < 4; ++round) {
        const int raw = round & 1;
        reset();
        memset(gotDown, 0, sizeof gotDown);
        memset(gotUp, 0, sizeof gotUp);
        memset(gotTap, 0, sizeof gotTap);
        memset(expectDown, 0, sizeof expectDown);
        memset(expectTap, 0, sizeof expectTap);
        if (raw) {
            PAD_setRaw(10000);
            ++rawRounds;
        }
        for (i = 0; i < 20000; ++i) {
            const uint32_t what = rnd() % 10;
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
                poll(3 + (int)(rnd() % 5));
                ++expectDown[0];
                phys.gpioKey = 0;
                poll(3 + (int)(rnd() % 5));
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
            } else {
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
            }
            // Tally and drop the log as we go.
            {
                int j;
                for (j = 0; j < outCount && j < OUT_MAX; ++j) {
                    const out_t *o = &outs[j];
                    if (o->kind == OUT_RAW) {
                        CHECK(raw);
                        if (o->event == RAW_EVT_DOWN) ++gotDown[o->slot];
                        if (o->event == RAW_EVT_UP) ++gotUp[o->slot];
                        if (o->event == RAW_EVT_TAP) ++gotTap[o->slot];
                    } else if (o->kind == OUT_PRESS || o->kind == OUT_RELEASE) {
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
        }
        for (i = 0; i < SLOT_COUNT; ++i) {
            CHECK_EQ(gotDown[i], expectDown[i]);
            CHECK_EQ(gotUp[i], expectDown[i]);
            CHECK_EQ(gotTap[i], expectTap[i]);
        }
        // Nothing left held at the end.
        for (i = 0; i < SLOT_COUNT; ++i) {
            CHECK_EQ(Pad.held[i], 0);
        }
        CHECK_EQ(Pad.rawHeld[0] | Pad.rawHeld[1] | Pad.rawHeld[2], 0);
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
    test_layers();
    test_raw_mode();
    test_soak();
    printf("tst_fw_logic: %d checks, %d failures\n", checks, failures);
    return failures ? 1 : 0;
}
