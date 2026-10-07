# CH552 macro pad (USB 1189:8890, serial key153): programming record

> **Current state (2026-10-07 10:12, working).** On this unit the write format
> is **key ID first**. Each key gets one raw 64-byte hidraw write to interface 1,
> `[keyId][mods][00][usage][00 00 00 00 00]` plus zeros. Byte 0 is the key ID;
> the next 8 bytes are stored verbatim as that key's boot-keyboard report. It
> applies at once and survives a replug. There is no open, close or save frame.
> Key IDs 1–24 now hold the F14–F19 × {none, LShift, LCtrl, LAlt} scheme
> (`ch552-padprog flash`, default `--dialect keyid`). See "Key-ID-first format"
> at the end of this file.
>
> The vendor app's frames (`[slot][type][n][i][mods][code]` + `AA AA`) and the
> 0x03-marked blob03 frames had **no effect** on this unit. The sections in
> between are kept as the investigation record.

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

## Silent after programming (resolved: wrong frame format, see the end)

Before programming, every key and knob sent an all-zero HID report on hidraw4,
hidraw6 and hidraw7, and no evdev keys. After the flash, the coordinator saw
**no reports at all**. I diagnosed this passively from sysfs and `/proc`, with
no device I/O:

* The frames are byte-compatible with the vendor app's report-ID-0
  `Download_Click` (`FormMain.cs:498–560`). They contain no mode, LED, layer or
  bootloader opcodes.
* The flash differs from the vendor app in two ways:
  * it wrote 24 slots, while the vendor app writes slots 1–18;
  * it sent 24 `AA AA` commits in quick succession.
* The pad re-enumerated at 08:29 as USB device 21. `verify-pad.sh` held its
  event nodes at the time.

Recovery steps, in order. Stop at the first one that brings reports back:

1. **Passive capture.** Stop `verify-pad.sh` and the service. Then, as you,
   with no writes:

   ```sh
   timeout 20 od -An -tx1 -w64 /dev/hidraw4 & timeout 20 od -An -tx1 -w64 /dev/hidraw6 & timeout 20 od -An -tx1 -w64 /dev/hidraw7 & wait
   ```

   Press a few keys and knobs while it runs. Unplug and replug the pad once,
   too.
2. **Minimal reflash** of slot 1 only, with a long pause after the commit:

   ```sh
   ch552-padprog flash --slots 1-1 --settle-ms 1500                 # dry run: 4 frames
   ch552-padprog flash --slots 1-1 --settle-ms 1500 --yes --log /tmp/pad-slot1.json
   ```

   Key 1 should then type F14. If it does, flash the rest in vendor-sized
   steps (`--slots 2-18`, then `--slots 19-24`).
3. **Blank**: `ch552-padprog blank --yes`. Add `--slots` to limit it.
4. **The vendor Windows app.** It writes slots 1–18 with its own timing.

`--slots N` or `--slots A-B` (within 1–24) limits a `plan`, `flash` or `blank`
to those slots. `--settle-ms` (0–10000, default 120) sets the pause after each
commit frame. Every frame still passes the same allow-list: ping, binding,
empty-key and commit frames only. No firmware or bootloader mode is ever used.

## Step 2 result and the blob03 proposal (did not work)

On 2026-10-07 at 09:36:57 I ran `ch552-padprog flash --slots 1-1 --settle-ms 1500 --yes`. It sent 4 vendor-format frames (65-byte hidraw writes), with no write errors and no device replies. The log is `docs/records/flash-20261007-slot1-step2.json`. After a replug, the coordinator's capture of key 1 showed 30 reports, all zero. Neither flash changed anything.

### How the tools write to this pad

Facts about this pad, read from sysfs and the descriptors only:
* It is low-speed (1.5 Mb/s) with bcdDevice 1.00.
* Interface 1 has an interrupt OUT endpoint 0x02 and an interrupt IN endpoint 0x82, both with wMaxPacketSize 8.
* Its report descriptor declares usage page 1, usage 0, an 8-byte input, a 64-byte output, and **no report IDs**.
* In Linux 6.18, hidraw `write()` goes to the interrupt OUT endpoint. The kernel drops byte 0 only when it is 0x00.

