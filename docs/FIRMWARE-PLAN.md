# Firmware replacement plan for the CH552 pad (1189:8890, serial key153)

Status: **plan only.** Nothing has been flashed and the bootloader has not been
entered. Every device step below needs the user's approval.

## Why replace the firmware

The stock firmware cannot be programmed reliably.
* The vendor's own protocol has no effect: our flash was byte-identical to the
  vendor app, and the same apps also failed in upstream issue #168.
* Only one literal record was ever accepted: key 1 → F14, at 10:05.
* A rewrite of that record was rejected.
* Every multi-frame write either went unapplied or left the pad silent until
  replug.

The record is in `hardware-ch552.md`. The stock firmware is therefore not worth
keeping. It also **cannot be backed up**: the WCH ROM bootloader has no read
command, and EpicLPer showed that its verify command cannot serve as a read
oracle on the CH552. No vendor image exists for download either, since the
vendor's "refresh" tools target other PIDs. **Flashing loses the stock firmware
for good.**

## 1. Firmware choice

The user asked for host-configurable keymaps, several layers or modes, flexible
per-input actions, and a raw report for the daemon. The design favours a
**simple, robust firmware**: unique input events, plus an optional standalone
keymap for use without the daemon. The **daemon does the heavy logic**: layers,
Kdenlive contexts and acceleration.

| | EpicLPer **CH552-OpenMacroPad** 1.0.0 | perkinsb1024 **CH552-Macropad-v2** | denniswjpg **padkit** | yswallow **CH552duinoKeyboard** | wagiminator **Macropad-plus/mini** | **Proposed: our fork of OpenMacroPad** |
|---|---|---|---|---|---|---|
| Licence | firmware CC BY-SA 3.0, tools MIT | **none published** (all rights reserved) | MIT | LGPL-2.1 | CC BY-SA 3.0 | CC BY-SA 3.0 (inherited); host side ours |
| Board family | **SY181 "12+3" with a TM1650 key scanner** and CH552G; key 1 on GPIO P1.5 | 3-key / 6-key, direct GPIO, WS2812 | 6-key + knob, direct GPIO, WS2812 | generic, ch55xduino | 4 keys + knob | as OpenMacroPad, extended to 15 keys + 3 encoders |
| Fits 15 keys + 3 knobs | 12 keys + 2 knobs; needs a port (TM1650 has room for 28 keys; third encoder footprint is routed) | no | no | possible with porting | no | **yes** (design target) |
| Keymap changed from host, no reflash | **yes** (vendor HID: GET/SET_ACTION, data flash, CRC) | yes (WebHID) | yes (browser) | no (compile time) | no (compile time) | **yes**, same protocol, 24 slots |
| Standalone layers | no | **5–7 layers, chords** | no | yes (compile time) | no | **optional: 2 layers** in data flash (compact 2-byte actions) + layer key |
| Action kinds | key + any modifiers, consumer, mouse | key, text, mouse, media | keys | keys | keys/consumer | key + modifiers, consumer, mouse, layer switch |
| Raw/vendor event report for a daemon | no | no | no (a daemon maps F-keys) | no | no | **yes**: per-input `down/up/cw/ccw` with sequence number, behind a heartbeat |
| Software jump to bootloader | **yes** (command + hold key 1 at plug-in) | ? | ? | ? | no | yes |
| Robustness | validation, CRC-8, defaults on corruption, self-test tool | ? | ? | ? | n/a | same + heartbeat fallback |
| Size (built here, SDCC 4.5.0) | 5784 / 14336 B | - | - | - | - | est. < 9 KB |

**Recommendation:** fork **CH552-OpenMacroPad**.
* It is the only candidate written for this board family: a TM1650 scanner and
  a CH552G, with key 1 on GPIO P1.5. Our key 1 was likewise the only key whose
  stock record ever stuck.
* Its host protocol, storage and recovery design are the most careful of the
  candidates.
* Its licence permits a fork. perkinsb1024's feature set is closest to "many
  possibilities", but it targets different hardware and has no licence.

Additions in the fork:
1. **24 slots:** keys 1–15, then knob n press, cw, ccw. Pins and scan codes come
   from the discovery run (§2).
2. **Raw mode for the daemon:**
   * The host sends `CMD_RAW_MODE {on, timeout_ms}`.
   * While heartbeats arrive, the firmware sends only vendor input reports
     `[id 4][seq][slot][event]` and no keyboard, consumer or mouse reports.
   * When heartbeats stop (daemon stopped or crashed), it releases everything
     and returns to the standalone keymap within `timeout_ms`.
   * The daemon gets unique, lossless input IDs with no chord decoding.
   * The daemon still grabs the pad's evdev nodes as a second line of defence.
