# Settings API: `org.smplos.ControlSurface1`

The D-Bus interface that `control-surfaced` (and `mock-control-surfaced`)
offers to the smplOS Settings "Keypad" app, the bar icon and scripts.
Background and roadmap: `docs/GENERALIZATION-PLAN.md`.

| | |
|---|---|
| Bus | session |
| Service | `org.smplos.ControlSurface` |
| Object | `/org/smplos/ControlSurface` |
| Interface | `org.smplos.ControlSurface1` |
| Version | property `ApiVersion` = 1. Additions (new methods, signals, JSON fields) keep 1; a breaking change gets `ControlSurface2`. Clients ignore unknown JSON fields. |

Structured results are **JSON strings** with an `"ok"` field. Failures carry
`"error": {"code", "message"}`. Simple state lives in typed properties, which
emit `org.freedesktop.DBus.Properties.PropertiesChanged`.

## Properties (read-only)

| Name | Type | Meaning |
|---|---|---|
| `ApiVersion` | u | 1 |
| `DaemonVersion` | s | e.g. `0.1.0`, or `mock-0.1.0` from the mock |
| `Mode` | s | `run`, `dry-run` (keys printed, not sent) or `mock` |
| `DevicePresent` | b | a 1189:8890 pad is connected and held by the daemon |
| `FirmwareType` | s | `control-surface`, `openmacropad`, `stock`, `unknown`; `""` when absent |
| `ActiveProfile` | s | profile chosen for the focused window |
| `ActiveLayer` | s | layer of the last dispatched input (`""` = profile base bindings) |
| `ConfigHash` | s | SHA-256 of the config text in effect |
| `Identifying` | b | identify mode is on (actions suppressed) |
| `CheatsheetVisible` | b | the cheatsheet overlay should be shown |

## Signals

| Signal | When |
|---|---|
| `InputEvent(s slot, s event, i delta)` | Every pad input, always (identify or not). `slot`: `key1`..`key16`, `knob1`..`knob3`. `event`: `press`, `release` (keys and knob switches), `ccw`, `cw` (one detent each). `delta`: -1 for ccw, +1 for cw, 0 otherwise. |
| `IdentifyChanged(b on)` | identify mode started or ended |
| `DeviceChanged(b present, s firmwareType)` | hot-plug or a firmware change; for the bar icon |
| `ConfigChanged(s hash)` | a new config is in effect (SetConfig, ReloadConfig, or a file edit) |
| `ConfigRejected(s error)` | a new config was refused; the previous one stays in effect |
| `PluginsChanged()` | a plugin's status changed; call `ListPlugins` |
| `FlashProgress(s jobId, s phase, s message)` | flash job progress (see below) |
| `CheatsheetVisibilityChanged(b visible)` | show or hide the overlay |
| `CheatsheetChanged(s json)` | the overlay's content: once when shown, then on every change while shown (focus, profile, Kdenlive layer, mode, config, layout) |

## Methods

### State

| Method | Returns |
|---|---|
| `GetStatus() → s` | everything at once: `{ok, apiVersion, daemonVersion, mode, device, layout, activeProfile, activeLayer, window: {class, title}, config: {path, hash, error, warnings[]}, identify: {active, remainingMs}, cheatsheet: {visible, eww}, flash: job \| null}` |
| `GetDevice() → s` | `{ok, present, vendor, product, serial, manufacturer, productName, bcdDevice, firmware: {type, version, versionSource, board, slots}, inputMode, devnodes[], layout}`; only `{ok, present: false, layout}` when absent. `inputMode`: `raw` (firmware events, report 5) or `evdev-chords`. `firmware.version` is the full version from the firmware's `GET_INFO` (`"2.0.1"`, `versionSource: "GET_INFO"`, `slots` its slot count) when the control-surface firmware answers (asked in raw mode, and once per plug-in with `device.input: "evdev"`), else `"2.0"` from bcdDevice (`versionSource: "bcdDevice"`, `slots: null`). The same object is in `GetFirmwareStatus().device`. |
| `GetLayout() → s` | `{ok, layout}` or `{ok: false, error: {code: "unknown"}}` |
| `ListBoardProfiles() → s` | `{ok, profiles: [layout...]}` |

A **layout** is
`{id, name, source, rows, columns, slotCount, keys: [{control, slot, row, column}], knobs: [{control, slots: {ccw, press, cw}, row, column}]}`,
and where it is the pad's layout in effect (`GetLayout`, `GetDevice`,
`GetStatus`, `ValidateConfig`, `status`, `check-config`) also
`firmwareLayout` (the layout the firmware names, or null), `firmwareSlots`
(its slot count, from `GET_INFO` when known, or null) and `matchesFirmware`
(slot counts agree; null without firmware information).
Rows and columns count from 0, and knobs sit in the right-most column.

