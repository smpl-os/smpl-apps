// SPDX-License-Identifier: CC-BY-SA-3.0
// Same licence as the firmware it tests (firmware/LICENSE.upstream).
//
// Host tests for the pad firmware's keymap storage and host protocol
// (firmware/src/padstore.c) against a simulated 128-byte data flash, including
// write failures and power cuts at every write.

#include "eeprom.h"
#include "padstore.h"

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
// Simulated data flash and platform
// ---------------------------------------------------------------------------------

static uint8_t flash[EEPROM_SIZE];
static int writes = 0;
static int cutAfter = -1;               // writes allowed before the power goes; -1 = never
static int failWrites = 0;              // writes report failure (and do not land)

uint8_t EEPROM_read(uint8_t addr)
{
    CHECK(addr < EEPROM_SIZE);
    return addr < EEPROM_SIZE ? flash[addr] : 0xFF;
}

uint8_t EEPROM_write(uint8_t addr, uint8_t value)
{
    CHECK(addr < EEPROM_SIZE);
    if (cutAfter >= 0 && writes >= cutAfter) {
        return 1;                       // power is gone: the write never lands
    }
    ++writes;
    if (failWrites) {
        return 0;
    }
    flash[addr] = value;
    return 1;
}

void PAD_hwPress(uint16_t packed) { (void)packed; }
void PAD_hwRelease(uint16_t packed) { (void)packed; }
void PAD_hwWait(uint8_t ms) { (void)ms; }
void PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer)
{
    (void)seq; (void)slot; (void)event; (void)layer;
}

static uint8_t boot(void)
{
    static const uint8_t encA[KNOB_COUNT] = {1, 1, 1};
    static const uint8_t encB[KNOB_COUNT] = {1, 1, 1};
    uint8_t start;
    cutAfter = -1;
    failWrites = 0;
    start = PADSTORE_init();
    PAD_init(start, 0, 0x2E, encA, encB);
    return start;
}

static uint8_t rep[16];

static uint8_t cmd(uint8_t c, uint8_t a, uint8_t b, uint8_t d, uint8_t e, uint8_t f, uint8_t layer)
{
    uint8_t req[16] = {0};
    req[0] = CFG_REPORT_ID;
    req[1] = c;
    req[2] = a;
    req[3] = b;
    req[4] = d;
    req[5] = e;
    req[6] = f;
    req[7] = layer;
    memset(rep, 0xAA, sizeof rep);
    return PADSTORE_handle(req, rep);
}

static uint8_t setAction(uint8_t layer, uint8_t slot, uint8_t type, uint8_t mod, uint16_t code)
{
    cmd(CMD_SET_ACTION, slot, type, mod, (uint8_t)(code & 0xFF), (uint8_t)(code >> 8), layer);
    return rep[7];
}

static int keymapIsDefault(void)
{
    int l, s;
    for (l = 0; l < LAYER_COUNT; ++l) {
        for (s = 0; s < SLOT_COUNT; ++s) {
            if (PAD_keymap((uint8_t)l, (uint8_t)s) != PAD_defaultAction((uint8_t)l, (uint8_t)s)) {
                return 0;
            }
        }
    }
    return 1;
}

typedef uint16_t map_t[LAYER_COUNT][SLOT_COUNT];

static void snapshot(map_t m)
{
    int l, s;
    for (l = 0; l < LAYER_COUNT; ++l) {
        for (s = 0; s < SLOT_COUNT; ++s) {
            m[l][s] = PAD_keymap((uint8_t)l, (uint8_t)s);
        }
    }
}

static int sameMap(map_t a, map_t b)
{
    return memcmp(a, b, sizeof(map_t)) == 0;
}

static void freshFlash(uint8_t fill)
{
    memset(flash, fill, sizeof flash);
    writes = 0;
}

// ---------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------

