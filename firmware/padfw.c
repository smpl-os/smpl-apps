// ===================================================================================
// Control-surface firmware for the 15-key, 3-knob CH552G + TM1650 pad
// ===================================================================================
//
// A fork of EpicLPer's CH552-OpenMacroPad (CC BY-SA 3.0), on wagiminator's
// CH55x USB stack. Board map in src/config.h, measured with discovery.c.
//
// Two ways to use it:
//   - Keymap mode (default): every input sends the action stored for it in the
//     active layer. Layer 0 is a distinct harmless chord per input, layer 1 a
//     standalone set. Both are editable over HID without reflashing.
//   - Raw mode: the control-surface daemon asks for raw events (report ID 5)
//     and keeps raw mode alive with heartbeats. While it is on, the pad sends no
//     keyboard, consumer or mouse output; when the heartbeats stop it falls back
//     to the keymap on its own.
//
// All input decoding and dispatch lives in src/padlogic.c, which the host test
// suite exercises; this file only connects it to the hardware.
//
// Bootloader:
//   - Hold key 1 (top left, P1.5) while plugging in. That is the CH552 ROM's
//     own download pin, so it works even if this firmware is broken.
//   - Or hold any other key or knob switch while plugging in; this firmware
//     then jumps to the bootloader itself.
//   - Or send CMD_BOOTLOADER with its 'B','L' guard over HID.

#include "src/config.h"
#include "src/system.h"
#include "src/delay.h"
#include "src/gpio.h"
#include "src/tm1650.h"
#include "src/usb_conkbd.h"
#include "src/usb_mouse.h"
#include "src/usb_hid.h"
#include "src/padcfg.h"
#include "src/padlogic.h"

void USB_interrupt(void);
void USB_ISR(void) __interrupt(INT_NO_USB) {
  USB_interrupt();
}

// 1 ms tick for the raw-mode heartbeat. An 8-bit counter is read atomically.
static volatile uint8_t msTicks = 0;
void TMR2_ISR(void) __interrupt(INT_NO_TMR2) {
  TF2 = 0;
  msTicks++;
}

#define T2_RELOAD (65536UL - (F_CPU / 12 / 1000))

static void timerInit(void) {
  T2MOD &= ~(bTMR_CLK | bT2_CLK);               // Fsys / 12
  T2CON = 0;                                    // 16-bit auto-reload timer
  RCAP2L = (uint8_t)(T2_RELOAD & 0xFF);
  RCAP2H = (uint8_t)(T2_RELOAD >> 8);
  TL2 = RCAP2L;
  TH2 = RCAP2H;
  ET2 = 1;
  TR2 = 1;
}

#pragma disable_warning 110

// ===================================================================================
// Platform hooks for padlogic
// ===================================================================================

static uint8_t modsOf(uint16_t packed) {
  uint8_t c = (uint8_t)((packed >> 8) & 0x1F);
  return (c & 0x10) ? (uint8_t)((c & 0x0F) << 4) : (uint8_t)(c & 0x0F);
}

void PAD_hwPress(uint16_t packed) {
  uint16_t code;
  switch((uint8_t)(packed >> 13)) {
    case ACT_KEY:
      KBD_pressUsage((uint8_t)(packed & 0xFF), modsOf(packed));
      break;
    case ACT_CON:
      CON_press(packed & 0x3FF);
      break;
    case ACT_MOUSE:
      code = packed & 0x7FF;
      switch((uint8_t)(code >> 8)) {
        case MS_BUTTON: MOUSE_press((uint8_t)code);            break;
        case MS_WHEEL:  MOUSE_wheel((int8_t)code);             break;
        case MS_PAN:    MOUSE_pan((int8_t)code);               break;
        case MS_MOVE_X: MOUSE_move((int8_t)code, 0);           break;
        case MS_MOVE_Y: MOUSE_move(0, (int8_t)code);           break;
      }
      break;
    default:
      break;
  }
}

void PAD_hwRelease(uint16_t packed) {
  uint8_t s, keep = 0;
  switch((uint8_t)(packed >> 13)) {
    case ACT_KEY:
      // A modifier another held chord still needs stays down.
      for(s = 0; s < SLOT_COUNT; s++)
        if((Pad.held[s] >> 13) == ACT_KEY) keep |= modsOf(Pad.held[s]);
      KBD_releaseUsage((uint8_t)(packed & 0xFF), modsOf(packed) & (uint8_t)~keep);
      break;
    case ACT_CON:
      CON_release(packed & 0x3FF);
      break;
    case ACT_MOUSE:
      // Wheel, pan and movement are one-shot deltas with nothing to release.
      if(((packed >> 8) & 0x07) == MS_BUTTON) MOUSE_release((uint8_t)(packed & 0xFF));
      break;
    default:
      break;
  }
}

void PAD_hwWait(uint8_t ms) {
  DLY_ms(ms);
}

static __xdata uint8_t rawReport[5];

void PAD_hwRaw(uint8_t seq, uint8_t slot, uint8_t event, uint8_t layer) {
  rawReport[0] = RAW_REPORT_ID;
  rawReport[1] = seq;
  rawReport[2] = slot;
  rawReport[3] = event;
  rawReport[4] = layer;
  HID_sendReport(rawReport, sizeof(rawReport));
}

// ===================================================================================
// Input sampling
// ===================================================================================

static __xdata uint8_t encA[KNOB_COUNT];
static __xdata uint8_t encB[KNOB_COUNT];

static void sampleEncoders(void) {
  encA[0] = PIN_read(PIN_ENC1_A) ? 1 : 0;
  encB[0] = PIN_read(PIN_ENC1_B) ? 1 : 0;
  encA[1] = PIN_read(PIN_ENC2_A) ? 1 : 0;
  encB[1] = PIN_read(PIN_ENC2_B) ? 1 : 0;
  encA[2] = PIN_read(PIN_ENC3_A) ? 1 : 0;
  encB[2] = PIN_read(PIN_ENC3_B) ? 1 : 0;
}

// ===================================================================================
// Main
// ===================================================================================

void main(void) {
  uint8_t busOK, key, now, last, startLayer;

  CLK_config();
  DLY_ms(10);

  PIN_input_PU(PIN_KEY1);
  PIN_input_PU(PIN_ENC1_A);
  PIN_input_PU(PIN_ENC1_B);
  PIN_input_PU(PIN_ENC2_A);
  PIN_input_PU(PIN_ENC2_B);
  PIN_input_PU(PIN_ENC3_A);
  PIN_input_PU(PIN_ENC3_B);
  DLY_ms(2);

  // Escape hatches, checked before anything that could go wrong. The ROM has
  // already looked at P1.5 by now; checking it again costs nothing.
  if(!PIN_read(PIN_KEY1)) BOOT_now();
  busOK = TM1650_init();
  DLY_ms(20);                                   // let the chip scan the matrix once
  if(busOK && (TM1650_readKey() & TM1650_KEY_PRESSED)) BOOT_now();

  startLayer = PADCFG_init();
  sampleEncoders();
  PAD_init(startLayer, 0, TM1650_readKey(), encA, encB);

  KBD_init();
  timerInit();
  last = msTicks;

  while(1) {
    PADCFG_task();                              // pending host command, if any

    key = TM1650_readKey();
    sampleEncoders();
    PAD_poll(!PIN_read(PIN_KEY1), key, encA, encB);

    now = msTicks;
    PAD_tick((uint8_t)(now - last));
    last = now;

    DLY_ms(1);
  }
}
