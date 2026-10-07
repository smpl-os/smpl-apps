// ===================================================================================
// Pure input and keymap logic (see padlogic.h)
// ===================================================================================
#include "padlogic.h"

#ifdef SDCC
#define PAD_CODE __code
#else
#define PAD_CODE
#endif

// Measured with discovery.c on serial key153 (2026-10-07, captures discovery2-5),
// holding the pad with the knobs on the right. Key 1 is on GPIO P1.5, so the
// TM1650 codes start at key 2; the matrix order does not follow the rows. The
// three knob switches share DIG4.
static PAD_CODE const uint8_t KEYCODE_SLOT[][2] = {
  /* row 1 */             {0x44, 1},  {0x4C, 2},  {0x54, 3},  {0x5C, 4},
  /* row 2 */ {0x64, 5},  {0x45, 6},  {0x4D, 7},  {0x55, 8},  {0x5D, 9},
  /* row 3 */ {0x65, 10}, {0x46, 11}, {0x4E, 12}, {0x56, 13}, {0x5E, 14},
  {0x67, SLOT_KNOB(0, KNOB_PRESS)},     // top knob, also pulls P1.4 low
  {0x5F, SLOT_KNOB(1, KNOB_PRESS)},     // middle knob, also pulls P1.6 low
  {0x57, SLOT_KNOB(2, KNOB_PRESS)},     // bottom knob
};
#define KEYCODE_SLOT_COUNT (sizeof(KEYCODE_SLOT) / sizeof(KEYCODE_SLOT[0]))

uint8_t PAD_slotForKeycode(uint8_t keycode) {
  uint8_t i;
  if(!(keycode & 0x40)) return SLOT_NONE;       // only presses map to a slot
  for(i = 0; i < KEYCODE_SLOT_COUNT; i++)
    if(KEYCODE_SLOT[i][0] == keycode) return KEYCODE_SLOT[i][1];
  return SLOT_NONE;
}

// -----------------------------------------------------------------------------------
// Default keymap
// -----------------------------------------------------------------------------------

#define K(m, u)  ((uint16_t)(ACT_KEY << 13) | (uint16_t)((m) << 8) | (u))
#define C(u)     ((uint16_t)(ACT_CON << 13) | (u))
#define M(s, v)  ((uint16_t)(ACT_MOUSE << 13) | (uint16_t)((s) << 8) | (v))
#define L(o, t)  ((uint16_t)(ACT_LAYER << 13) | (uint16_t)((o) << 8) | (t))

// Layer 0: one distinct, harmless chord per input, F14-F19 x {none, LShift,
// LCtrl, LAlt} (compact modifier bits: ctrl 0x01, shift 0x02, alt 0x04), which
// is the daemon's scheme, so the daemon works the same with or without raw mode.
// Layer 1: a standalone set (F13-F24, media, volume, scroll, arrows); knob 3
// press toggles between the layers.
static PAD_CODE const uint16_t DEFAULTS[LAYER_COUNT][SLOT_COUNT] = {
  {
    K(0x00, 0x69), K(0x00, 0x6A), K(0x00, 0x6B), K(0x00, 0x6C), K(0x00, 0x6D), K(0x00, 0x6E),
    K(0x02, 0x69), K(0x02, 0x6A), K(0x02, 0x6B), K(0x02, 0x6C), K(0x02, 0x6D), K(0x02, 0x6E),
    K(0x01, 0x69), K(0x01, 0x6A), K(0x01, 0x6B), K(0x01, 0x6C), K(0x01, 0x6D), K(0x01, 0x6E),
    K(0x04, 0x69), K(0x04, 0x6A), K(0x04, 0x6B), K(0x04, 0x6C), K(0x04, 0x6D), K(0x04, 0x6E),
  },
  {
    K(0, 0x68), K(0, 0x69), K(0, 0x6A), K(0, 0x6B), K(0, 0x6C),          // keys 1-5:  F13-F17
    K(0, 0x6D), K(0, 0x6E), K(0, 0x6F), K(0, 0x70), K(0, 0x71),          // keys 6-10: F18-F22
    K(0, 0x72), K(0, 0x73), C(0xCD), C(0xB6), C(0xB5),                   // F23 F24 play/pause prev next
    C(0xEA), C(0xE2), C(0xE9),                                           // knob 1: vol- mute vol+
    M(MS_WHEEL, 0xFF), M(MS_BUTTON, 0x04), M(MS_WHEEL, 0x01),            // knob 2: scroll, middle click
    K(0, 0x50), L(LAYER_OP_TOGGLE, 0), K(0, 0x4F),                       // knob 3: left, layer, right
  },
};