static void test_first_boot(void)
{
    uint8_t copy[EEPROM_SIZE];

    // Erased data flash: defaults, written once.
    freshFlash(0xFF);
    CHECK_EQ(boot(), 0);
    CHECK(keymapIsDefault());
    CHECK(writes > ACTION_BYTES);
    CHECK_EQ(flash[HDR_MAGIC0], CFG_MAGIC0);
    CHECK_EQ(flash[HDR_MAGIC1], CFG_MAGIC1);
    CHECK_EQ(flash[HDR_VERSION], CFG_VERSION);
    CHECK_EQ(flash[HDR_COUNT], SLOT_COUNT);
    CHECK_EQ(flash[HDR_START_LAYER], 0);
    CHECK_EQ(flash[HDR_CRC], PAD_crc8(flash + ACTION_BASE, ACTION_BYTES));
    // Little-endian compact actions straight after the header.
    CHECK_EQ(flash[ACTION_BASE] | (flash[ACTION_BASE + 1] << 8), PAD_defaultAction(0, 0));
    CHECK_EQ(flash[ACTION_BASE + 2 * SLOT_COUNT] | (flash[ACTION_BASE + 2 * SLOT_COUNT + 1] << 8),
             PAD_defaultAction(1, 0));

    // A normal boot reads only: no data-flash wear per power-up.
    memcpy(copy, flash, sizeof copy);
    writes = 0;
    CHECK_EQ(boot(), 0);
    CHECK_EQ(writes, 0);
    CHECK(memcmp(copy, flash, sizeof copy) == 0);
    CHECK(keymapIsDefault());

    // Whatever the stock firmware or EpicLPer's v2 layout left: defaults.
    {
        int i, trial;
        uint32_t x = 0x9E3779B9u;
        for (trial = 0; trial < 200; ++trial) {
            for (i = 0; i < EEPROM_SIZE; ++i) {
                x = x * 1664525u + 1013904223u;
                flash[i] = (uint8_t)(x >> 24);
            }
            if (trial == 0) {
                flash[0] = 'O';                 // v2 magic
                flash[1] = 'M';
                flash[2] = 2;
            }
            boot();
            CHECK(keymapIsDefault());
            CHECK_EQ(Pad.layers.base, 0);
        }
    }
}

static void test_get_info(void)
{
    freshFlash(0xFF);
    boot();
    CHECK_EQ(cmd(CMD_GET_INFO, 0, 0, 0, 0, 0, 0), 0);
    CHECK_EQ(rep[0], CFG_REPORT_ID);
    CHECK_EQ(rep[1], CMD_GET_INFO);
    CHECK_EQ(rep[2], 'C');
    CHECK_EQ(rep[3], 'S');
    CHECK_EQ(rep[4], 3);
    CHECK_EQ(rep[5], 24);
    CHECK_EQ(rep[6], 128);
    CHECK_EQ(rep[7], ST_OK);
    CHECK_EQ(rep[8], FW_VERSION_MAJOR);
    CHECK_EQ(rep[9], FW_VERSION_MINOR);
    CHECK_EQ(rep[10], FW_VERSION_PATCH);
    CHECK_EQ(rep[11], 2);
    CHECK_EQ(rep[12], 0);
    CHECK_EQ(rep[13], 0);
    CHECK_EQ(rep[14], 0);
    CHECK_EQ(rep[15], 0);
}

static void test_get_set(void)
{
    int s;
    freshFlash(0xFF);
    boot();

    // Every default reads back in host form.
    for (s = 0; s < SLOT_COUNT; ++s) {
        cmd(CMD_GET_ACTION, (uint8_t)s, 0, 0, 0, 0, 0);
        CHECK_EQ(rep[7], ST_OK);
        CHECK_EQ(rep[2], s);
        CHECK_EQ(rep[3], ACT_KEY);
        CHECK_EQ(rep[5], 0x69 + s % 6);
        CHECK_EQ(rep[6], 0);
        CHECK_EQ(rep[8], 0);
    }
    cmd(CMD_GET_ACTION, 12, 0, 0, 0, 0, 1);
    CHECK_EQ(rep[3], ACT_CON);
    CHECK_EQ(rep[5] | (rep[6] << 8), 0xCD);
    CHECK_EQ(rep[8], 1);
    cmd(CMD_GET_ACTION, 24, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_BAD_INDEX);
    cmd(CMD_GET_ACTION, 0, 0, 0, 0, 0, 2);
    CHECK_EQ(rep[7], ST_BAD_INDEX);

    // A change is two action bytes plus the CRC, and survives a reboot.
    writes = 0;
    CHECK_EQ(setAction(1, 5, ACT_KEY, 0x22, 0x04), ST_BAD_ACTION);   // left + right shift
    CHECK_EQ(writes, 0);
    CHECK_EQ(setAction(1, 5, ACT_KEY, 0x20, 0x04), ST_OK);
    CHECK_EQ(writes, 3);
    CHECK_EQ(setAction(0, 23, ACT_CON, 0, 0x192), ST_OK);
    CHECK_EQ(setAction(1, 0, ACT_LAYER, 0, (LAYER_OP_SET << 8) | 0), ST_OK);
    CHECK_EQ(setAction(1, 1, ACT_NONE, 0, 0), ST_OK);
    boot();
    cmd(CMD_GET_ACTION, 5, 0, 0, 0, 0, 1);
    CHECK_EQ(rep[3], ACT_KEY);
    CHECK_EQ(rep[4], 0x20);
    CHECK_EQ(rep[5], 0x04);
    cmd(CMD_GET_ACTION, 23, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[3], ACT_CON);
    CHECK_EQ(rep[5] | (rep[6] << 8), 0x192);
    cmd(CMD_GET_ACTION, 1, 0, 0, 0, 0, 1);
    CHECK_EQ(rep[3], ACT_NONE);
    CHECK_EQ(PAD_keymap(1, 1), 0);

    // Refusals write nothing and change nothing.
    {
        map_t before, after;
        snapshot(before);
        writes = 0;
        CHECK_EQ(setAction(0, 24, ACT_KEY, 0, 0x04), ST_BAD_INDEX);
        CHECK_EQ(setAction(2, 0, ACT_KEY, 0, 0x04), ST_BAD_INDEX);
        CHECK_EQ(setAction(0, 0, ACT_KEY, 0x11, 0x04), ST_BAD_ACTION);    // mixed hands
        CHECK_EQ(setAction(0, 0, ACT_KEY, 0, 0xE8), ST_BAD_ACTION);
        CHECK_EQ(setAction(0, 0, ACT_CON, 0, 0x400), ST_BAD_ACTION);
        CHECK_EQ(setAction(0, 0, ACT_LAYER, 0, 2), ST_BAD_ACTION);
        CHECK_EQ(setAction(0, 0, 9, 0, 1), ST_BAD_ACTION);
        CHECK_EQ(writes, 0);
        snapshot(after);
        CHECK(sameMap(before, after));
    }

    // A failing write is reported.
    failWrites = 1;
    CHECK_EQ(setAction(0, 0, ACT_KEY, 0, 0x05), ST_WRITE_FAIL);
    failWrites = 0;
}

