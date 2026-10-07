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
* **Kdenlive** is context-aware through the proposed
  `org.kde.kdenlive.ControlSurface1` D-Bus interface
  (`docs/kdenlive-api-contract.md`): actions by name, continuous controls (jog,
  shuttle, zoom, parameter and colour-wheel nudges, automation, trim, gain), and
  context layers. For example, when Lift/Gamma/Gain is focused, the three knobs
  become the three wheels and a knob press cycles luma/R/G/B. Knob motion is
  coalesced (one message in flight per control, acked by Kdenlive). Stock
  Kdenlive without the interface gets the configured fallback keys instead.
* Dry-run and simulate modes, a mock Kdenlive, and 10 test suites.

## Quick start

```sh
scripts/install-user.sh          # build (out of tree), test, install to ~/.local; unit installed but NOT enabled
scripts/verify-pad.sh            # press each control when asked; writes ~/.config/control-surface/hardware-map.json
control-surfaced run --dry-run   # watch what each control would do, without sending anything
systemctl --user enable --now control-surface.service
```

Configuration: `~/.config/control-surface/config.jsonc` (installed from
`data/config.example.jsonc`, which documents every binding form). Check it with
`control-surfaced check-config`.

## Commands

| Command | Purpose |
|---|---|
| `control-surfaced run [--dry-run] [--quiet]` | the daemon |
| `control-surfaced verify [--no-write]` | interactive pad check / hardware map learning |
| `control-surfaced simulate FILE\|-` | run scripted events (`window org.kde.kdenlive`, `key3`, `knob1 +5`, `knob2 press`, `context {…}`, `kdenlive off`) and print the resulting actions |
| `control-surfaced list-devices` / `check-config` / `example-config` | diagnostics |
| `control-surfaced bench-dbus N --kdenlive-service NAME` | contract round-trip latency |
| `mock-kdenlive [--apply-delay MS]` | reference implementation of the Kdenlive contract with a console |
| `ch552-padprog list\|plan\|flash\|blank [--yes]` | program the pad (see `docs/hardware-ch552.md`) |

## Building and testing

```sh
cmake -S . -B /mnt/ai/keypad-lab/build/control-surface -G Ninja
cmake --build /mnt/ai/keypad-lab/build/control-surface
ctest --test-dir /mnt/ai/keypad-lab/build/control-surface --output-on-failure
CS_TEST_REAL_PAD=1 /mnt/ai/keypad-lab/build/control-surface/tst_paddevice   # grabs and releases the real pad
```

The uinput test grabs its own virtual keyboard before emitting, so no key ever
reaches the desktop.

## Documents

* `docs/hardware-ch552.md`: protocol research, what was flashed, how to verify and restore.
* `docs/kdenlive-api-contract.md`: the Kdenlive interface proposal with an
  editing inventory and a coverage matrix.
* `docs/design.md`: daemon architecture, latency and safety decisions.
