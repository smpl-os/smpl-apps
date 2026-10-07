// ===================================================================================
// USB side of the keymap protocol (control-surface fork of CH552-OpenMacroPad)
// ===================================================================================
// The USB interrupt parks a request; the main loop hands it to padstore.c and
// sends the reply. Storage and command handling live in padstore.c.
#pragma once
#include <stdint.h>
#include "padstore.h"

// Load from data flash, or apply and store defaults. Returns the stored start layer.
uint8_t PADCFG_init(void);
void PADCFG_onReport(void);             // called from the USB interrupt
void PADCFG_task(void);                 // called from the main loop
