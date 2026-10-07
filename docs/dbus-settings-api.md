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

## Methods

### State

| Method | Returns |
|---|---|
| `GetStatus() → s` | everything at once: `{ok, apiVersion, daemonVersion, mode, device, layout, activeProfile, activeLayer, window: {class, title}, config: {path, hash, error, warnings[]}, identify: {active, remainingMs}, flash: job \| null}` |
| `GetDevice() → s` | `{ok, present, vendor, product, serial, manufacturer, productName, bcdDevice, firmware: {type, version, board}, inputMode, devnodes[], layout}`; only `{ok, present: false, layout}` when absent. `inputMode`: `raw` (firmware events, report 5) or `evdev-chords`. |
| `GetLayout() → s` | `{ok, layout}` or `{ok: false, error: {code: "unknown"}}` |
| `ListBoardProfiles() → s` | `{ok, profiles: [layout...]}` |

A **layout** is
`{id, name, source, rows, columns, slotCount, keys: [{control, slot, row, column}], knobs: [{control, slots: {ccw, press, cw}, row, column}]}`.
Rows and columns count from 0, and knobs sit in the right-most column.
`source` is `firmware` (the pad names its board), `config` (the config's
`"layout"`), `hardware-map` (derived from the learned map), `measured` or
`template` (built-in profiles). The built-in profiles are `sy181-15k3e`
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
| `ValidateConfig(s text) → s` | `{ok, errors[], warnings[], profiles: [{name, layers, bindings, kdenlive, keyFallback}], hardwareSource}`. Nothing is written or applied. |
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

## CLI mirrors

| Command | Output |
|---|---|
| `control-surfaced monitor [--json]` | One line per input: `{"slot","event","delta","ms"}`. It follows a running daemon (or the mock) over D-Bus; without one, it reads the pad directly (grabbed, nothing dispatched). |
| `control-surfaced status [--json]` | The daemon's `GetStatus`, or offline: `{ok, daemon: false, mode: "offline", device, bootloaderPresent, layout, config}` |
| `control-surfaced check-config [-c FILE] --json` | `{ok, error: {message, profile, layer, slot} \| null, warnings[], warningDetails[], profiles[], path, source}`; exit 0 or 2 |
| `control-surfaced list-actions [--json]` | `GetCatalog("kdenlive")` offline |
| `control-surfaced features [--json]` | `GetFeatures` |
| `control-surfaced firmware-info [--json]` | `GET_INFO` from a pad running the control-surface firmware: `{ok, node, version, format, slots, layers, activeLayer, startLayer, rawActive, eepromBytes, stats?}`. From 2.0.1, `stats` is `{knobs: [{cw, ccw, illegal}], overruns, queueDrops, maxQueue}`, the encoder counters since power-on or the last clear. |
| `control-surfaced enter-bootloader --yes [--json]` | `CMD_BOOTLOADER`; the pad shows as 4348:55e0 until flashed or replugged |

`firmware-info` and `enter-bootloader` only talk to a hidraw node whose report
descriptor has reports 3 and 5 (protocol v3), never to stock firmware.

## Config additions the editor can write

* Mouse bindings: `{"mouse": "left|right|middle|back|forward|wheel-up|wheel-down|wheel-left|wheel-right"}`.
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