uint16_t PAD_defaultAction(uint8_t layer, uint8_t slot) {
  if(layer >= LAYER_COUNT || slot >= SLOT_COUNT) return 0;
  return DEFAULTS[layer][slot];
}

// -----------------------------------------------------------------------------------
// Actions
// -----------------------------------------------------------------------------------

// Left-hand modifiers are bits 0-3, right-hand bits 4-7 of the host mask.
static uint8_t modsToCompact(uint8_t mod, uint8_t *ok) {
  uint8_t left = mod & 0x0F, right = mod >> 4;
  *ok = 1;
  if(left && right) {
    *ok = 0;                                    // mixed hands: not storable
    return 0;
  }
  if(right) return right | 0x10;
  return left;
}

static uint8_t modsFromCompact(uint8_t c) {
  return (c & 0x10) ? (uint8_t)((c & 0x0F) << 4) : (uint8_t)(c & 0x0F);
}

uint8_t PAD_validAction(uint8_t type, uint8_t mod, uint16_t code) {
  uint8_t ok;
  switch(type) {
    case ACT_NONE:
      return (mod == 0) && (code == 0);
    case ACT_KEY:
      if(code == 0 || code > 0xE7) return 0;    // keyboard page ends at 0xE7
      modsToCompact(mod, &ok);
      return ok;
    case ACT_CON:
      return (mod == 0) && (code > 0) && (code <= CON_CODE_MAX);
    case ACT_MOUSE:
      if(mod) return 0;
      if((code >> 8) > MS_SUB_MAX) return 0;
      if((code >> 8) == MS_BUTTON) return ((code & 0xFF) != 0) && ((code & 0xFF) <= 0x1F);
      return (code & 0xFF) != 0;                // a zero delta would do nothing
    case ACT_LAYER:
      if(mod) return 0;
      if((code >> 8) > LAYER_OP_MAX) return 0;
      return (code & 0xFF) < LAYER_COUNT;
    default:
      return 0;
  }
}

uint8_t PAD_encode(uint8_t type, uint8_t mod, uint16_t code, uint16_t *out) {
  uint8_t ok;
  uint16_t m;
  if(!PAD_validAction(type, mod, code)) return 0;
  switch(type) {
    case ACT_KEY:
      m = modsToCompact(mod, &ok);
      *out = (uint16_t)(ACT_KEY << 13) | (uint16_t)(m << 8) | (code & 0xFF);
      break;
    case ACT_CON:
      *out = (uint16_t)(ACT_CON << 13) | (code & 0x3FF);
      break;
    case ACT_MOUSE:
      *out = (uint16_t)(ACT_MOUSE << 13) | (code & 0x7FF);
      break;
    case ACT_LAYER:
      *out = (uint16_t)(ACT_LAYER << 13) | (code & 0x3FF);
      break;
    default:
      *out = 0;
      break;
  }
  return 1;
}

void PAD_decode(uint16_t packed, uint8_t *type, uint8_t *mod, uint16_t *code) {
  *type = (uint8_t)(packed >> 13);
  *mod = 0;
  *code = 0;
  switch(*type) {
    case ACT_KEY:
      *mod = modsFromCompact((uint8_t)((packed >> 8) & 0x1F));
      *code = packed & 0xFF;
      break;
    case ACT_CON:
      *code = packed & 0x3FF;
      break;
    case ACT_MOUSE:
      *code = packed & 0x7FF;
      break;
    case ACT_LAYER:
      *code = packed & 0x3FF;
      break;
    default:
      *type = ACT_NONE;
      break;
  }
}

uint8_t PAD_validPacked(uint16_t packed) {
  uint8_t type, mod;
  uint16_t code, again;
  if(packed == 0) return 1;                     // ACT_NONE
  PAD_decode(packed, &type, &mod, &code);
  if(type == ACT_NONE) return 0;                // nonzero junk with an unknown type
  // Bits a type does not use must be zero: re-encoding must give the same value.
  if(!PAD_encode(type, mod, code, &again)) return 0;
  return again == packed;
}

