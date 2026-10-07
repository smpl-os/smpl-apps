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
// Encoder: detented, both lines high at rest, one full quadrature cycle per detent.
// Clockwise is A falling first: (A,B) 11 -> 01 -> 00 -> 10 -> 11.
//
// Each valid single-line transition counts +-1; a step is reported only when the
// encoder is back at rest with |count| >= 2. A knob press on this board pulls one
// encoder line low (top: B, middle: B): that is one transition out and one back,
// so it nets to zero and is never a step. Contact bounce cancels the same way,
// and a skipped intermediate state still leaves |count| >= 2. While the knob's
// switch is held the count is frozen at zero.
// -----------------------------------------------------------------------------------
typedef struct {
  uint8_t state;                        // last (A << 1) | B
  int8_t  count;                        // quarter steps since the last rest
} encstate_t;

void   PAD_encoderInit(encstate_t *e, uint8_t a, uint8_t b);
// Returns +1 (clockwise), -1 or 0.
int8_t PAD_encoderUpdate(encstate_t *e, uint8_t a, uint8_t b, uint8_t switchHeld);

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

// Raw event codes (report ID 5): [seq][slot][event][layer]
#define RAW_EVT_DOWN    1
#define RAW_EVT_UP      2
#define RAW_EVT_TAP     3               // one knob detent (slot says which way)

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
#define PAD_TAP_GAP_MS  15              // knob detents press, wait, release

typedef struct {
  rawmode_t    raw;
  layerstate_t layers;
  uint8_t      rawSeq;
  uint8_t      rawHeld[3];              // slots whose DOWN went out as a raw event
  uint16_t     held[SLOT_COUNT];        // packed action pressed for each held slot
  debounce_t   gpioKey;                 // key 1, 1 = pressed
  debounce_t   tm;                      // TM1650 key register
  uint8_t      tmSlot;                  // slot the TM1650 is holding down, or SLOT_NONE
  encstate_t   enc[KNOB_COUNT];
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
void     PAD_hwWait(uint8_t ms);
void     PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer);

// startLayer is the layer stored as the power-on default.
void    PAD_init(uint8_t startLayer, uint8_t gpioKeyDown, uint8_t tmCode,
                 const uint8_t *encA, const uint8_t *encB);
// One poll: key 1 (1 = pressed), the TM1650 register, encoder lines (1 = high).
void    PAD_poll(uint8_t gpioKeyDown, uint8_t tmCode,
                 const uint8_t *encA, const uint8_t *encB);
// Elapsed time for the raw-mode heartbeat.
void    PAD_tick(uint16_t elapsedMs);
// A slot event, after decoding (exposed for tests).
void    PAD_event(uint8_t slot, uint8_t event);
// Host request; 0 means off. Returns 0 if the timeout is out of range.
uint8_t PAD_setRaw(uint16_t timeoutMs);
// Host layer selection; drops any momentary layer.
void    PAD_setLayer(uint8_t layer);
// Release everything this pad is holding down on the host.
void    PAD_releaseAll(void);
