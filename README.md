# control-surface

A small native daemon (C++20, Qt 6 Core/Network/DBus, libudev; no Python) that
turns the CH552 macro pad (USB `1189:8890`: 15 keys, 3 knobs that turn and
press) into a per-application control surface on Hyprland.

* Grabs **only** the pad: matched by VID:PID and serial, checked again with
  `EVIOCGID` before `EVIOCGRAB`. Re-attaches after unplug/replug via udev. Your
  keyboard and mouse are never opened.
* Follows the focused window through Hyprland's IPC sockets
  (`.socket2.sock` events, `j/activewindow` for the PID). The backend is
  pluggable; KWin is planned.
* Per-app **profiles**. For plain apps it types keys through one uinput
  keyboard. Knob taps are paced, and reversing a knob cancels queued motion.
* **Kdenlive** is context-aware through the K23 interface
  `org.kde.kdenlive.ControlSurface1`, wire revision 2
  (`docs/kdenlive-api-contract.md`; MR1–MR3 qualified and accepted end to end,
  `docs/E2E-ACCEPTANCE.md`). It uses curated actions, continuous controls
  (MR1: jog/shuttle/zoom; MR2: parameter and colour-wheel nudges; MR3: track,
  scroll, gain, resize trim) and per-context layers. These cover the timeline,
  the clip and project monitors, the effect parameter, the Lift/Gamma/Gain
  wheels, and the track/mixer and trim pages. For example, when a
  Lift/Gamma/Gain widget is focused, Kdenlive publishes one handle per wheel
  (`colorWheels`): the three knobs become the three wheels without moving
  focus, and a knob press cycles value/R/G/B.
  * Only what Kdenlive advertises is used.
  * Knob motion is coalesced per gesture and target. Acks are matched on
    (session, seq). Kdenlive keeps one editing gesture at a time, so the
    daemon ends one knob's gesture (one undo entry) before another starts.
  * **API only.** The interface is off by default in Kdenlive. While it is
    absent, the pad does nothing in Kdenlive and the daemon shows one notice.
    Stock shortcuts are typed only with `"keyFallback": true` on the
    profile. Refusals and timeouts never type keys.
* Validated configuration with errors that name the place, hot reload on
  save, press + turn ("shift") bindings, and per-binding acceleration.
* Dry-run and simulate modes, a mock Kdenlive (`--stage 1|2|3`, `--off`,
  `--tick-ms`), and
  11 test suites, including one on a private D-Bus session bus.

## Quick start

`USER-QUICKSTART.md` is the user guide: it covers verifying the pad, enabling
Kdenlive's interface, the control map and editing the config.

```sh
scripts/install-user.sh          # build (out of tree), test, install to ~/.local; unit installed but NOT enabled
scripts/verify-pad.sh            # press each control when asked; writes ~/.config/control-surface/hardware-map.json
control-surfaced run --dry-run   # watch what each control would do, without sending anything
systemctl --user enable --now control-surface.service
```

Configuration: `~/.config/control-surface/config.jsonc` (installed from
`data/config.example.jsonc`: the per-app profiles first, advanced settings at
the end). It is reloaded on save. Simple options never need an editor:
`control-surfaced get` lists them, `control-surfaced set input raw` changes
one (validated, with a backup, comments kept). Check the file with
`control-surfaced check-config`, and compare it with what Kdenlive offers with
`control-surfaced list-capabilities`. Every option and binding form:
[docs/config-reference.md](docs/config-reference.md).

## Commands

