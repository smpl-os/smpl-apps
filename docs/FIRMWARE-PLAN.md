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
   * a raw-HID input backend (vendor report 5 plus heartbeats);
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
* **Discovery captures 2–5** (parent, grab held): the definitive map in §7.1.
  Key 1 is a GPIO key on P1.5, which is why the first pass missed it; the knob
  switches sit on TM1650 DIG4 and also pull one encoder line low.
* **Fork built and tested** (§7): firmware 2.0.0, host suites `fw_logic` and
  `fw_store`.
* **14:32 2.0.0 flashed and verified** by the parent (flash-and-verify rc 0):
  * all 24 inputs in order;
  * per knob, 3 ccw, 3 cw and 2 presses exact, with no stray rotation from
    presses;
  * key 1 and key 15 together both register.
  * **Defect:** a fast revolution gave 10–11 of about 20 detents (slow: 19).
* **2.0.1** fixes it (§7.2): interrupt-sampled encoders, a full-cycle
  decoder, non-blocking ordered tap output, 1 ms endpoints, a raw `count`
  byte, and `CMD_GET_STATS`.
* **15:05 2.0.1 flashed and verified** by the parent:
  * `ENTER_BOOTLOADER=1`: `CMD_BOOTLOADER` was answered, so no key hold was
    needed;
  * Verify OK, `GET_INFO` 2.0.1, and the keymap survived.
  * Top knob, one revolution each: slow cw 20, fast cw 20, fast ccw 20.
  * `stats`: cw 40, ccw 20, missed states 0, overruns 0, drops 0, deepest
    queue 1.
  * The release JSON records `verifiedOnHardware: true`.

## 7. The fork as built: control-surface firmware 2.0.0

Source: `firmware/` (CC BY-SA 3.0, see `firmware/README.md`). All input
decoding, dispatch, raw mode, layers, keymap storage and the host protocol are
plain C in `firmware/src/padlogic.c` and `firmware/src/padstore.c`; SDCC builds
them for the CH552 and the host test suite builds the very same files.
`padfw.c` and `padcfg.c` only connect them to the pins, the TM1650 and USB.

### 7.1 Measured map (discovery captures 2–5, knobs on the right)

| Row | Left → right | Slots |
|---|---|---|
| 1 | GPIO P1.5, TM `44`, `4C`, `54`, `5C` | 0–4 |
| 2 | TM `64`, `45`, `4D`, `55`, `5D` | 5–9 |
| 3 | TM `65`, `46`, `4E`, `56`, `5E` | 10–14 |

| Knob | Press (TM) | Encoder A / B | Press also pulls | Slots ccw / press / cw |
|---|---|---|---|---|
| top | `67` | P3.2 / P1.4 | P1.4 low | 15 / 16 / 17 |
| middle | `5F` | P1.7 / P1.6 | P1.6 low | 18 / 19 / 20 |
| bottom | `57` | P3.0 / P1.1 | not observed | 21 / 22 / 23 |

TM1650 bus SDA = P3.3, SCL = P3.4. P3.1 is unused. Clockwise is A falling
first. The slot order is the daemon's keys-then-knobs order (`scheme.cpp`).

### 7.2 Input decoding (2.0.1)

* **Encoders** are sampled by a 4 kHz Timer2 interrupt, independent of the
  main loop, so USB reports, TM1650 reads and keymap typing never cost a
  detent.
  * Decoding uses a transition table (Buxton style): each valid one-line
    transition counts ±1, and a detent is emitted every 4 counts in one
    direction.
  * On return to rest, a remaining |count| ≥ 2 also counts (one skipped state).
  * A two-line jump (a missed state) counts as ±2 in the current direction
    mid-rotation, and as nothing at rest.
  * Detents accumulate per knob and direction, up to 100 each, until the main
    loop takes them.
  * A knob press pulls B low and back (−1, +1), which nets to zero. While the
    TM1650 reports that knob's switch, the count is held at zero: with B held
    low a turn carries no direction.
* **Keys:** a value must repeat on 3 main-loop passes in a row (one pass per
  millisecond) to count. The TM1650 reports one key at a time, so two TM1650
  keys cannot be held together (rolling from one to another releases the
  first). Key 1 is a GPIO and works together with any TM1650 key.
* Inputs held at power-on are treated as the resting state and send nothing
  until released and pressed again.