| Tool | Transport | Report ID / first wire byte | Frame | Result on this family |
|---|---|---|---|---|
| Vendor app (`HidLib.cs`, WriteMode 1) | HidLibrary WriteFile on `mi_01`, interrupt OUT | Probes 3 → 0 → 2. Windows rejects a non-zero ID on a collection without report IDs, so it uses 0 and the first byte is the slot. | 64 data bytes, only 8 meaningful: `[slot][type][n][i][mods][code]`, then `AA AA` | #168: "says uploaded, nothing changes". |
| This tool, flashes 1 and 2 | hidraw write, interrupt OUT | 0 (stripped), so the first byte is the slot | Same as the vendor app | Nothing changed. |
| rOzzy1987/MacroPad (lists 8890 as `Legacy`, `mi_01`) | HidLibrary, as the vendor app | Probes 0 → 2 → 3 | As the vendor app | Not reported for this revision. |
| cho45 WebHID (8890, 3 keys + 1 knob) | `sendReport`, interrupt OUT | Probes 3, 0, 2 | **8-byte** reports, vendor layout | Its own device. |
| kriomant ch57x (`k8890`) | libusb `write_interrupt` on EP 0x02 | **0x03** in the data | `[03][keyId][(layer<<4)\|1][n][i][mods][code]` | #168, same descriptor family: **changed the pad** (modifier garbage). |
| barkleesanders/padclaude (`padflash.swift`) | IOHIDDeviceSetReport, report ID 3 | **0x03** | `[03 A1 01]`, then `[03][keyId][mods][00][usage][0 0 0 0]` per key, then `[03 AA AA]`, 3 ms apart | **Working.** Raw capture shows F13–F18 on keys 1–6 of a low-speed 8890 with the same 64-byte out / 8-byte EP config interface. |

Three conclusions follow:
* The only writes that ever changed an 8890 of this descriptor family put **0x03 first on the wire**, while the descriptor declares no IDs. Writes with the slot as the first byte (vendor app, MacroPad's ID-0 path, this tool) did nothing.
* padclaude measured how this revision stores data: the 8 bytes after `[03][keyId]` are kept **verbatim** as that key's boot-keyboard report. That also explains the blank state: empty records replay as all-zero reports.
* No tool reads a reply, waits for an ack, or sends a version query, reboot or save beyond `AA AA`. `AA A1` saves LED settings and `A1 nn` selects a layer.

### The blob03 method (`--dialect blob03`; tried, no effect)

* Write raw 64-byte frames to the interface-1 hidraw node. There is no leading report-number byte: byte 0 is 0x03, so the kernel sends all 64 bytes on EP 0x02.
* Use one session: `03 A1 01`, then `03 01 00 00 69 00 00 00 00` (key ID 1 → F14, no modifiers), then `03 AA AA`, 3 ms apart, with 1500 ms after the close.
* The frames are padclaude's measured wire bytes, byte for byte.
* A separate allow-list (`blob::isAllowedFrame`) admits only these three frame shapes, with key IDs 1–24, a usage ≤ 0x91 and zero tails. It refuses `FE`, `EF`, `5A`, `FC`, `B0`, `AA A1`, and frames without the marker.

```sh
ch552-padprog flash --dialect blob03 --slots 1-1 --settle-ms 1500                    # dry run (3 frames)
ch552-padprog flash --dialect blob03 --slots 1-1 --settle-ms 1500 --yes --log FILE   # needs approval
```

Test it first **without** a replug, to see whether it takes effect live, then again after a replug, to see whether it persisted. Grab event18–21 during the test (for example `control-surfaced verify --no-write`) so that a mis-stored modifier cannot reach the desktop.

If key 1 still sends zeros, the next candidate is the same frames over the control endpoint: SET_REPORT through usbfs, after detaching usbhid from interface 1 only. padclaude claims this is required, but its macOS path most likely used the interrupt pipe as well. That step needs its own approval.

### Blob03 result and a re-reading (2026-10-07 09:50)

After the blob03 slot-1 run (`docs/records` holds the log), key 1 still sent all zeros, both before and after a replug. Only key 1 was captured.

**Layers.** The vendor app's UI has Layer1–3, defaulting to 1. With report ID 0, which is our descriptor, the app never encodes a layer: there is no `A1 nn` and no layer nibble. In every other tool, `A1 nn` exists only in the report-ID-3 dialect. The Rockheung spec documents it as "switch active layer". On pads that have layers, a side button switches them and LEDs show the layer briefly (kriomant's `example-mapping.yaml`). No source describes a programming mode, a pinhole, or a key combination held at plug-in.

