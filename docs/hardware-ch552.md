# CH552 macro pad (USB 1189:8890, serial key153): programming record

## Device facts (read from sysfs, 2026-10-07)

* wch.cn "CH552", bcdDevice 1.00, **low-speed** USB 1.0, 4 HID interfaces:
  * IF0 boot keyboard (EP 0x81 IN). Keyboard usages 0x00–0x91, 8-byte reports → `event18`, `hidraw4`
  * IF1 **configuration channel**: EP 0x02 interrupt OUT and EP 0x82 IN, both
    `wMaxPacketSize` 8. Its report descriptor has **no report ID**, an 8-byte
    input and a **64-byte output** report → `event19`, `hidraw5`
  * IF2 mouse → `event20`, `hidraw6`
  * IF3 consumer bitmap (24 bits) + system-control byte → `event21`, `hidraw7`
* Raw descriptors and evdev capabilities from before the flash:
  `docs/records/preflash-20261007/`.
* Before programming, every key and knob sent an all-zero report (blank map;
  parent session's capture).

## Choosing the programmer (vetting)

| Tool | License | Verdict for this pad |
|---|---|---|
| kriomant/ch57x-keyboard-tool 1.7.0 (`bdbffca`), built and tested (43/43) in `/mnt/ai/keypad-lab` with `CARGO_HOME`/`CARGO_TARGET_DIR` there | MIT OR Apache-2.0 | **Not used.** It maps 1189:8890 to `k8890` (`ch57x-2`), which caps buttons at 12 (knob IDs 13–21). It prefixes `0x03` as data and sends a `0xfe` start frame (the report-ID-3 dialect). Upstream #168 shows the same descriptor family misbehaving with it. 15+3 pads are reported working only on 8840 (`k884x`). |
| barkleesanders/padclaude (`37842ea`) | MIT | macOS only. Its "blob" format was derived on a 6-key pad, and its own notes report corrupted maps while probing. Used for background only. |
| User's vendor app source, `~/Documents/source/tools/MINI KeyBoard` ("3 knob keyboard KEY_BOAED_EN_2022.6.15", decompiled C#) | vendor | **Ground truth for the frame format.** `KeyBoardVersion_Check` probes report IDs 3 → 0 → 2. With report ID 0 (our descriptor), `Download_Click` sends `[slot][type&0x0F][count][index][mods][code]` with no layer nibble and no `0xA1` layer select, then `[0xAA 0xAA]` (write flash). It writes through HidLibrary `WriteFile` (interrupt OUT). Its UI wires only keys 1–12 and knobs 1–2 (slots 13–18). |
| civilian7/hid-macro-keypad, jgt87/Macropad, GeorgeZhai/mini-keyboard-studio-macos, fleximus/macro_keyboard | MIT | Consistent with the vendor app: interrupt-OUT `WriteFile` works and `SetOutputReport` fails (civilian7). Index 0 is a header frame with code 0. A header-only frame (count 1, no step) **erases** a slot. Never send `0xEF`/`0x5A` (bootloader) or `0xFC` (variant). |

So this repository has its own small flasher, `ch552-padprog`
(`src/padprog`, `src/proto`). It writes exactly the vendor app's report-ID-0
frames through **hidraw `write()` on interface 1** (Linux's equivalent of
`WriteFile`: usbhid sends it on the interrupt-OUT endpoint, 8 packets of 8
bytes). Safety rails:

* Refuses any target except interface 1 of 1189:8890 with the requested serial.
* Refuses unless the interface's report descriptor has a 512-bit output report
  (determines report ID 0 vs 3).
* Re-checks every frame against an allow-list: slot 1–24 keyboard type, `AA AA`,
  `A1`, all-zero ping. Bootloader/variant/LED opcodes are rejected.
* Is a dry run unless `--yes` is given.

Never used: bootloader mode, firmware commands, LED commands.

## The code scheme written to the pad

24 distinct chords: **F14–F19 × {none, Shift, Ctrl, Alt}**. Slot *n* (1-based)
emits key F14 + (n−1) mod 6 with modifier group ⌊(n−1)/6⌋.

| Slots | Chords |
|---|---|
| 1–6 | F14 F15 F16 F17 F18 F19 |
| 7–12 | Shift+F14 … Shift+F19 |
| 13–18 | Ctrl+F14 … Ctrl+F19 |
| 19–24 | Alt+F14 … Alt+F19 |

Why this scheme is harmless when the daemon is not running:

* In the default xkb `inet(evdev)` map, F14–F18 produce `XF86Launch5..9` and
  F19 produces `F19`. All are single-level, so no modifier changes the keysym.
  Nothing in the user's Hyprland config binds them (checked
  `~/.config/hypr/*`), and no app types a character for them.