The layout in effect, first match wins:

1. `config`: the config's `"layout"`, an explicit override (a variant, or a
   Custom grid from Settings). It wins over the firmware.
2. `firmware`: the board the pad's descriptors name.
3. `hardware-map`: derived from a hardware map (learned by `verify`, inline,
   or a scheme the config names).
4. `default`: nothing known; the measured 15+3 board.

When an override has another slot count than the firmware, the config gets
a warning (`GetStatus().config.warnings`, `ValidateConfig`, `check-config`,
starting with `layout:`). Raw input then stays off: the firmware's slots
would land on the wrong controls, so the pad's own keymap (evdev chords) is
used, and inputs outside the layout show nowhere. `measured` and `template`
are the sources of the built-in profiles in `ListBoardProfiles`. The built-in profiles are `sy181-15k3e`
(measured: this pad) and the grid templates `generic-3k`, `generic-3k1e`,
`generic-6k1e`, `generic-10k`, `generic-12k2e`, `generic-12k3e` and
`generic-16k3e`. A config sets one with `"layout": "<id>"` or
`"layout": {"keys": 0..16, "knobs": 0..3, "columns": 1..8}`.

### Identify ("press a key to map it")

`SetIdentify(b on)`. While on, every input still produces `InputEvent`, but
nothing is dispatched (no keys, no Kdenlive actions). It turns off:
* with `SetIdentify(false)`;
* when the caller's bus connection goes away (a crashed Settings app never
  leaves the pad muted);
* after 10 minutes as a safety net. Calling `SetIdentify(true)` again renews it.

### Config

| Method | Returns |
|---|---|
| `GetConfig() → (s text, s path, s hash)` | the file as it is on disk and its SHA-256 (`""` if missing) |
| `ValidateConfig(s text) → s` | `{ok, errors[], warnings[], profiles: [{name, layers, bindings, kdenlive, keyFallback}], hardwareSource, layout}`: `layout` is what this config would give on the pad plugged in now (with `firmwareLayout`, `matchesFirmware`); a mismatching override adds a `layout:` warning. Nothing is written or applied. |
| `SetConfig(s text, s expectedHash) → s` | `{ok, hash, backup, errors[], warnings[], error?}` |
| `ReloadConfig() → s` | `{ok, hash, warnings[], error?}` (re-reads the file) |

`SetConfig` rules:
* The text must validate (error code `invalid`).
* `expectedHash` must equal the file's current hash from `GetConfig`; pass `""`
  to create a missing file. A mismatch means the file changed since it was read
  (error `hash-mismatch`): read it again and merge.
* The text must be at most 1 MiB (`too-large`).
* The previous file is kept as `<path>.bak`, and the new one is written
  atomically, then applied.
* `apply` means the file was written but the daemon refused to apply it.
* A hand edit is never overwritten silently.

For line-level error locations, use `control-surfaced check-config -c FILE
--json`. It reports `error: {message, profile, layer, slot}` and
`warningDetails`.

### Plugins and catalogs

| Method | Returns |
|---|---|
| `ListPlugins() → s` | `{ok, plugins: [{id, name, tier, status, detail, apps[], ...}]}` |
| `GetCatalog(s pluginId) → s` | for `kdenlive`: `{ok, contract, actions: [{id, text, shortcut, checkable, group, editing, playback}], controls: [{name, stage, unit, editing, description}], commands: [...]}`, offline (the 71 curated actions, 10 controls, 3 commands). Other ids: `unknown-plugin`. |
| `GetFeatures() → s` | binding kinds with examples, `keyNames`, `modifierNames`, `mouseNames`, slot grammar and limits (16 keys, 3 knobs), layouts, device options |

Plugins today:

| id | tier | status values |
|---|---|---|
| `keys` | keys | `ready`, `unavailable` (no /dev/uinput yet; retried every 10 s), `dry-run` |
| `command` | command | `ready`, `dry-run` |
| `kdenlive` | api | `detached`, `pending`, `absent`, `available` |

For `kdenlive`, `apps[]` holds the window-class regexes of the profiles that
use it, plus `contract` and `pid`. A running Kdenlive's own action list stays
authoritative (`control-surfaced list-capabilities`).

### Firmware

