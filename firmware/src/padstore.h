// ===================================================================================
// Keymap storage and host protocol, hardware-free (control-surface fork)
// ===================================================================================
//
// Based on CH552-OpenMacroPad's padcfg (CC BY-SA 3.0). Changes: 24 slots, two
// layers stored as compact 16-bit actions, a raw event mode for the
// control-surface daemon, and layer selection from the host.
//
// The keymap lives in data flash (128 bytes, CRC-checked) and is edited over a
// vendor HID collection: a binding change is one small data-flash write.
//
// Only EEPROM_read/EEPROM_write (eeprom.h) touch hardware, so the host tests
// run this file against a simulated data flash (tests/tst_fw_store.c).
#pragma once
#include <stdint.h>
#include "padlogic.h"

#define FW_VERSION_MAJOR 2
#define FW_VERSION_MINOR 0
#define FW_VERSION_PATCH 0

// Host protocol, report ID 3 (15 bytes each way after the ID):
//   request  [3][cmd][a][b][c][d][e][f] ...
//   reply    [3][cmd echo][...][status @7][...]
#define CFG_REPORT_ID   3
#define RAW_REPORT_ID   5               // raw events: [5][seq][slot][event][layer]
#define CFG_MAGIC0      'C'
#define CFG_MAGIC1      'S'
#define CFG_VERSION     3               // storage format

#define CMD_GET_INFO    0x01            // -> magic, format, slots, eeprom, fw version, layers, active, raw, start layer
#define CMD_GET_ACTION  0x02            // [slot][-][-][-][-][layer] -> type, mod, code lo/hi, status, layer
#define CMD_SET_ACTION  0x03            // [slot][type][mod][code lo][code hi][layer]
#define CMD_RESET       0x04            // defaults for both layers, start layer 0
#define CMD_BOOTLOADER  0x05            // [ 'B' ][ 'L' ] -> reply, then jump to the ROM bootloader
#define CMD_DUMP        0x06            // [offset] -> 12 raw data-flash bytes @3, status @15
#define CMD_CORRUPT     0x07            // damage the stored CRC (recovery test)
#define CMD_RAW_MODE    0x08            // [timeout lo][timeout hi] ms, 0 = off; repeat as heartbeat
#define CMD_SET_LAYER   0x09            // [layer][persist as start layer: 0/1]

#define ST_OK           1
#define ST_BAD_INDEX    2
#define ST_BAD_ACTION   3
#define ST_WRITE_FAIL   4
#define ST_BAD_ARG      5
#define ST_UNKNOWN      6               // command not recognised

// Data flash layout: header, then layer 0 and layer 1, two bytes (lo, hi) per
// slot: 6 + 2 * 24 * 2 = 102 of 128 bytes.
#define HDR_MAGIC0      0
#define HDR_MAGIC1      1
#define HDR_VERSION     2
#define HDR_COUNT       3
#define HDR_CRC         4               // CRC-8 over the action bytes
#define HDR_START_LAYER 5
#define ACTION_BASE     6
#define ACTION_BYTES    (LAYER_COUNT * SLOT_COUNT * 2)

// Load from data flash, or apply and store defaults. Returns the start layer.
uint8_t PADSTORE_init(void);
// Handle one request (report ID first, 16 bytes) and fill a 16-byte reply.
// Returns 1 when the reply has been granted a bootloader jump: send it, then jump.
uint8_t PADSTORE_handle(const uint8_t *req, uint8_t *reply);