uint8_t PAD_crc8(const uint8_t *data, uint8_t len) {
  uint8_t i, bit, crc = 0xFF;
  for(i = 0; i < len; i++) {
    crc ^= data[i];
    for(bit = 0; bit < 8; bit++)
      crc = (crc & 0x80) ? (uint8_t)((crc << 1) ^ 0x07) : (uint8_t)(crc << 1);
  }
  return crc;
}

// -----------------------------------------------------------------------------------
// Encoder
// -----------------------------------------------------------------------------------

#define ENC_REST 3                              // both lines high

// Quarter-step delta for old state * 4 + new state; state = (A << 1) | B.
// Clockwise runs 3 -> 1 -> 0 -> 2 -> 3. Two-line jumps carry no direction: 0.
static PAD_CODE const int8_t ENC_DELTA[16] = {
  /* from 0 */  0, -1, +1,  0,
  /* from 1 */ +1,  0,  0, -1,
  /* from 2 */ -1,  0,  0, +1,
  /* from 3 */  0, +1, -1,  0,
};

void PAD_encoderInit(encstate_t *e, uint8_t a, uint8_t b) {
  e->state = (uint8_t)((a ? 2 : 0) | (b ? 1 : 0));
  e->count = 0;
}

int8_t PAD_encoderUpdate(encstate_t *e, uint8_t a, uint8_t b, uint8_t switchHeld) {
  uint8_t now = (uint8_t)((a ? 2 : 0) | (b ? 1 : 0));
  int8_t step = 0;
  if(now == e->state) return 0;
  if(switchHeld) e->count = 0;                  // pressing moves a line; ignore it
  else {
    e->count += ENC_DELTA[(e->state << 2) | now];
    if(e->count > 8) e->count = 8;              // runaway bounce stays bounded
    if(e->count < -8) e->count = -8;
  }
  e->state = now;
  if(now == ENC_REST) {
    if(e->count >= 2) step = 1;
    else if(e->count <= -2) step = -1;
    e->count = 0;
  }
  return step;
}

// -----------------------------------------------------------------------------------
// Debounce
// -----------------------------------------------------------------------------------

void PAD_debounceInit(debounce_t *d, uint8_t value) {
  d->stable = value;
  d->pending = value;
  d->same = 0;
}

uint8_t PAD_debounce(debounce_t *d, uint8_t sample) {
  if(sample == d->stable) {
    d->same = 0;                                // bounced back: start over
    d->pending = sample;
    return 0;
  }
  if(sample != d->pending) {
    d->pending = sample;
    d->same = 1;
  } else {
    d->same++;
  }
  if(d->same >= DEBOUNCE_SAMPLES) {
    d->stable = sample;
    d->same = 0;
    return 1;
  }
  return 0;
}

// -----------------------------------------------------------------------------------
// Raw mode
// -----------------------------------------------------------------------------------

uint8_t PAD_rawRequest(rawmode_t *r, uint16_t timeoutMs) {
  if(timeoutMs > RAW_TIMEOUT_MAX_MS) return 0;
  r->timeoutMs = timeoutMs;
  r->leftMs = timeoutMs;
  return 1;
}

uint8_t PAD_rawTick(rawmode_t *r, uint16_t elapsedMs) {
  if(r->timeoutMs == 0) return 0;
  if(elapsedMs < r->leftMs) {
    r->leftMs -= elapsedMs;
    return 0;
  }
  r->timeoutMs = 0;
  r->leftMs = 0;
  return 1;
}

uint8_t PAD_rawActive(const rawmode_t *r) {
  return r->timeoutMs != 0;
}

// -----------------------------------------------------------------------------------
// Layers
// -----------------------------------------------------------------------------------

uint8_t PAD_activeLayer(const layerstate_t *l) {
  return l->momentary != 0xFF ? l->momentary : l->base;
}