| Command | Purpose |
|---|---|
| `control-surfaced run [--dry-run] [--quiet]` | the daemon |
| `control-surfaced verify [--no-write]` | interactive pad check / hardware map learning |
| `control-surfaced simulate FILE\|-` | run scripted events (`window org.kde.kdenlive`, `key3`, `knob1 +5`, `knob2 press`/`hold`/`release`, `context {…}`, `kdenlive off`, `expect notice`, `expect no-keys`, …) and print the resulting actions; `--kdenlive-service NAME` drives a real Kdenlive with a non-emitting key sink |
| `control-surfaced list-capabilities [--json] [--kdenlive-service NAME]` | what the running Kdenlive offers (controls, commands, actions, limits, context paths for `when`) and which configured bindings it does not offer; read-only, no lease |
| `control-surfaced set KEY VALUE` / `get [KEY]` `[-c FILE] [--json]` | change or show a simple option (`input`, `serial`, `cheatsheet.*`, `settings.*`) in place: validated, backed up, written atomically; waits for a running daemon to apply it |
| `control-surfaced status [--json]` | daemon, pad, firmware, effective layout and config (offline when the daemon is not running) |
| `control-surfaced monitor [--json] [--identify]` | live input, one line per press/release/detent; follows daemon restarts; `--identify` reports without dispatching |
| `control-surfaced list-actions [--json]` / `features [--json]` | offline: the curated Kdenlive actions, controls and commands; binding kinds and key/mouse names |
| `scripts/stress-test.py [--mode evdev\|raw\|both]` | with a person at the pad: exact press/release pairs, holds of 0.1–5 s, two keys at once, every knob detent against the firmware's counts, no raw-mode flips |
| `control-surfaced cheatsheet [--json] [--follow] [--window CLASS]` | what every input does now (the overlay's content); `--follow` for debugging, `--window` for an offline preview |
| `control-surfaced run --eww-window NAME --eww-config DIR` | also push the cheatsheet into eww (`eww update pad_sheet=…`, `open`/`close` the window); `--eww` for the variable only; the config's `cheatsheet.eww` overrides (docs/dbus-settings-api.md) |
| `control-surfaced firmware-info [--json]` / `enter-bootloader --yes` | protocol v3 on a pad running the control-surface firmware |
| `control-surfaced list-devices` / `check-config [--json]` / `example-config` | diagnostics (`check-config --json` names the failing profile, layer and slot) |
| `mock-control-surfaced` | the settings API (`org.smplos.ControlSurface1`) with a simulated pad, for UI work |
| `control-surfaced bench-dbus N --kdenlive-service NAME` | contract round-trip latency |
| `mock-kdenlive [--apply-delay MS] [--tick-ms MS]` | reference implementation of the Kdenlive contract with a console (`wheel gamma`, `hover lift`, `param level`, `grouped on`, …) |
| `ch552-padprog list\|plan\|flash\|blank [--slots A-B] [--settle-ms N] [--dialect keyid\|vendor\|blob03] [--yes]` | program the pad: default `keyid`, one `[keyId][8-byte report]` record per key, effective at once and persistent (see `docs/hardware-ch552.md`) |

## Building and testing

```sh
cmake -S . -B /mnt/ai/keypad-lab/build/control-surface -G Ninja
cmake --build /mnt/ai/keypad-lab/build/control-surface
ctest --test-dir /mnt/ai/keypad-lab/build/control-surface --output-on-failure
CS_TEST_REAL_PAD=1 /mnt/ai/keypad-lab/build/control-surface/tst_paddevice   # grabs and releases the real pad
```

The uinput test grabs its own virtual keyboard and pointer before emitting, so
no key or click ever reaches the desktop. The settings, CLI and Kdenlive bus
tests run on a private D-Bus (`dbus-run-session`).

## Documents

* `docs/hardware-ch552.md`: protocol research, what was flashed, how to verify and restore.
* `USER-QUICKSTART.md`: the user guide.
* `docs/kdenlive-api-contract.md`: the editing inventory, the coverage
  matrix, a verbatim mirror of the qualified K23 revision-2 contract, and the
  daemon's client policy.
* `docs/E2E-ACCEPTANCE.md`: the real-editor acceptance against the qualified
  Kdenlive; the evidence is in `docs/records/k23-acceptance-20261007/`.
* `docs/design.md`: daemon architecture, latency and safety decisions.
* `docs/FIRMWARE-PLAN.md`: the open firmware for this pad (§7: map, protocol,
  tests, flash and verification); `firmware/README.md`: origin and licence.
* `docs/GENERALIZATION-PLAN.md`: the pad family, the settings API and plugins.
* `docs/dbus-settings-api.md`: the `org.smplos.ControlSurface1` reference.
