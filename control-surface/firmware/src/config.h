// ===================================================================================
// User configuration for the 15-key, 3-knob CH552G + TM1650 pad (1189:8890, key153)
// ===================================================================================
//
// Every assignment below was measured with discovery.c on this unit (captures
// discovery2-5, 2026-10-07), holding the pad with the knobs on the right. See
// docs/FIRMWARE-PLAN.md for the capture record.

#pragma once

// -----------------------------------------------------------------------------------
// TM1650 two-wire bus (keys 2-15 and the three knob switches)
// -----------------------------------------------------------------------------------
// Confirmed by discovery ("BUS OK SDA=P33 SCL=P34"). -DTM1650_SWAP_BUS remains
// for other boards; with the lines exchanged the bus simply reads silence.

#ifdef TM1650_SWAP_BUS
  #define PIN_TM_SDA        P34
  #define PIN_TM_SCL        P33
#else
  #define PIN_TM_SDA        P33
  #define PIN_TM_SCL        P34
#endif

// -----------------------------------------------------------------------------------
// GPIO inputs
// -----------------------------------------------------------------------------------
// Key 1 (top left) is wired straight to P1.5, active low. P1.5 is also the ROM
// bootloader's download pin on this chip (DOWNLOAD_CFG), which is why holding
// key 1 while plugging in reaches the bootloader whatever firmware is loaded.

#define PIN_KEY1          P15

// Encoder A/B lines, both high at rest. Clockwise is A falling first.
// A knob press also pulls its B line low (top and middle measured); the
// decoder in padlogic.c cancels that.
#define PIN_ENC1_A        P32         // top knob
#define PIN_ENC1_B        P14
#define PIN_ENC2_A        P17         // middle knob
#define PIN_ENC2_B        P16
#define PIN_ENC3_A        P30         // bottom knob
#define PIN_ENC3_B        P11

// P3.1 never moved during discovery and is left alone.

// The eight pins the discovery firmware watches. Kept so discovery.c still
// builds; the real firmware uses the specific assignments above.
#define GPIO_CANDIDATES   P32, P14, P15, P16, P17, P31, P30, P11
#define GPIO_CANDIDATE_COUNT  8

// -----------------------------------------------------------------------------------
// USB device descriptor
// -----------------------------------------------------------------------------------
// Same VID:PID and serial as the stock firmware, so the daemon's device match
// (1189:8890, serial key153) and the udev uaccess rule keep working.

#define USB_VENDOR_ID       0x1189      // VID
#define USB_PRODUCT_ID      0x8890      // PID
#define USB_DEVICE_VERSION  0x0200      // v2.0 (BCD): control-surface fork

// USB configuration descriptor
#define USB_MAX_POWER_mA    100         // max power in mA

// USB descriptor strings
#define MANUFACTURER_STR    'O','p','e','n','M','a','c','r','o','P','a','d'
#define PRODUCT_STR         'C','o','n','t','r','o','l',' ','S','u','r','f','a','c','e',' ','1','5','+','3'
#define SERIAL_STR          'k','e','y','1','5','3'
#define INTERFACE_STR       'H','I','D','-','K','e','y','b','o','a','r','d'