static void test_reset_layer_raw_boot(void)
{
    freshFlash(0xFF);
    boot();
    setAction(0, 3, ACT_KEY, 0, 0x04);
    CHECK(!keymapIsDefault());
    cmd(CMD_RESET, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_OK);
    CHECK(keymapIsDefault());
    boot();
    CHECK(keymapIsDefault());

    // Layer: volatile unless persisted; bad values refused.
    cmd(CMD_SET_LAYER, 1, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_OK);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
    CHECK_EQ(boot(), 0);
    cmd(CMD_SET_LAYER, 1, 1, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_OK);
    CHECK_EQ(boot(), 1);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
    cmd(CMD_GET_INFO, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[12], 1);
    CHECK_EQ(rep[14], 1);
    cmd(CMD_SET_LAYER, 2, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_BAD_ARG);
    cmd(CMD_SET_LAYER, 0, 2, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_BAD_ARG);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 1);
    cmd(CMD_RESET, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(PAD_activeLayer(&Pad.layers), 0);
    CHECK_EQ(boot(), 0);

    // Raw mode requests.
    cmd(CMD_RAW_MODE, 0xE8, 0x03, 0, 0, 0, 0);          // 1000 ms
    CHECK_EQ(rep[7], ST_OK);
    CHECK_EQ(rep[8], 1);
    cmd(CMD_RAW_MODE, 0x11, 0x27, 0, 0, 0, 0);          // 10001 ms
    CHECK_EQ(rep[7], ST_BAD_ARG);
    CHECK_EQ(rep[8], 1);
    cmd(CMD_GET_INFO, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[13], 1);
    cmd(CMD_RAW_MODE, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[8], 0);
    boot();
    CHECK(!PAD_rawActive(&Pad.raw));                    // never survives a reboot

    // Bootloader only with its guard.
    CHECK_EQ(cmd(CMD_BOOTLOADER, 0, 0, 0, 0, 0, 0), 0);
    CHECK_EQ(rep[7], ST_BAD_ARG);
    CHECK_EQ(cmd(CMD_BOOTLOADER, 'B', 'X', 0, 0, 0, 0), 0);
    CHECK_EQ(cmd(CMD_BOOTLOADER, 'B', 'L', 0, 0, 0, 0), 1);
    CHECK_EQ(rep[7], ST_OK);

    // Unknown commands are echoed and refused.
    CHECK_EQ(cmd(0x42, 0, 0, 0, 0, 0, 0), 0);
    CHECK_EQ(rep[1], 0x42);
    CHECK_EQ(rep[7], ST_UNKNOWN);
}