| Method | Returns |
|---|---|
| `GetFirmwareStatus() → s` | `{ok, device, images: [{path, size, sha256, meta?, metaMatches?}], imageDirs[], flash: {allowed, tool, toolFound, job}}`. `meta` is the image's `<name>.json` (name, version, board, licence, sha256, `verifiedOnHardware`, `hardwareVerification`, `knownIssues`, `supersededBy`, ...). Offer the newest image without `supersededBy`. |
| `StartFlash(s image, s sha256, b dryRun) → s` | `{ok, jobId, dryRun}`, or an error: `busy`, `invalid-arguments`, `not-allowed` |
| `CancelFlash(s jobId) → b` | false if unknown, finished, or writing |

Flash rules:
* A **dry run** is always allowed and touches nothing. It walks every phase
  and ends `done`; `notes` in the job say what a real run would lack.
* A **real** flash needs all of the following:
  * the daemon was started with `--allow-flash`;
  * the image is an absolute path inside an image directory
    (`~/.local/share/control-surface/firmware`,
    `/usr/share/control-surface/firmware`, or `--firmware-dir`), and symlinks
    out of them do not count;
  * the image is 1..14336 bytes;
  * the caller's SHA-256 matches the file;
  * the flash tool (`wchisp`) is found.
* The tool is always run as `wchisp flash <file>`. It writes code flash only,
  never the chip's configuration registers.
* `<file>` is a private 0600 copy of the bytes that were hashed at the check.
  Changing or re-pointing the image afterwards has no effect.
* wchisp cannot be told which device to use (`-d` is a libusb index), so the
  job flashes only when the new bootloader session is the **only** WCH ISP
  device (4348:55e0 or 1a86:55e0) connected. Otherwise it keeps waiting and
  says so.
* The pad must come back on the same USB port. Another 1189:8890 pad that was
  already connected does not count.

Phases (`FlashProgress`):

| Phase | What happens |
|---|---|
| `checking` | the checks above |
| `release-grab` | the daemon lets go of the pad |
| `waiting-bootloader` | Open firmware is asked to switch to the ROM bootloader by itself (other WCH ISP devices must be unplugged). Otherwise, or if that fails, the message tells the user to unplug the pad, hold the top-left key, plug it in and let go. Only a *new* 4348:55e0 session counts. Timeout 120 s. |
| `flashing` | the tool runs; cancelling is refused |
| `waiting-device` | the pad must come back as 1189:8890 within 30 s |
| `verifying` | the firmware it reports |
| `done` / `failed` / `cancelled` | the daemon grabs the pad again |

The job JSON (in `GetStatus().flash` and `GetFirmwareStatus().flash.job`) is
`{id, image, dryRun, phase, message, finished, ok, toolExitCode?, toolOutput?, firmware?, notes?}`.

### Cheatsheet

An overlay of what every key and knob does **right now**. It follows the
focused app's profile, Kdenlive's context layers (for example the colour-wheel
layer instead of the timeline), and daemon modes; an unknown app shows the
global profile. The daemon supplies only content and visibility; the desktop
draws it (smplOS: the running eww, which the daemon updates itself), so no
extra process is involved.

| Method | Returns |
|---|---|
| `GetCheatsheet() → s` | the content below, for the focused window (also while hidden) |
| `GetCheatsheetFor(s windowClass, s title, s kdenliveContextJson) → s` | the same for any app and Kdenlive context (`""` or e.g. `{"colorWheels": true}`), for editors. Kdenlive is assumed to answer. Changes nothing. |
| `ShowCheatsheet()`, `HideCheatsheet()`, `ToggleCheatsheet()` | for the bar, hotkeys or Settings |

Shown and hidden by a binding (`{"cheatsheet": "toggle"}`, or `"hold"`: shown
while held; keys and knob presses only), by these methods, or hidden by
`autoHideMs` without pad input. Unplugging the pad hides it.

Content:

```json
{"ok": true, "visible": true, "title": "Kdenlive · Wheels", "profile": "Kdenlive",
 "layers": ["Wheels"], "window": {"class": "org.kde.kdenlive", "title": "…"},
 "context": {"focus": "effectStack"}, "notice": "",
 "options": {"opacity": 0.85, "autoHideMs": 0, "position": "center"},
 "layout": {"id": "sy181-15k3e", "name": "…", "rows": 3, "columns": 6, "source": "firmware"},
 "keys":  [{"control": "key1", "row": 0, "column": 0, <entry>}, …],
 "knobs": [{"control": "knob1", "row": 0, "column": 5,
            "ccw": <entry>, "press": <entry>, "cw": <entry>, "shiftCcw": <entry>, "shiftCw": <entry>}, …]}
```

