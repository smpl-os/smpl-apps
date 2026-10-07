// ===================================================================================
// USB side of the keymap protocol (see padcfg.h)
// ===================================================================================
// Based on CH552-OpenMacroPad's padcfg.c (CC BY-SA 3.0).

#include "ch554.h"
#include "system.h"
#include "delay.h"
#include "padcfg.h"
#include "usb_descr.h"
#include "usb_handler.h"
#include "usb_hid.h"

// Command handoff: the USB interrupt only parks the request.
static __xdata uint8_t cmdBuf[16];
static volatile __bit cmdPending = 0;
static __xdata uint8_t reply[16];

uint8_t PADCFG_init(void) {
  return PADSTORE_init();
}

void PADCFG_onReport(void) {
  uint8_t i;
  if(cmdPending) return;                        // still working on the last one
  if(EP2_buffer[0] != CFG_REPORT_ID) return;    // not for us, probably keyboard LEDs
  for(i = 0; i < sizeof(cmdBuf); i++) cmdBuf[i] = EP2_buffer[i];
  cmdPending = 1;
}

void PADCFG_task(void) {
  uint8_t boot;
  if(!cmdPending) return;
  boot = PADSTORE_handle(cmdBuf, reply);
  cmdPending = 0;
  if(boot) PAD_releaseAll();                    // nothing stays pressed on the host
  HID_sendReport(reply, sizeof(reply));
  if(boot) {
    DLY_ms(50);                                 // let the reply go out first
    BOOT_now();
  }
}