static void test_dump(void)
{
    int off, i;
    freshFlash(0xFF);
    boot();
    for (off = 0; off < EEPROM_SIZE; off += 12) {
        cmd(CMD_DUMP, (uint8_t)off, 0, 0, 0, 0, 0);
        CHECK_EQ(rep[15], ST_OK);
        CHECK_EQ(rep[2], off);
        for (i = 0; i < 12; ++i) {
            CHECK_EQ(rep[3 + i], off + i < EEPROM_SIZE ? flash[off + i] : 0);
        }
    }
    cmd(CMD_DUMP, 128, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[15], ST_BAD_INDEX);
    cmd(CMD_DUMP, 255, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[15], ST_BAD_INDEX);
}

static void test_corruption(void)
{
    int i;
    freshFlash(0xFF);
    boot();
    setAction(0, 2, ACT_KEY, 0, 0x04);

    cmd(CMD_CORRUPT, 0, 0, 0, 0, 0, 0);
    CHECK_EQ(rep[7], ST_OK);
    boot();
    CHECK(keymapIsDefault());

    // Any single flipped bit in the header or actions falls back to defaults.
    for (i = 0; i < (ACTION_BASE + ACTION_BYTES) * 8; ++i) {
        if (i / 8 == HDR_START_LAYER) {
            continue;                           // checked separately below
        }
        freshFlash(0xFF);
        boot();
        setAction(1, 7, ACT_KEY, 0, 0x04);
        flash[i / 8] ^= (uint8_t)(1 << (i % 8));
        boot();
        CHECK(keymapIsDefault());
    }
    // Start layer out of range: defaults, layer 0.
    freshFlash(0xFF);
    boot();
    flash[HDR_START_LAYER] = 7;
    CHECK_EQ(boot(), 0);

    // A correct CRC over meaningless actions is still refused.
    freshFlash(0xFF);
    boot();
    flash[ACTION_BASE] = 0xFF;
    flash[ACTION_BASE + 1] = 0xFF;
    flash[HDR_CRC] = PAD_crc8(flash + ACTION_BASE, ACTION_BYTES);
    boot();
    CHECK(keymapIsDefault());
}

// Power cut after every possible number of writes: the next boot must find
// either the old keymap or the defaults (or, for a single change, the new one),
// never a mixture.
static void test_power_cuts(void)
{
    map_t custom, changed;
    int n, total;

    // CMD_RESET from a custom keymap.
    freshFlash(0xFF);
    boot();
    setAction(0, 0, ACT_CON, 0, 0xE9);
    setAction(1, 23, ACT_KEY, 0x04, 0x29);
    snapshot(custom);
    {
        uint8_t saved[EEPROM_SIZE];
        memcpy(saved, flash, sizeof saved);
        writes = 0;
        cmd(CMD_RESET, 0, 0, 0, 0, 0, 0);
        total = writes;
        CHECK(total > ACTION_BYTES);
        for (n = 0; n <= total; ++n) {
            map_t got;
            memcpy(flash, saved, sizeof saved);
            boot();
            writes = 0;
            cutAfter = n;
            cmd(CMD_RESET, 0, 0, 0, 0, 0, 0);
            boot();
            snapshot(got);
            CHECK(sameMap(got, custom) || keymapIsDefault());
        }
    }

    // A single SET_ACTION.
    freshFlash(0xFF);
    boot();
    setAction(0, 0, ACT_CON, 0, 0xE9);
    snapshot(custom);
    {
        uint8_t saved[EEPROM_SIZE];
        memcpy(saved, flash, sizeof saved);
        setAction(0, 9, ACT_MOUSE, 0, (MS_WHEEL << 8) | 3);
        snapshot(changed);
        for (n = 0; n <= 3; ++n) {
            map_t got;
            memcpy(flash, saved, sizeof saved);
            boot();
            writes = 0;
            cutAfter = n;
            setAction(0, 9, ACT_MOUSE, 0, (MS_WHEEL << 8) | 3);
            boot();
            snapshot(got);
            CHECK(sameMap(got, custom) || sameMap(got, changed) || keymapIsDefault());
        }
    }

    // First boot interrupted: the next boot still ends with a valid default map.
    for (n = 0; n < ACTION_BASE + ACTION_BYTES + 8; ++n) {
        freshFlash(0xFF);
        cutAfter = n;
        PADSTORE_init();
        boot();
        CHECK(keymapIsDefault());
        CHECK_EQ(flash[HDR_MAGIC0], CFG_MAGIC0);
    }
}

int main(void)
{
    test_first_boot();
    test_get_info();
    test_get_set();
    test_reset_layer_raw_boot();
    test_dump();
    test_corruption();
    test_power_cuts();
    printf("tst_fw_store: %d checks, %d failures\n", checks, failures);
    return failures ? 1 : 0;
}
