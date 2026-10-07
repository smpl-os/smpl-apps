// ===================================================================================
// Pure input and keymap logic for the 15-key, 3-knob pad (no hardware access)
// ===================================================================================
//
// Kept free of SDCC storage classes and registers so the same file builds for
// the CH552 and for the host test suite (tests/tst_fw_logic.c).
//
// Part of the control-surface fork of CH552-OpenMacroPad; firmware licence
// CC BY-SA 3.0 (see firmware/LICENSE.upstream).
#pragma once
#include <stdint.h>

// -----------------------------------------------------------------------------------
// Slots: keys 1-15 are 0-14, then knob n (0 = top) ccw/press/cw = 15 + 3n + 0/1/2.
// Same order as the daemon's keys-then-knobs hardware map.
// -----------------------------------------------------------------------------------
#define SLOT_COUNT      24
#define KEY_COUNT       15
#define KNOB_COUNT      3
#define SLOT_NONE       0xFF
#define SLOT_KNOB(n, r) (15 + 3 * (n) + (r))
#define KNOB_CCW        0
#define KNOB_PRESS      1
#define KNOB_CW         2

// Key 1 (top left) is on GPIO P1.5, the ROM bootloader's download pin; every
// other key and the three knob switches come from the TM1650 key register.
#define SLOT_GPIO_KEY   0

// TM1650 key register (bit 6 = pressed) to slot, or SLOT_NONE.
uint8_t PAD_slotForKeycode(uint8_t keycode);

// -----------------------------------------------------------------------------------
// Actions as the host sees them (protocol v3): type, modifier mask, 16-bit code.
// -----------------------------------------------------------------------------------
#define ACT_NONE        0
#define ACT_KEY         1               // keyboard usage + modifier mask
#define ACT_CON         2               // consumer usage (<= 0x3FF)
#define ACT_MOUSE       3               // code = subtype << 8 | value
#define ACT_LAYER       4               // code = op << 8 | target layer
#define ACT_TYPE_MAX    ACT_LAYER

#define MS_BUTTON       0
#define MS_WHEEL        1
#define MS_PAN          2
#define MS_MOVE_X       3
#define MS_MOVE_Y       4
#define MS_SUB_MAX      MS_MOVE_Y

#define LAYER_OP_TOGGLE    0            // tap: switch between layer 0 and 1
#define LAYER_OP_MOMENTARY 1            // held: target layer while down
#define LAYER_OP_SET       2            // tap: go to target layer
#define LAYER_OP_MAX       LAYER_OP_SET
#define LAYER_COUNT        2

#define CON_CODE_MAX    0x03FF

// Compact storage: 16 bits per action so two full layers fit the 128-byte data
// flash (6-byte header + 2 x 24 x 2 = 102 bytes).
//   bits 15-13  type
//   key:   bits 12-8 = ctrl, shift, alt, gui, right-hand flag; bits 7-0 = usage
//   con:   bits 9-0 = usage, 12-10 zero
//   mouse: bits 10-8 = subtype, bits 7-0 = value, 12-11 zero
//   layer: bits 9-8 = op, bits 7-0 = target layer, 12-10 zero
// A key chord whose modifiers mix left and right hands cannot be stored.

// Validate a host action; 1 if the input path can act on it and it is storable.
uint8_t PAD_validAction(uint8_t type, uint8_t mod, uint16_t code);
// Host form <-> compact form. Encode returns 0 for actions that fail validation.
uint8_t  PAD_encode(uint8_t type, uint8_t mod, uint16_t code, uint16_t *out);
void     PAD_decode(uint16_t packed, uint8_t *type, uint8_t *mod, uint16_t *code);
// Compact value that decodes to a valid action (stored data is checked on boot).
uint8_t  PAD_validPacked(uint16_t packed);
// Factory keymap, compact form (0 = ACT_NONE).
uint16_t PAD_defaultAction(uint8_t layer, uint8_t slot);

// CRC-8 (poly 0x07, init 0xFF) over a byte buffer; matches the stored block check.
uint8_t PAD_crc8(const uint8_t *data, uint8_t len);

