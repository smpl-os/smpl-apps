# Control-surface pad firmware

Firmware for the 15-key, 3-knob CH552G + TM1650 macro pad (USB 1189:8890,
serial `key153`). It replaces the stock firmware, which could not be
programmed reliably (`docs/hardware-ch552.md`).

Plan, measured map, protocol, tests, flash procedure and recovery:
**`docs/FIRMWARE-PLAN.md` §7**.

## Origin and licence

This directory is a fork of **EpicLPer's CH552-OpenMacroPad**
(<https://github.com/EpicLPer/CH552-OpenMacroPad>), which builds on
**Stefan Wagner's** CH55x USB HID code (<https://github.com/wagiminator>,
CH552-Macropad-mini and CH552-USB-Knob). Those are licensed
**CC BY-SA 3.0**, and the firmware here is compiled together with them, so
everything under `firmware/` and the two host test files that compile it
(`tests/tst_fw_logic.c`, `tests/tst_fw_store.c`) are distributed under
**CC BY-SA 3.0** as well. See `LICENSE.upstream` for the full notice. The rest
of this repository is GPL-2.0-or-later. `src/ch554.h` is WCH's register header.

Changes from upstream (2.0.0):
* board map for this 15+3 pad, measured with `discovery.c` (key 1 on P1.5,
  14 TM1650 keys, three encoders and their switches);
* all input decoding and dispatch moved into hardware-free C
  (`src/padlogic.c`): quadrature decoding that ignores the line a knob press
  pulls low, debounce, layer-aware dispatch, raw event mode with heartbeat;
* keymap storage and host protocol moved into `src/padstore.c`: 24 slots,
  two layers of compact 2-byte actions in data flash, CRC and validity
  checks, protocol v3 (`CMD_RAW_MODE`, `CMD_SET_LAYER`, layer-addressed
  GET/SET);
* raw input report ID 5 in the vendor collection;
* `KBD_pressUsage`/`KBD_releaseUsage` take HID usages directly;
* USB strings `OpenMacroPad` / `Control Surface 15+3` / `key153`, bcdDevice 2.00;
* `build.py --out DIR`; the build never flashes.

## Files

| File | Role |
|---|---|
| `padfw.c` | main loop, pins, Timer2 tick, platform hooks |
| `src/padlogic.[ch]` | slots, key map, encoder, debounce, raw mode, layers, defaults (host-tested) |
| `src/padstore.[ch]` | data-flash keymap and protocol v3 (host-tested) |
| `src/padcfg.[ch]` | USB hand-off between the interrupt and `padstore` |
| `src/config.h` | pins and USB strings |
| `discovery.c` | diagnostic firmware that types what the hardware does |
| `padctl.py` | host tool for protocol v3 over hidraw |
| `flash-and-verify.sh` | one-step flash and read-only verification |
| `build.py` | SDCC build |