3. **Optional standalone layers:**
   * Data flash is only 128 bytes. One full layer uses 24 × 4 B + a 6 B
     header = 102 B.
   * Two layers fit with compact 2-byte actions: a 2-bit type, a 4-bit
     left-modifier mask and a 10-bit code (keyboard usage, consumer usage up to
     0x3FF, or mouse subtype/delta).
   * A `layer momentary/toggle` action type switches between them.
   * Default map: keys F13–F24 and Shift+F13–F15, knobs volume and media.
4. **USB identity:**
   * Keep 1189:8890, so the existing uaccess rules apply.
   * Serial string `key153`, so the daemon's device match stays the same.
   * Product string `Control Surface 15+3`.
5. Keep `CMD_BOOTLOADER` (with a magic value), `CMD_DUMP` and the self-test.

## 2. Pin mapping without the stock firmware

**What the user does:**
* Unplug the pad and open the case. Remove the keycaps only if the screws hide
  under them.
* Photograph **both sides of the PCB** in good, even light, square-on and in
  focus, each side in full and in close-ups.
* Make sure these are readable:
  * the silkscreen board ID (EpicLPer's reads `SY181P2R03003A 12+3`);
  * the markings on U1, the CH552 variant (G = SOP-16, T = TSSOP-20);
  * the markings on U2 (TM1650 or similar, if present);
  * any two-pad header near USB (EpicLPer's is **J2**, the boot pads);
  * the encoder footprints.

Save the photos to the session files.

Then:
1. I compare the photos with EpicLPer's traced map in their `docs/HARDWARE.md`.
   If the board matches, most of the map is known already.
2. I flash `discovery.bin` (needs approval). It reports by typing hex text:
   * `BUS OK SDA=P33 SCL=P34` and the idle code;
   * `Knn` for each TM1650 key event;
   * `Gnn` for GPIO changes.
3. While the parent session grabs `event18–21` and captures `hidraw4`, the user
   presses keys 1–15 in row order, then each knob's press, three detents cw and
   three ccw. The typed codes never reach the desktop.
   * If the bus is silent, try `discovery-swapbus.bin`.
   * If there is no TM1650 at all (for example a CH552T with a direct matrix),
     a different discovery build is needed; the photos settle that first.

## 3. Entering the bootloader

The CH552's ROM bootloader (USB **4348:55e0**) cannot be erased. It starts at
power-on when the boot condition holds, and hands over to the application after
a few seconds if nothing happens. A visit to the bootloader that writes nothing
is harmless.

In order of preference:
1. **Hold the top-left key while plugging in.**
   * This works if the chip's config selects P1.5 as its boot pin and key 1 is
     wired to P1.5, as on EpicLPer's board. Unverified on stock firmware.
   * Zero hardware risk: if it fails, the pad simply boots normally.
   * Check with `lsusb -d 4348:55e0`.
2. **Short the J2 pads while plugging in** (tweezers), if the board has them.
   EpicLPer confirmed this on the 12+3 board.
3. **P3.6 (UDP/D+) to 3.3 V through a 10 kΩ resistor** while plugging in,
   e.g. U1 pin 12 to pin 16 on a CH552G. Use this only after the photos confirm
   the pins. **Never short 5 V or GND to anything.**

**Read-protection.** The stock firmware's read-protection status is unknown
and does not matter, because no read path exists at all.

## 4. Tooling (user-local on /mnt/ai, no system packages)

| Tool | Version | Location | SHA-256 |
|---|---|---|---|
| SDCC | 4.5.0 #15242 | `/mnt/ai/keypad-lab/tools/sdcc-4.5.0` | archive `3395722e…` |
| wchisp (Rust, GPL-2.0, CH552 = chip 0x52, 14 KiB, EEPROM 128 B) | 0.3.0 | `/mnt/ai/keypad-lab/tools/wchisp/bin/wchisp` | `1797200e…` |
| EpicLPer `chflash.py` (pyusb), backup flasher | 1.0.0 | `/mnt/ai/keypad-lab/fw/CH552-OpenMacroPad/tools` | n/a |

The udev rule `data/udev/71-wch-isp-bootloader.rules` gives the user access to
4348:55e0 with `TAG+="uaccess"`. Installing it **needs a pkexec approval**:

```sh
pkexec install -m644 data/udev/71-wch-isp-bootloader.rules /etc/udev/rules.d/
pkexec udevadm control --reload
```

**Candidate binaries.** These are build outputs only, nothing was flashed, and
they are kept in `/mnt/ai/keypad-lab/fw/build/openmacropad/firmware/out/`:

| Binary | Size | SHA-256 | Purpose |
|---|---|---|---|
| `discovery.bin` | 5704 B | `6ece5ffe6481874ad712ac13052d5e8b15ecdcc08a2f0fb967e6ca6121f8f49c` | mapping run, TM1650 bus SDA=P3.3 SCL=P3.4 |
| `discovery-swapbus.bin` | 5704 B | `38d7d2833af7411de656c8b03d8d6c029f026512ca82e32bac8edcad70a9b2bd` | same with the bus swapped |
| `padfw.bin` | 5784 B | `3b95f45a480070e8514f5272316cb3812395f9d3efe6cf49a9514dd8cb4858b3` | upstream 1.0.0, 12+2 map; byte-identical to EpicLPer's prebuilt, so the build is reproducible |
| `padfw-swapbus.bin` | 5784 B | `11eb52fb307091402317ec74ac130e8240832b8d825489d050e2aaca8969a375` | same with the bus swapped |

Our fork is built after the discovery run, because its key table needs the
measured codes.

## 5. Recovery

* The ROM bootloader is permanent. Method 2 or 3 of §3 always reaches it,
  whatever is in code flash. EpicLPer: "no realistic way to brick the board".
* With our firmware installed there are two more routes: `CMD_BOOTLOADER` from
  the host, and holding the top-left key while plugging in.
* `discovery.bin` is the fallback diagnostic firmware, and upstream `padfw.bin`
  is a known-good keyboard firmware.
* The keymap lives in data flash, which a code-flash erase leaves alone. A
  corrupt keymap fails its CRC at boot and is replaced by the defaults.

## 6. Procedure (each numbered step needs approval)

1. Photos of both PCB sides (§2). No risk.
2. I review the photos. I settle the chip variant, the TM1650, the boot pads,
   the third encoder's pins and the 15-key wiring, then update this plan.
3. `pkexec`-install the udev rule (§4).
4. **Point of no return.** Enter the bootloader (§3) with
   `wchisp info` running, which is read-only. Then
   `wchisp flash discovery.bin`. The stock firmware is erased.
5. Run the mapping session (§2, step 3) with the parent's grab and capture.
6. I build our fork with the measured map. Default keymap: unique F13–F24 /
   Shift codes, knobs volume and media.
7. Flash the fork through `CMD_BOOTLOADER`, or §3 if that fails.
8. Verify:
   * all 24 inputs give distinct events;
   * `GET_INFO` answers;
   * a keymap change persists across a replug;
   * raw mode works and falls back to the keymap when heartbeats stop.
9. Daemon work, which needs no device access:
   * a raw-HID input backend (vendor report 4 plus heartbeats);
   * a slot-based hardware map;
   * tests.

The service stays disabled until the user enables it.

## Risk assessment

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Stock firmware lost | **certain** at step 4 | low: it cannot be programmed reliably anyway | accepted by the user; documented |
| Brick | very low | pad unusable until reflashed | ROM bootloader and J2 / P3.6 always work |
| Wrong pads shorted while opening up | low to medium | hardware damage | follow the photos; tweezers on J2 only; never short 5 V; option 1 first |
| Wrong pin map | medium on first build | some inputs dead | discovery first; reflash is cheap |
| Discovery or fork typing into the desktop | low | stray keystrokes | parent grabs event18–21 during every session; default keymap uses unbound F13–F24 |
| Code-flash wear | negligible | - | keymap in data flash, which is rated for far more writes than code flash; code flash only for the few reflashes (~200 cycles) |
| Licence | none for personal use | - | firmware fork stays CC BY-SA 3.0 with attribution; daemon side is ours |
| Daemon and firmware mismatch | low | inputs ignored | `GET_INFO` reports firmware version and slot count; the daemon checks it |

## Progress log

* **11:5x** The user entered the ROM bootloader by holding the top-left key at
  plug-in. That confirms DOWNLOAD_CFG = P1.5 (wchisp: BTVER 02.40,
  CODE_PROTECT 0, NO_BOOT_LOAD 1, UID 6E-78-A3-4C).
* **11:57** Two `wchisp flash` attempts timed out at identify. The bootloader
  wedges after repeated sessions. Nothing was erased.
* **11:59:07–10** Fresh session (devnum 29). `wchisp flash discovery.bin`
  erased 8 sectors, wrote 6144 B, **Verify OK**, then reset. The config
  registers were read only and left unchanged. Log:
  `docs/records/flash-20261007-wchisp-discovery.log`. **The stock firmware is
  gone from here on.**
* The pad re-enumerated as 1189:8890 at full speed, "SY181 / Macropad 12+3 /
  CH552GPAD", with one HID interface: hidraw4, event18 (kbd), event19 (mouse).
  The discovery mapping session is next (the parent captures).