void PAD_layerAction(layerstate_t *l, uint16_t code, uint8_t slot, uint8_t pressed) {
  uint8_t op = (uint8_t)(code >> 8), target = (uint8_t)(code & 0xFF);
  if(target >= LAYER_COUNT) return;
  switch(op) {
    case LAYER_OP_TOGGLE:
      if(pressed) l->base = l->base ? 0 : 1;
      break;
    case LAYER_OP_SET:
      if(pressed) l->base = target;
      break;
    case LAYER_OP_MOMENTARY:
      if(pressed) {
        l->momentary = target;
        l->momentarySlot = slot;
      } else if(l->momentarySlot == slot) {
        l->momentary = 0xFF;
        l->momentarySlot = SLOT_NONE;
      }
      break;
    default:
      break;
  }
}

// -----------------------------------------------------------------------------------
// Pad core
// -----------------------------------------------------------------------------------

#ifdef SDCC
__xdata padstate_t Pad;
#else
padstate_t Pad;
#endif

static uint8_t rawHeldGet(uint8_t slot) {
  return (Pad.rawHeld[slot >> 3] >> (slot & 7)) & 1;
}

static void rawHeldSet(uint8_t slot, uint8_t on) {
  if(on) Pad.rawHeld[slot >> 3] |= (uint8_t)(1 << (slot & 7));
  else   Pad.rawHeld[slot >> 3] &= (uint8_t)~(1 << (slot & 7));
}

static void emitRaw(uint8_t slot, uint8_t event) {
  Pad.rawSeq++;
  PAD_hwRaw(Pad.rawSeq, slot, event, PAD_activeLayer(&Pad.layers));
}

// Keymap side of a press. Layer actions change state; everything else goes out.
static void actionDown(uint8_t slot, uint16_t packed) {
  if((packed >> 13) == ACT_LAYER) PAD_layerAction(&Pad.layers, packed & 0x3FF, slot, 1);
  else PAD_hwPress(packed);
}

static void actionUp(uint8_t slot, uint16_t packed) {
  if((packed >> 13) == ACT_LAYER) PAD_layerAction(&Pad.layers, packed & 0x3FF, slot, 0);
  else PAD_hwRelease(packed);
}

void PAD_event(uint8_t slot, uint8_t event) {
  uint16_t packed;
  if(slot >= SLOT_COUNT) return;

  if(PAD_rawActive(&Pad.raw)) {
    switch(event) {
      case RAW_EVT_DOWN:
        if(Pad.held[slot]) return;              // went down in keymap mode; ignore
        rawHeldSet(slot, 1);
        emitRaw(slot, RAW_EVT_DOWN);
        break;
      case RAW_EVT_UP:
        if(!rawHeldGet(slot)) return;           // no DOWN was sent for it
        rawHeldSet(slot, 0);
        emitRaw(slot, RAW_EVT_UP);
        break;
      case RAW_EVT_TAP:
        emitRaw(slot, RAW_EVT_TAP);
        break;
    }
    return;
  }

  switch(event) {
    case RAW_EVT_DOWN:
      if(Pad.held[slot]) return;
      packed = PAD_keymap(PAD_activeLayer(&Pad.layers), slot);
      if(!packed) return;
      Pad.held[slot] = packed;                  // release what was pressed, even
      actionDown(slot, packed);                 // if the layer changes meanwhile
      break;
    case RAW_EVT_UP:
      packed = Pad.held[slot];
      if(!packed) return;
      Pad.held[slot] = 0;
      actionUp(slot, packed);
      break;
    case RAW_EVT_TAP:
      packed = PAD_keymap(PAD_activeLayer(&Pad.layers), slot);
      if(!packed) return;
      if((packed >> 13) == ACT_LAYER) {
        // A detent is press and release at once: momentary does nothing.
        PAD_layerAction(&Pad.layers, packed & 0x3FF, slot, 1);
        PAD_layerAction(&Pad.layers, packed & 0x3FF, slot, 0);
        return;
      }
      if(((packed >> 13) == ACT_MOUSE) && (((packed >> 8) & 0x07) != MS_BUTTON)) {
        PAD_hwPress(packed);                    // wheel, pan, move: one-shot deltas
        return;
      }
      PAD_hwPress(packed);
      PAD_hwWait(PAD_TAP_GAP_MS);
      PAD_hwRelease(packed);
      break;
  }
}