* **Output never blocks the input:**
  * The HID endpoints poll at 1 ms (`bInterval` 1; 2.0.0 used 10 ms).
  * Keymap detents are typed from an ordered queue of runs (16 runs; up to 200
    detents per run): press, 5 ms, release, at most one step per pass. Every
    detent is typed in the order turned, a quick back-and-forth included.
  * In raw mode one report carries all detents taken at once (`count` byte),
    so a slow host never loses any.
* **2.0.0 defect (fixed in 2.0.1):** a fast full revolution gave 10–11 of about
  20 detents.
  * The encoders were sampled in the main loop.
  * Each detent blocked that loop for about 55 ms: four HID reports at the
    10 ms poll interval, plus a 15 ms tap delay.
  * The decoder also only counted on reaching rest.
* **Diagnostics:** `CMD_GET_STATS` (`padctl.py stats`, and the `stats` field in
  `control-surfaced firmware-info --json`) reports:
  * decoded detents per knob and direction;
  * missed-state transitions;
  * accumulator overruns, tap-queue drops, and the deepest queue.
* **Timing budget:** the interrupt is about 40 instructions plus about 170 for
  the decoder, a few percent of the CPU at 4 kHz. Data-flash writes mask
  interrupts briefly, only while the keymap is being changed.

### 7.3 Modes

* **Keymap mode** (default): each input sends its action from the active
  layer. Keys press and release with the physical key; knob detents are taps
  (press, 5 ms, release, queued in order); wheel, pan and move actions are
  one-shot. A held key
  always releases what it pressed, even if the layer changed meanwhile. A
  modifier that another held chord still needs stays down.
* **Raw mode**: `CMD_RAW_MODE` with a timeout of 1–10000 ms. The pad then sends
  only report 5 events `[seq][slot][event][layer][count]` (1 = down, 2 = up,
  3 = tap; `count` = detents in a tap, 2.0.1+)
  and no keyboard, consumer or mouse output. Repeating the command is the
  heartbeat. Entering raw mode releases everything the keymap held. On
  timeout or `CMD_RAW_MODE 0` it returns to the keymap; a key held across the
  switch stays silent until pressed again (never a raw UP without its DOWN, no
  stray keymap release). The timeout runs on a 1 ms Timer2 tick.
* **Layers**: two, stored in data flash. Layer 0 is the daemon's scheme, so the
  current daemon works unchanged. Layer 1 is standalone: keys F13–F24,
  Play/Pause, Previous, Next; top knob Vol−/Mute/Vol+; middle knob wheel
  −1 / middle click / wheel +1; bottom knob Left / **layer toggle** / Right.
  Actions of type LAYER: toggle, momentary (held key, e.g. key 1 + knob
  turns), set. The power-on layer is stored (`CMD_SET_LAYER` with persist).

Layer 0 as the host sees it (Linux evdev codes in brackets):

| Slots | Inputs | Chord |
|---|---|---|
| 0–5 | keys 1–6 | F14–F19 (184–189) |
| 6–11 | keys 7–12 | LeftShift (42) + F14–F19 |
| 12–17 | keys 13–15, top knob ccw / press / cw | LeftCtrl (29) + F14–F19 |
| 18–23 | middle knob ccw / press / cw, bottom knob ccw / press / cw | LeftAlt (56) + F14–F19 |

### 7.4 Protocol v3 (report ID 3, 15 bytes each way; vendor page 0xFF00)

| Cmd | Request bytes 2… | Reply |
|---|---|---|
| `01` GET_INFO | – | `C S 3 24 128 status 2 0 0 layers active raw start` |
| `02` GET_ACTION | slot, …, layer @7 | type, mod, code lo, code hi, status, layer |
| `03` SET_ACTION | slot, type, mod, code lo, code hi, layer | status (1 ok, 2 index, 3 action, 4 write) |
| `04` RESET | – | defaults for both layers, start layer 0 |
| `05` BOOTLOADER | `'B' 'L'` | status, then jump to the ROM bootloader |
| `06` DUMP | offset | 12 data-flash bytes @3, status @15 |
| `07` CORRUPT | – | flips the stored CRC (recovery test) |
| `08` RAW_MODE | timeout lo, hi (ms) | status, raw active |
| `09` SET_LAYER | layer, persist 0/1 | status, layer |
| `0A` GET_STATS (2.0.1+) | page 0/1, clear 0/1 | six u16 in bytes 3–6 and 8–15; page 0: missed states per knob, overruns, queue drops, deepest queue; page 1: cw, ccw per knob |
| other | – | status 6 (unknown) |