// -----------------------------------------------------------------------------------
// Encoders: detented, both lines high at rest, one full quadrature cycle per detent.
// Clockwise is A falling first: (A,B) 11 -> 01 -> 00 -> 10 -> 11.
//
// Sampled from a 4 kHz timer interrupt (PAD_encoderIsr), independent of the main
// loop, so slow USB reports or I2C reads never cost a detent. Each valid
// single-line transition counts +-1 (a transition table, Buxton style); a detent
// is emitted every 4 counts in one direction, and on return to rest a remaining
// |count| >= 2 also counts (one skipped state). Detents accumulate per knob until
// the main loop takes them (PAD_encoderTake), so none are dropped when output is
// slower than the spin.
//
// Knob press: on this board a press pulls one encoder line low and back. That is
// -1 then +1, so it nets to zero; while the knob's switch is reported held the
// count is also frozen at zero (with B held low a turn carries no direction).
// A two-line jump (a missed state) counts as two quarter steps in the current
// direction mid-rotation, and as nothing at rest; either way it is counted as
// an illegal transition (a diagnostic: sampling too slow or a noisy line).
// -----------------------------------------------------------------------------------
#define ENC_ACC_MAX     100             // detents waiting to be taken, per knob

// pins: bit 2k+1 = knob k line A, bit 2k = line B (1 = high).
void    PAD_encoderReset(uint8_t pins);
void    PAD_encoderIsr(uint8_t pins);
// Detents since the last call, then zero; interrupt-safe. Clockwise and
// counter-clockwise are counted apart, so a quick back-and-forth is not lost:
// bits 0-6 ccw, bits 8-14 cw, bit 15 set if the latest detent was clockwise.
uint16_t PAD_encoderTake(uint8_t knob);
#define ENC_TAKE_CCW(t)     ((uint8_t)((t) & 0x7F))
#define ENC_TAKE_CW(t)      ((uint8_t)(((t) >> 8) & 0x7F))
#define ENC_TAKE_LAST_CW(t) (((t) & 0x8000) != 0)
// Knob switches currently held (bit k = knob k), set by the main loop.
void    PAD_encoderSetHeld(uint8_t mask);

// Diagnostics, readable with CMD_GET_STATS.
typedef struct {
  uint16_t illegal[KNOB_COUNT];         // two-line jumps
  uint16_t cw[KNOB_COUNT];              // detents decoded, per direction
  uint16_t ccw[KNOB_COUNT];
  uint16_t overruns;                    // detents lost to a full accumulator (interrupt side)
  uint16_t queueDrops;                  // detents lost to a full keymap tap queue (main loop)
  uint16_t maxQueue;                    // most detents waiting to be typed at once
} padstats_t;

// -----------------------------------------------------------------------------------
// Debounce: a sample must repeat DEBOUNCE_SAMPLES times in a row to be accepted.
// -----------------------------------------------------------------------------------
#define DEBOUNCE_SAMPLES 3
typedef struct {
  uint8_t stable;
  uint8_t pending;
  uint8_t same;
} debounce_t;

void    PAD_debounceInit(debounce_t *d, uint8_t value);
// Returns 1 when the stable value changed (read it from d->stable).
uint8_t PAD_debounce(debounce_t *d, uint8_t sample);

// -----------------------------------------------------------------------------------
// Raw mode: the daemon asks for raw events with a timeout and keeps it alive with
// heartbeats (repeated CMD_RAW_MODE). Without one the pad returns to its keymap.
// -----------------------------------------------------------------------------------
#define RAW_TIMEOUT_MAX_MS 10000
typedef struct {
  uint16_t timeoutMs;                   // 0 = off
  uint16_t leftMs;
} rawmode_t;

// Returns 1 if the request was accepted (0 < timeout <= max, or 0 to turn off).
uint8_t PAD_rawRequest(rawmode_t *r, uint16_t timeoutMs);
// Advance by elapsed milliseconds; returns 1 exactly when raw mode just expired.
uint8_t PAD_rawTick(rawmode_t *r, uint16_t elapsedMs);
uint8_t PAD_rawActive(const rawmode_t *r);