void PAD_releaseAll(void) {
  uint8_t s;
  for(s = 0; s < SLOT_COUNT; s++) {
    if(Pad.held[s]) {
      uint16_t packed = Pad.held[s];
      Pad.held[s] = 0;
      if((packed >> 13) == ACT_LAYER) PAD_layerAction(&Pad.layers, packed & 0x3FF, s, 0);
      else PAD_hwRelease(packed);
    }
  }
}

uint8_t PAD_setRaw(uint16_t timeoutMs) {
  uint8_t was = PAD_rawActive(&Pad.raw);
  if(!PAD_rawRequest(&Pad.raw, timeoutMs)) return 0;
  if(!was && PAD_rawActive(&Pad.raw)) {
    PAD_releaseAll();                           // nothing stays stuck on the host
  } else if(was && !PAD_rawActive(&Pad.raw)) {
    Pad.rawHeld[0] = Pad.rawHeld[1] = Pad.rawHeld[2] = 0;
  }
  return 1;
}

void PAD_tick(uint16_t elapsedMs) {
  if(PAD_rawTick(&Pad.raw, elapsedMs)) {
    // Heartbeat lost: back to the keymap. Keys still down stay silent until
    // they come up, so nothing is pressed that the user did not press anew.
    Pad.rawHeld[0] = Pad.rawHeld[1] = Pad.rawHeld[2] = 0;
  }
}

void PAD_setLayer(uint8_t layer) {
  if(layer >= LAYER_COUNT) return;
  Pad.layers.base = layer;
  Pad.layers.momentary = 0xFF;
  Pad.layers.momentarySlot = SLOT_NONE;
}

void PAD_init(uint8_t startLayer, uint8_t gpioKeyDown, uint8_t tmCode,
              const uint8_t *encA, const uint8_t *encB) {
  uint8_t i;
  Pad.raw.timeoutMs = 0;
  Pad.raw.leftMs = 0;
  Pad.layers.base = startLayer < LAYER_COUNT ? startLayer : 0;
  Pad.layers.momentary = 0xFF;
  Pad.layers.momentarySlot = SLOT_NONE;
  Pad.rawSeq = 0;
  Pad.rawHeld[0] = Pad.rawHeld[1] = Pad.rawHeld[2] = 0;
  for(i = 0; i < SLOT_COUNT; i++) Pad.held[i] = 0;
  // Whatever is down at start-up counts as the resting state: it produces no
  // event until it is released and pressed again.
  PAD_debounceInit(&Pad.gpioKey, gpioKeyDown ? 1 : 0);
  PAD_debounceInit(&Pad.tm, tmCode);
  Pad.tmSlot = PAD_slotForKeycode(tmCode);
  for(i = 0; i < KNOB_COUNT; i++) PAD_encoderInit(&Pad.enc[i], encA[i], encB[i]);
}

void PAD_poll(uint8_t gpioKeyDown, uint8_t tmCode,
              const uint8_t *encA, const uint8_t *encB) {
  uint8_t i, slot;
  int8_t step;

  if(PAD_debounce(&Pad.gpioKey, gpioKeyDown ? 1 : 0))
    PAD_event(SLOT_GPIO_KEY, Pad.gpioKey.stable ? RAW_EVT_DOWN : RAW_EVT_UP);

  // The TM1650 holds one key at a time: a new code releases the previous slot.
  if(PAD_debounce(&Pad.tm, tmCode)) {
    slot = PAD_slotForKeycode(Pad.tm.stable);
    if(slot != Pad.tmSlot) {
      if(Pad.tmSlot != SLOT_NONE) PAD_event(Pad.tmSlot, RAW_EVT_UP);
      Pad.tmSlot = slot;
      if(slot != SLOT_NONE) PAD_event(slot, RAW_EVT_DOWN);
    }
  }

  for(i = 0; i < KNOB_COUNT; i++) {
    step = PAD_encoderUpdate(&Pad.enc[i], encA[i], encB[i],
                             Pad.tmSlot == SLOT_KNOB(i, KNOB_PRESS));
    if(step > 0) PAD_event(SLOT_KNOB(i, KNOB_CW), RAW_EVT_TAP);
    else if(step < 0) PAD_event(SLOT_KNOB(i, KNOB_CCW), RAW_EVT_TAP);
  }
}