Action types: 0 none, 1 key (HID usage ≤ 0xE7 + modifier mask, one hand only),
2 consumer (usage ≤ 0x3FF), 3 mouse (subtype button/wheel/pan/x/y << 8 | value),
4 layer (op toggle/momentary/set << 8 | layer). Host tool:
`firmware/padctl.py` (`info`, `get`, `dump`, `watch` are safe; `set`,
`layer`, `reset`, `bootloader` need `--yes`).

**Storage**: data flash `C S 3 24 crc start` + 2 × 24 × 2 bytes (102 of 128).
A boot reads only (no wear). The CRC-8 and a per-action validity check catch
damage; anything invalid (including EpicLPer's v2 layout or stock leftovers)
is replaced by the defaults. Full saves invalidate the magic first and write
the header last.

### 7.5 Build and tests

```sh
PATH=/mnt/ai/keypad-lab/tools/sdcc-4.5.0/bin:$PATH \
  python3 firmware/build.py --out /mnt/ai/keypad-lab/fw/build/control-surface padfw.c
ctest --test-dir /mnt/ai/keypad-lab/build/control-surface -R fw_
```

| Image | Size | SHA-256 |
|---|---|---|
| `padfw.bin` 2.0.0 | 10744 B (10740 of 14336 used; XRAM 331/768; stack 133 B free) | `af866d807e9c95f11483fe937d225b0645f75b50dda8ee0cd5f45fb7f52343e9` |
| `padfw.bin` 2.0.1 | 11328 B (11327 of 14336 used; XRAM 421/768; stack 130 B free) | `4c6f6f70315d71f8561c68e3c0b8e72fb0f1a6fb370ff8122e2cd05e5af0a9f2` |

Two independent builds produce the same hash.

* 2.0.1: `fw_logic` (about 6.5 million checks) runs a timed simulator: the
  encoder interrupt every 250 µs, the main loop every 1 ms, and each USB report
  blocking the loop like the endpoint does.
  * Fast spins of 20 detents, from 3.75 ms down to 0.5 ms per transition,
    with bounce and jitter, on every knob, in keymap and raw mode, at report
    paces of 0, 1 and 10 ms: exact counts every time.
  * A quick back-and-forth comes out complete and in order.
  * A knob press during a fast spin, and all three knobs spinning at once.
  * Missed rest states, the accumulator and queue limits, and the soak at
    both report paces.
  * Mutations: sampling in the main loop as 2.0.0 did fails 184 checks;
    emitting only at rest, as 2.0.0 did, fails 3.
* 2.0.0: `fw_logic` (about 316,000 checks): the measured key map, every code in
  0x00–0xFF, the action codec over all 65,536 stored words, CRC vectors, the
  default keymaps (layer 0 = daemon scheme), encoder sequences (clean, bounce,
  press-without-turn on each knob, press with A chatter, turns while held,
  missed states, reversals, long chatter, mid-detent power-on), debounce,
  keys (GPIO + TM1650 together, roll-over, bounce, held at power-on), knobs,
  layers (toggle, momentary, held across a change), raw mode (heartbeat,
  expiry, explicit off, re-entry with a key held, sequence wrap, layer field),
  and a seeded soak of 80,000 random physical operations with bounce in
  keymap and raw mode that must reproduce exactly the presses and detents
  performed and leave nothing held.
* `fw_store` (about 638,000 checks): first boot on erased and random flash,
  read-only normal boot, every protocol command and refusal, write failure,
  every single-bit flip in header and actions, CRC-valid nonsense, power cut
  after every write of a reset, a single change and the first boot.
* Mutation checks: disabling the knob-hold freeze, the debounce, the raw-entry
  release, the expiry clean-up, the validity check or the start-layer check,
  or moving the keymap, each makes a suite fail.

### 7.5a Releases and host commands

* `firmware/make-release.sh` builds twice, requires identical images, and
  writes `firmware/release/<name>.bin` and `<name>.json` (name, version, board,
  licence, SHA-256, size, source commit, toolchain, `verifiedOnHardware`) next
  to the CC BY-SA 3.0 `LICENSE`. smplOS installs them to
  `/usr/share/control-surface/firmware/`; `scripts/install-user.sh` copies them
  to `~/.local/share/control-surface/firmware/`. Both are the settings API's
  default image directories.
* `control-surfaced firmware-info [--json]`: `GET_INFO` (version, slots,
  layers, active and start layer, raw mode) from a pad running this firmware.
* `control-surfaced enter-bootloader --yes`: `CMD_BOOTLOADER`; the pad then
  shows as 4348:55e0 until it is flashed or replugged. The settings API's flash
  job does the same by itself when the pad runs this firmware, so the wizard
  only needs the top-left key for other firmware (or if the request fails).
* Both talk only to a hidraw node whose report descriptor has reports 3 and 5;
  stock or other firmware is never written to.

### 7.6 Recovery

* **Top-left key held while plugging in**: the CH552 ROM's own P1.5 check,
  independent of any firmware. Release the key once the pad shows as
  4348:55e0, or the freshly flashed firmware will drop straight back into the
  bootloader on its first boot.
* Any other key or knob switch held at plug-in: this firmware jumps to the
  bootloader itself (as discovery does).
* `padctl.py bootloader --yes` (`CMD_BOOTLOADER` with its guard).
* Fallback images: `discovery.bin` (diagnostic) can be rebuilt from this tree;
  the original `6ece5ffe…` build is in `/mnt/ai/keypad-lab/fw/build/openmacropad/firmware/out/`.
* The config registers are never written (DOWNLOAD_CFG stays P1.5).

### 7.7 One-step flash and verify (needs the user)

```sh
firmware/flash-and-verify.sh <image> <sha256> <logdir> [version]
# pad already on 2.0.x: no key hold needed
ENTER_BOOTLOADER=1 firmware/flash-and-verify.sh firmware/release/control-surface-sy181-15k3e-2.0.1.bin \
    4c6f6f70315d71f8561c68e3c0b8e72fb0f1a6fb370ff8122e2cd05e5af0a9f2 <logdir> 2.0.1
```

1. The parent starts its instant-grab capture first, then the script.
2. **With `ENTER_BOOTLOADER=1`** the script asks the running firmware for the
   bootloader (`padctl.py bootloader --yes`) *after* noting any existing
   bootloader session, so the new one counts. Without it, or if the request is
   not answered, the user unplugs the pad, holds the **top-left** key, plugs it
   in, and lets go after about one second.
3. The script then:
   * checks the image hash;
   * waits for a fresh 4348:55e0 session and runs `wchisp flash` once (erase,
     write, verify, reset; no config);
   * waits for 1189:8890 serial key153 (manufacturer `OpenMacroPad`, product
     `Control Surface 15+3`, bcdDevice 2.00);
   * runs the read-only checks: `GET_INFO` (`CS`, format 3, 24 slots,
     2 layers, the expected version, raw off), all 48 actions, and the
     data-flash dump.
   * With 2.0.1, it also zeroes the encoder counters (`padctl.py stats
     --clear`), so a measurement starts from zero.
4. Exit code 0 means all passed. The keymap in data flash survives the
   reflash.

### 7.8 Verification after the flash (user present, parent grabbing)

1. **Press capture**, 3 rounds: keys 1–15 in row order, then per knob top to
   bottom: press, 3 cw, 3 ccw. Expected per round: 15 + 3 presses, each one
   press/release of the chord in §7.3, 9 cw and 9 ccw taps, and **no** turn
   events during the presses.
2. **Raw mode**: `firmware/padctl.py watch --seconds 60` while the user repeats
   one round. Expected: raw events with gap-free sequence numbers and **zero**
   evdev events in the parent capture.
3. **Heartbeat loss**: `kill -9` the watch process while a key is held, release
   it, wait 2 s, press a key: the keymap chord must come back, with nothing
   stuck.
4. **Persistence** (writes, each needs approval): `padctl.py set 0 1 0 0x68
   --layer 0 --yes` (key 1 → F13), replug, check key 1 = F13, then
   `padctl.py reset --yes` and check key 1 = F14 again.
5. **Layer 1**: `padctl.py layer 1 --yes`; check volume on the top knob and the
   bottom-knob press toggling back to layer 0.

### 7.8a Re-measuring the knobs on 2.0.1

1. `firmware/padctl.py stats --clear`.
2. One slow and one fast full revolution per direction on the top knob.
3. `firmware/padctl.py stats` must then show about 20 detents for each
   revolution, with `missed-state`, overruns and drops at 0. The parent's
   capture must show the same number of chords.
4. A non-zero `missed-state` means transitions shorter than 250 µs. That calls
   for a faster timer, not a different decoder.

### 7.9 Soak plan (with the user)

* 10 minutes of fast knob spins in both directions, knob presses during and
  between spins, and key mashing, under `padctl.py watch`: no sequence gaps,
  press slots only for presses, no turn events while a knob is held.
* A long session with the daemon in raw mode (once its raw backend lands):
  no USB resets in `journalctl -k`, no stuck keys after the daemon is stopped.