* Excluded on purpose:
  * **F13** = `XF86Tools`, which opens System Settings on Plasma.
  * **F20** = `XF86AudioMicMute`.
  * **F21–F23** = touchpad toggle/on/off and Assistant.
  * **F24**: Ctrl or Super gives `XF86TouchpadToggle`.
  * Media/consumer codes, Japanese/Korean IME keys and Ctrl+C-style chords.
* Modifiers are only Shift/Ctrl/Alt and travel in the same report as the key,
  so the daemon decodes each chord atomically on the grabbed device. Nothing
  leaks to the desktop.

Physical position ↔ slot is **not yet confirmed** (no one could press keys
during the flash). Two numberings are plausible:

* **A, keys-then-knobs** (daemon default): keys 1–15 = slots 1–15; knob *k*
  ccw/press/cw = 16+3(k−1)+{0,1,2}.
  So key13–15 = Ctrl+F14–F16; knob1 = Ctrl+F17 (ccw), Ctrl+F18 (press),
  Ctrl+F19 (cw); knob2 = Alt+F14/F15/F16; knob3 = Alt+F17/F18/F19.
* **B, vendor-twelve**: keys 1–12 = slots 1–12; knobs at 13–21; keys 13–15 at
  22–24.

Either way, all 24 inputs emit distinct chords. `scripts/verify-pad.sh`
records the real mapping and writes it for the daemon, whichever numbering the
firmware uses.

## What was written (2026-10-07 01:07 PDT)

Command:
`ch552-padprog flash --serial key153 --yes --log …/flash-20261007-scheme-f14f19x4.json`

* Target `/dev/hidraw5`, interface 1, serial `key153`, report ID 0.
* 73 frames:
  * 1 ping (`00 00 …`, as the vendor app sends on connect)
  * 24 × [header, step, `AA AA` commit]. Example for slot 7:
    `07 01 01 00 02 00`, then `07 01 01 01 02 69`, then `aa aa`. Each frame is
    zero-padded to 64 bytes and prefixed with report number 0 for hidraw.
* Result: **73/73 writes accepted (65 bytes each, no errors)**. Each 64-byte
  frame took ~57 ms on the wire, which is 8 low-speed packets at the 10 ms
  interval with no NAK/timeout. No kernel errors, no reset or disconnect; the
  device stayed enumerated with all 4 evdev nodes.
* The device sent **no input reports on IF1** during or after the writes. Other
  report-ID-0 projects report the same, so there is no read-back or ack to
  verify against.
* Full log with per-frame timing: `docs/records/flash-20261007-scheme-f14f19x4.json`
  (SHA256 `cff16507…6fe2`) and `.txt`.

What could be verified without pressing keys:

1. The USB-level acceptance and timing above.
2. The pad still enumerates identically.
3. The daemon grabs and releases all four nodes (`CS_TEST_REAL_PAD=1 tst_paddevice`).

That flashed slots produce the intended chords can **only** be confirmed by
pressing the controls.

## Verification when the user is back (one command)

```sh
~/Documents/source/control-surface/scripts/verify-pad.sh
```

It stops the service if it is running, grabs the pad, and asks for each of the
24 inputs in turn: key row/column, then each knob ccw, cw and press. It prints
the chord each one emits and reports the numbering (A, B or custom),
duplicates, chords not from the scheme, and whether the pad holds keys or only
sends taps. It writes `~/.config/control-surface/hardware-map.json` (backing up
any old file) and saves a transcript to `~/.local/state/control-surface/`.

If the result is:

* **numbering A or B, 24/24:** done. Start the service:
  `systemctl --user enable --now control-surface.service`.
* **custom but 24/24 distinct:** still fine. The written map is used as is.
* **nothing at all / still blank:** the firmware ignored the frames. Re-run
  `ch552-padprog flash --yes`, then try the vendor app or open an issue. Do not
  experiment with other opcodes.
* **some controls produce the same chord or a non-scheme code:** slots exceed
  the firmware's table or the numbering is unusual. Send the transcript; a
  flash with a different slot range is a one-line change in `src/proto/scheme.cpp`.

## Restoring a blank map

```sh
ch552-padprog blank --serial key153          # dry run: shows the 49 frames
ch552-padprog blank --serial key153 --yes    # writes them
```

This sends the ping, then for every slot 1–24 a header-only "empty key" frame
(`NN 01 01 00 00 00`) and `AA AA`. That is the erase form civilian7 verified on
a report-ID-3 pad. On this report-ID-0 pad it is **untested**. The alternative
is to reflash any known map (`ch552-padprog flash --yes`) or use the vendor app.
The pad's firmware itself was never touched.