// Raw event codes (report ID 5): [seq][slot][event][layer][count]
#define RAW_EVT_DOWN    1
#define RAW_EVT_UP      2
#define RAW_EVT_TAP     3               // knob detents (slot says which way, count how many)

// Layer state: base layer plus a momentary override.
typedef struct {
  uint8_t base;                         // set/toggle target, survives releases
  uint8_t momentary;                    // 0xFF = none
  uint8_t momentarySlot;                // slot holding the momentary layer
} layerstate_t;

uint8_t PAD_activeLayer(const layerstate_t *l);
// Apply a layer action on press (pressed = 1) or release (pressed = 0) of slot.
void PAD_layerAction(layerstate_t *l, uint16_t code, uint8_t slot, uint8_t pressed);

// -----------------------------------------------------------------------------------
// Pad core: input decoding and dispatch, shared by the firmware and the host tests.
// The platform supplies the PAD_hw* hooks and the keymap lookup below.
// -----------------------------------------------------------------------------------
#define PAD_TAP_GAP_MS  5               // knob detent: press, this long, release (non-blocking)
#define PAD_QUEUE_RUNS  16              // keymap tap queue: runs of detents on one slot, in order
#define PAD_RUN_MAX     200             // detents one run holds

typedef struct {
  rawmode_t    raw;
  layerstate_t layers;
  uint8_t      rawSeq;
  uint8_t      rawHeld[3];              // slots whose DOWN went out as a raw event
  uint16_t     held[SLOT_COUNT];        // packed action pressed for each held slot
  debounce_t   gpioKey;                 // key 1, 1 = pressed
  debounce_t   tm;                      // TM1650 key register
  uint8_t      tmSlot;                  // slot the TM1650 is holding down, or SLOT_NONE
  uint16_t     nowMs;                   // clock advanced by PAD_tick
  uint8_t      runSlot[PAD_QUEUE_RUNS]; // FIFO of (slot, detents): typed in the order turned
  uint8_t      runCount[PAD_QUEUE_RUNS];
  uint8_t      runHead, runLen;
  uint8_t      tapSlot;                 // detent being typed, or SLOT_NONE
  uint16_t     tapPacked;
  uint16_t     tapAt;
  padstats_t   stats;
} padstate_t;

#ifdef SDCC
extern __xdata padstate_t Pad;
#else
extern padstate_t Pad;
#endif

// Platform hooks.
uint16_t PAD_keymap(uint8_t layer, uint8_t slot);
void     PAD_hwPress(uint16_t packed);
void     PAD_hwRelease(uint16_t packed);
void     PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer, uint8_t count);
// Keep the encoder interrupt out while the main loop reads shared counters.
void     PAD_hwLock(void);
void     PAD_hwUnlock(void);

// startLayer is the layer stored as the power-on default; encPins as for the ISR.
void    PAD_init(uint8_t startLayer, uint8_t gpioKeyDown, uint8_t tmCode, uint8_t encPins);
// One main-loop pass: key 1 (1 = pressed), the TM1650 register; then the
// detents the interrupt decoded, and the keymap tap queue.
void    PAD_poll(uint8_t gpioKeyDown, uint8_t tmCode);
// Elapsed time: raw-mode heartbeat and the tap clock.
void    PAD_tick(uint16_t elapsedMs);
// Detents of one knob in one direction, as taken from the decoder (exposed for tests).
void    PAD_turn(uint8_t knob, uint8_t cw, uint8_t detents);
// Detents waiting in the keymap tap queue.
uint16_t PAD_queued(void);
// A copy of the diagnostics (interrupt-safe); clear = 1 zeroes them afterwards.
void    PAD_getStats(padstats_t *out, uint8_t clear);
// A key or knob-switch event, after debouncing (exposed for tests).
void    PAD_event(uint8_t slot, uint8_t event);
// Host request; 0 means off. Returns 0 if the timeout is out of range.
uint8_t PAD_setRaw(uint16_t timeoutMs);
// Host layer selection; drops any momentary layer.
void    PAD_setLayer(uint8_t layer);
// Release everything this pad is holding down on the host.
void    PAD_releaseAll(void);