**Re-reading padclaude.** padclaude made three measurements:
* a with-prefix frame `[03 AA AA]` became key 3 with modifiers 0xAA;
* legacy frames replayed as `11 01 01 00 code`;
* the final `[keyId mods 00 usage]` buffer replayed as `00 00 68`.

A simpler model fits all three: **wire byte 0 is the key ID, and bytes 1–8 are stored verbatim as that key's report**. IOKit added nothing to the wire.

Issue #168 fits the same model. kriomant's 0x03-led frames ended with `[03 AA AA]`, and the reporter saw "key 3 as a rapid Shift + Win key": 0xAA is LShift + LGui + RShift + RGui.

If this model is right, our blob03 frames all addressed **key ID 3**, not key 1.

## Key-ID-first format (confirmed, 2026-10-07)

1. **10:05:11.** A single approved frame `01 00 00 69 00 00 00 00` plus zeros
   (`docs/records/flash-20261007-keyid-slot1.json`).
   * Before any replug, one key sent the hidraw4 report `00 00 69 00 …` and
     evdev KEY_F14 (184) press and release.
   * After a replug, the **top-left key** still sent F14, and the other top-row
     keys still sent zeros.
   * The coordinator's capture logs are `keyid-first-raw.log` and
     `keyid-first-replug-raw.log`.
2. **10:12:53–59.** `ch552-padprog flash --dialect keyid --slots 1-24 --settle-ms 150 --yes`
   wrote key IDs 1–24, 150 ms apart. Each write returned 64 bytes, with no
   failures and no device replies (`docs/records/flash-20261007-keyid-1-24.json`).

The scheme written is ID n → modifier of group (n−1)/6 (none, LShift 0x02, LCtrl 0x01, LAlt 0x04), reserved 0, usage 0x69 + (n−1) % 6, which is F14–F19.

Why the earlier writes failed:
* **Vendor frames.** Byte 0 was the slot, so the frames were addressed
  correctly. But byte 1 (type 1) landed in the modifier slot, byte 3 (index)
  landed in the key slot, and so on. The pad never showed even that garbage
  after the vendor flashes. The likely reason: the vendor sequence's 64-byte
  ping and its `AA AA` commit hit key IDs 0 and 0xAA, and something in that
  sequence prevented the record from being applied. The cause is not
  isolated; only the key-ID-first single frame is known to work.
* **blob03.** Its frames were addressed to key ID 3 (byte 0 = 0x03). They did
  not persist on key 3 either: an all-key capture after a replug was all zeros.

The hardware map (which physical control has which key ID) is learned by the
coordinator's 24-input capture, or by `scripts/verify-pad.sh`. The earlier
all-key capture produced only 14 report bursts for 24 inputs. Some inputs,
probably knob turns, may use key IDs above 24 or emit on another interface.
Speculative IDs above 24 are deliberately not written: padclaude reports that a
wrong ID corrupted its pad's table until a reflash.

### Restore

* **One key:** `ch552-padprog blank --slots N --yes` writes a zero record,
  `[N] 00 00 00 …`.
* **All 24 keys:** `ch552-padprog blank --yes`. It uses the same key-ID format
  and is not yet tried on the device.
* **Any other map:** `ch552-padprog flash --yes`.

The firmware itself was never touched, and no command beyond these records was
sent.