`<entry>` is
`{"bound", "label", "kind", "custom", "state", "binding", "profile", "layer", "active"}`:

* `label` is the binding's own `"label"` (`custom: true`), or a readable name
  made from what it does: Kdenlive catalog titles ("Set Zone In"), controls
  ("Jog", "Lift R", "Trim out"), chords ("Ctrl+Z"), media keys ("Volume up"),
  mouse actions ("Scroll down"), "Run notify-send".
* `state` is a cycle's current value (e.g. the wheel axis `r`).
* `active` is false when the binding would do nothing now: Kdenlive's
  interface is off or not answering, or it does not offer that action or
  control.
* Unbound inputs have `bound: false`, `label: ""`, `kind: "none"`.
* `context` is present only while Kdenlive answers, and holds only `focus`;
  the playhead is left out, so playback does not resend the content.
* `notice` explains inactive Kdenlive bindings (interface off, or not
  answered yet).

Config: `"cheatsheet": {"opacity": 0.05..1, "autoHideMs": 0..600000 (0 = until
hidden; restarted by pad input), "position": "center|top|bottom|left|right|top-left|top-right|bottom-left|bottom-right",
"eww": see below}`.

#### Pushed into eww (no listener process)

The daemon puts the sheet into eww itself, so the desktop runs nothing extra
for it; the daemon only runs while a pad is in use. Every call is
`eww [--config DIR] …`, one at a time and in order; while one runs only the
latest state is kept (content changes are already coalesced over 40 ms):

| When | Calls |
|---|---|
| daemon start (and when the push is turned on) | `close WINDOW` (a crash may have left it open), `update pad_sheet=<hidden JSON>` |
| shown | `update pad_sheet=<GetCheatsheet JSON>`, then `open WINDOW --anchor A`, skipped when the update failed (an `open` would start an eww daemon) |
| content changes while shown | `update pad_sheet=<JSON>` |
| position changes while shown | `close WINDOW`, `update …`, `open WINDOW --anchor A` |
| hidden, unplugged, daemon exit (SIGTERM) | `close WINDOW`, then `update pad_sheet=<JSON with "visible": false>` |

`pad_sheet` is the full `GetCheatsheet` content (the hidden one keeps every
field, so widget expressions stay valid). Without a window only the variable
is updated. Anchors per `position`: center → `center`, top → `top center`,
bottom → `bottom center`, left → `center left`, right → `center right`,
top-left → `top left`, top-right → `top right`, bottom-left → `bottom left`,
bottom-right → `bottom right`. A failing eww is logged once until it works
again; `GetStatus().cheatsheet.eww` is
`{enabled, variable, window, config, calls, failures, lastError}`.

Turning it on: the command line gives defaults, the config overrides them.

* `control-surfaced run --eww` (variable only), `--eww-window NAME`,
  `--eww-config DIR`; any of them turns the push on. smplOS's unit:
  `ExecStart=/usr/bin/control-surfaced run --quiet --eww-window pad-cheatsheet --eww-config %h/.config/eww`.
* Config `"cheatsheet": {"eww": …}`: `false` turns it off; `true` turns it
  on with the flags' window and directory; an object
  `{"enabled": true, "variable": "pad_sheet", "window": "…", "config": "…", "binary": "eww"}`
  replaces only the fields it names (`"window": ""` = variable only).
  Changes apply on reload; a sheet shown under the old settings is taken down
  first.
* `mock-control-surfaced` takes the same three flags.

eww side:

```lisp
(defvar pad_sheet '{"visible":false}')
(defwindow pad-cheatsheet :stacking "overlay" :geometry (geometry :anchor "center")
  (box :class "pad-sheet" :style "opacity: ${pad_sheet.options.opacity}" :orientation "v"
    (label :text {pad_sheet.title})
    (box :orientation "h"
      (for k in {pad_sheet.keys}
        (label :class {k.active ? "on" : "off"} :text {k.label})))))
```

`control-surfaced cheatsheet --follow` (one JSON line at start and on every
change, shown or hidden) stays for debugging.

## CLI mirrors

