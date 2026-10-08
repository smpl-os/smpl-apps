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

// Command handoff: the USB interrupt only parks the request in PADSTORE_req.
static volatile __bit cmdPending = 0;

uint8_t PADCFG_init(void) {
  return PADSTORE_init();
}

// Runs in the USB interrupt: its local must not share memory with main-loop
// functions (SDCC overlays non-reentrant locals otherwise).
#ifdef SDCC
#pragma save
#pragma nooverlay
#endif
void PADCFG_onReport(void) {
  uint8_t i;
  if(EP2_buffer[0] != CFG_REPORT_ID) return;    // not for us, probably keyboard LEDs
  if(cmdPending) return;                        // cannot happen: EP2 NAKs meanwhile
  for(i = 0; i < sizeof(PADSTORE_req); i++) PADSTORE_req[i] = EP2_buffer[i];
  cmdPending = 1;
  // Hold the next OUT report (NAK) until this one is answered. The host
  // controller retries it, so a heartbeat right after another command is
  // delayed by a millisecond instead of lost (2.0.1 dropped it).
  UEP2_CTRL = UEP2_CTRL & ~MASK_UEP_R_RES | UEP_R_RES_NAK;
}
#ifdef SDCC
#pragma restore
#endif

void PADCFG_task(void) {
  uint8_t boot;
  if(!cmdPending) return;
  boot = PADSTORE_handle();
  if(boot) PAD_releaseAll();                    // nothing stays pressed on the host
  HID_sendReport(PADSTORE_reply, sizeof(PADSTORE_reply));
  if(boot) {
    DLY_ms(50);                                 // let the reply go out first
    BOOT_now();
  }
  // Answered (the reply is copied out): take the next report.
  IE_USB = 0;
  cmdPending = 0;
  UEP2_CTRL = UEP2_CTRL & ~MASK_UEP_R_RES | UEP_R_RES_ACK;
  IE_USB = 1;
}