| Command | Output |
|---|---|
| `control-surfaced monitor [--json]` | One line per input: `{"slot","event","delta","ms"}`. It follows a running daemon (or the mock) over D-Bus; without one, it reads the pad directly (grabbed, nothing dispatched). |
| `control-surfaced status [--json]` | The daemon's `GetStatus`, or offline: `{ok, daemon: false, mode: "offline", device, bootloaderPresent, layout, config}` (sysfs only: the version is bcdDevice's) |
| `control-surfaced check-config [-c FILE] --json` | `{ok, error: {message, profile, layer, slot} \| null, warnings[], warningDetails[], profiles[], path, source, layout}`; `layout` and a `layout:` mismatch warning against the pad plugged in now (sysfs only); exit 0 or 2 |
| `control-surfaced list-actions [--json]` | `GetCatalog("kdenlive")` offline |
| `control-surfaced features [--json]` | `GetFeatures` |
| `control-surfaced cheatsheet [--json] [--follow]` | the running daemon's `GetCheatsheet`; `--follow` prints a JSON line on every change (debugging; eww gets it pushed, see Cheatsheet) |
| `control-surfaced cheatsheet --window CLASS [--title T] [--context JSON]` | offline preview from the config (no daemon) |
| `control-surfaced firmware-info [--json]` | `GET_INFO` from a pad running the control-surface firmware: `{ok, node, version, format, slots, layers, activeLayer, startLayer, rawActive, eepromBytes, stats?}`. From 2.0.1, `stats` is `{knobs: [{cw, ccw, illegal}], overruns, queueDrops, maxQueue}`, the encoder counters since power-on or the last clear. |
| `control-surfaced enter-bootloader --yes [--json]` | `CMD_BOOTLOADER`; the pad shows as 4348:55e0 until flashed or replugged |

`firmware-info` and `enter-bootloader` only talk to a hidraw node whose report
descriptor has reports 3 and 5 (protocol v3), never to stock firmware.

## Config additions the editor can write

* Mouse bindings: `{"mouse": "left|right|middle|back|forward|wheel-up|wheel-down|wheel-left|wheel-right"}`.
* Cheatsheet bindings: `{"cheatsheet": "toggle"}` or `{"cheatsheet": "hold"}`
  on `keyN` or `knobN.press`, and the root `"cheatsheet"` options above. Any
  binding object may carry `"label"` for the overlay.
* Slots `key1`..`key16`.
* `"layout"`, described under State.
* `"device": {"serial": "", "input": "auto|evdev|raw"}`. An empty serial
  drives the first 1189:8890 pad found and keeps it if another is plugged in.

## The mock

`mock-control-surfaced [--config FILE] [--firmware-dir DIR] [--board ID] [--firmware control-surface|openmacropad|stock] [--unplugged]`

* It serves the same interface with a simulated pad. Its config and images
  default to `$XDG_RUNTIME_DIR/control-surface-mock/`, never the user's
  config.
* It prints the path and SHA-256 of a simulated image to flash.
* Input goes through the real engine into recorded keys; nothing is typed and
  nothing is run.
* Stop the real daemon first: only one process can own the name.

Extra interface `org.smplos.ControlSurface1.Mock` at `/org/smplos/ControlSurface/Mock`:

| Method | Effect |
|---|---|
| `Plug(s firmwareType, s board)` / `Unplug()` | hot-plug (`DeviceChanged`) |
| `Press(s control)`, `Hold(s control)`, `Release(s control)` | `key1`..`key16`, `knob1`..`knob3` (knob switch) |
| `Turn(s knob, i detents)` | -100..100, one event per detent |
| `Focus(s windowClass, s title)` | picks the profile |
| `SetPluginStatus(s id, s status)` | `""` restores the real status |
| `EnterBootloader()` | what the user does with the boot key (a new bootloader session) |
| `SetFlashOutcome(s outcome)` | `ok`, `tool-fails`, `no-return` |
| `TakeKeys() → as` | key and mouse actions dispatched since the last call (`F13`, `mouse:left`, ...) |
| `SetKdenliveState(s state)` | `available`, `absent`, `pending`, `detached` |
| `SetKdenliveContext(s json)` | Kdenlive's context as the engine sees it, e.g. `{"colorWheels": true}` for the wheel layer |
| signal `Dispatched(s slot, s binding, s layer)` | what the engine did with an input |

## Examples

```sh
# busctl prints real JSON; the method's own JSON is the first data element.
busctl --user --json=short call org.smplos.ControlSurface /org/smplos/ControlSurface \
  org.smplos.ControlSurface1 GetStatus | jq -r '.data[0]' | jq .device
gdbus call --session -d org.smplos.ControlSurface -o /org/smplos/ControlSurface \
  -m org.smplos.ControlSurface1.SetIdentify true
gdbus monitor --session -d org.smplos.ControlSurface -o /org/smplos/ControlSurface
control-surfaced monitor --json | jq -c 'select(.event=="press")'
```
