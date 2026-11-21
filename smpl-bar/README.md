# smpl-bar

Resident native Rust/Slint taskbar for the smplOS EWW migration. The binary is
standalone: Start Menu and the other smpl-apps remain separate processes.
`smpl-shell` contains the reusable desktop primitives.

## Status and side-by-side contract

This checkout implements a real vertical slice. It is **not** an in-place EWW
replacement yet and must remain opt-in until the OS integration gates below are
complete. Nothing here starts, stops, patches, or deploys EWW.

| EWW feature | Native status | Notes / gate |
|---|---|---|
| Start menu | Implemented | Launches the existing `toggle-start-menu` interface |
| Hyprland workspaces/title | Implemented | Direct `.socket.sock` queries and `.socket2.sock` event subscription; no `hyprctl`/`socat` in the event path |
| Niri workspaces/title | Process-backed gate | Persistent `niri msg event-stream` and bounded snapshots; direct `NIRI_SOCKET` decoding is still required for full acceptance |
| X11 fallback | Implemented | Native `x11rb` EWMH roots, PropertyNotify and direct `_NET_CURRENT_DESKTOP` action |
| USB/keypad/sync/dictation/keyboard | Implemented / integration snapshot | Sysfs/procfs plus existing launch/service interfaces; live service adapters are bounded and replaceable |
| Bluetooth/Wi-Fi/network/VPN | Implemented snapshot | Direct sysfs/rfkill/route state; SSID/device details still need native NM/BlueZ adapters |
| Volume/battery/clock/notifications | Implemented snapshot | Battery/clock are native; audio and dunst state probes run off the UI path; history/actions remain in native `notif-center` |
| Quick Settings | Implemented | Connectivity, airplane, night light, DND, dictation, volume, brightness, settings/logout/power actions |
| Calendar | Implemented | Launches the existing resident-capable native `toggle-calendar`/calendar process |
| USB eject | Implemented | Native popup and `udisksctl` action |
| Workspace overview/identify | Implemented, limited | One native overview and identify window; one identify window per monitor and stable-policy overview projection remain gated |
| Keypad cheatsheet | Implemented IPC/UI | Accepts typed sheet JSON; click-through rendering is gated by compositor input-region support |
| Generic SNI tray | **Acceptance gate** | No host, icon pixel, context-menu, DBusMenu, activation, watcher or owner-restart semantics are claimed |
| Exclusive layer bar | **Acceptance gate** | Winit creates a transparent borderless toplevel; a true Wayland layer-shell/strut is not implemented |

Unknown, unavailable, or explicitly gated modules do not pretend to be present:
typed `sni` configuration is rejected by `smpl-shell::config` with an
acceptance-gated error.

## Run

```bash
cargo build --release -p smpl-bar
target/release/smpl-bar --check
target/release/smpl-bar --resident --hidden
target/release/smpl-bar show
target/release/smpl-bar status
target/release/smpl-bar quit
```

`--preview` requests a smaller width for a private side-by-side comparison.
Positioning still belongs to the compositor because Slint/Winit does not provide
a Wayland layer-shell.

Compatible `bar-ctl`-style commands are:

```text
start|show|open
hide|close|stop
toggle
apply|reload
status|ensure
identify|refresh|redraw
popup quick-settings|usb|workspace-overview|keypad-cheatsheet
popup --close
set-keypad-present yes|no
keypad-sheet FILE|JSON|-
quit|kill
```

## IPC

The resident owns one Unix stream socket:

```text
${XDG_RUNTIME_DIR:-/run/user/$UID}/smplos/smpl-bar.sock
```

Requests are one bounded (128 KiB) newline-terminated JSON object; replies are
newline-terminated `{"ok":...}` responses. The socket directory and socket use
0700 and 0600 permissions. The CLI above is the supported client.

Representative requests:

```json
{"command":"show"}
{"command":"popup","name":"quick-settings"}
{"command":"apply"}
{"command":"set-keypad-present","present":true}
{"command":"keypad-sheet","sheet":{"visible":true,"title":"Editor","keys":[]}}
{"command":"quit"}
```

The `sheet` object follows control-surface's existing cheatsheet shape:
`visible`, `title`, `notice`, `keys` grouped as rows, and `options`. Click-through
is a capability field only; `smpl-bar` does not silently claim the compositor
input region was installed.

## Typed customization

The existing `~/.config/smplos/bar.conf` keys remain authoritative for
`ws_count`, `ws_position`, `ws_spacing`, and `ws_style`, with duplicate/invalid
managed values rejected. A native, fully typed overlay may be saved as
`~/.config/smplos/bar.json`:

```json
{
  "version": 1,
  "workspace-count": 4,
  "workspace-position": "left",
  "workspace-spacing": 2,
  "workspace-style": "numbers",
  "height": 32,
  "preview-offset-y": 36,
  "clock-format": "time",
  "clock-24h": true,
  "clock-date-format": "M/D",
  "left": ["start", "workspaces", "window-title"],
  "right": [
    "usb", "keypad", "sync", "dictation", "keyboard",
    "bluetooth", "wifi", "network", "volume", "battery",
    "clock", "notifications"
  ]
}
```

Native config rejects unknown fields, duplicate modules, out-of-range values,
and `sni`. Applying it reloads the layout without replacing or reinstalling a
bar process.

## Theme and alpha

The bar calls `smpl_common::init("smpl-bar", ...)`, so CPU Skia, shared alpha
patches, no CSD, the Tokio UI-thread context and the `smpl-bar` Wayland app ID
remain centralized. The bar watches the existing EWW palette with
`ThemeRole::Application`; popups independently use `ThemeRole::Popup`.
Foregrounds stay opaque and only the backing rectangles apply background
alpha.

## Telemetry and benchmark

Telemetry is off unless `SMPL_BAR_TELEMETRY=1`. `SMPL_BAR_TELEMETRY_FILE` writes
0600 JSON lines; without it lines go to stderr. Recorded milestones are:

* process/config/backend/IPC startup;
* compositor detection, subscribe and snapshot;
* first map request and first Slint snapshot paint;
* popup open latency and first snapshot paint;
* RSS, PSS, HWM and thread count;
* loop and UI-poll stalls;
* module discovery/probe completion.

The measurement script never connects to the user's compositor. It starts a
private headless Weston under the repository's `target/`, samples the existing
installed EWW process without changing it, runs smpl-bar, and writes a JSON
report:

```bash
smpl-bar/scripts/benchmark.sh
smpl-bar/scripts/benchmark.sh --duration 12 --output target/my-bar-bench
smpl-bar/scripts/benchmark.sh --hidden
```

The installed baseline read on the development machine was 46,508 KiB RSS,
70,508 KiB HWM, 73 threads and a 17,982,992-byte EWW binary. The final isolated
release measurements were:

| State | smpl-bar RSS | PSS | HWM | Threads | Binary | Slint paint |
|---|---:|---:|---:|---:|---:|---:|
| Hidden resident | 23,788 KiB | 13,225 KiB | 23,788 KiB | 4 | 13,596,856 B | not mapped |
| Mapped bar | 35,940 KiB | 23,681 KiB | 36,176 KiB | 4 | 13,596,856 B | 266,045 us |
| Bar + Quick Settings | 41,168 KiB | 29,557 KiB | 41,308 KiB | 4 | 13,588,152 B | popup 27,705 us |

The binary is 75.6% of EWW and thread count is 5.5%. Mapped RSS remains below
the contemporaneous EWW sample, but neither hidden nor mapped RSS meets the
23 MiB absolute acceptance target, so the script reports that failure
truthfully and `--enforce` remains a rollout gate.
The generic Weston fixture measures the normal mapped bar; `--preview` is
Hyprland-only and is rejected on Niri/X11/unknown sessions rather than risking
an overlapping side-by-side window. The EWW process is only sampled, so the
script does not invent a cold-start
ratio from an already-running process. A dedicated, isolated EWW cold-start
fixture and `first_paint <= 0.8x` comparison are still acceptance work.

## OS-side requirements (not implemented here)

1. **Opt-in coexistence:** install the release `smpl-bar` but keep EWW's current
   autostart and `bar-ctl` untouched until an explicit setting switches owners.
   A side-by-side test may start
   `smpl-bar --resident --hidden` and use IPC to show it while EWW remains.
2. **Wayland placement:** add compositor rules for Wayland app ID `smpl-bar`
   and preview titles `smpl-bar:*`. Position the 30–48 px bar at bottom center,
   disable CSD/borders/focus as appropriate, and use neutral window opacity
   `1.0 override` for self-managed alpha.
3. **Exclusive reservation:** a Winit toplevel cannot claim a layer-shell
   exclusive zone. Before replacing EWW, either add a Slint custom layer-shell
   platform (preferred for exact geometry/exclusivity) or prove a compositor
   strut/rule that reserves the selected height without blocking its popups.
4. **`bar-ctl` bridge:** route `start|toggle|stop|ensure|reload|apply|refresh`
   to the smpl-bar CLI/socket in native mode. Keep `bar.conf` and the
   `workspace-ctl`/Settings response contract unchanged.
5. **Popup bridge:** route `quick-settings`, `usb-popup`, and
   `workspace-overview` in `popup-toggle`; close native popups before opening
   another. Existing `toggle-calendar`/`toggle-notif-center` remain separate
   application toggles.
6. **Workspace policy:** preserve monitor-owned IDs by translating
   `workspace-ctl watch/status` into `smpl-bar` state, or move that policy
   projection into `smpl-shell`. The native vertical slice currently groups
   raw IDs and does not synchronize policy files.
7. **Keypad:** replace systemd `eww update keypad-present=...` with
   `smpl-bar set-keypad-present yes|no`; replace `eww update pad_sheet=...`
   and `eww open/close pad-cheatsheet` with the `keypad-sheet` JSON request.
   Do not remove the existing owner service or click-to-hide policy.
8. **Click-through:** map the cheatsheet click-through mode to an actual empty
   Wayland/X11 input region. Setting the JSON flag alone is not acceptance.
9. **Monitor services:** one identify window must be mapped on every connected
   output, following the selected primary monitor and workspace policy.
10. **Lifecycle:** add a user unit or compositor autostart only after the above
    switch exists. It must terminate `smpl-bar` cleanly, wait for its socket and
    never run a second owner alongside EWW by default.
11. **No false tray parity:** SNI is not required for the opt-in preview but is
    an explicit release gate for EWW replacement: watcher registration, item
    discovery/removal, owner changes, icon variants, activation and DBusMenu.

These requirements intentionally live in app documentation; this task does not
modify the smplOS repository.

## Acceptance gaps

The implementation tests do not turn these TODOs into passing claims:

* direct Niri socket adapter with no helper process;
* native PulseAudio/PipeWire, NetworkManager, BlueZ and dictation event
  subscriptions (the current external probe is bounded/off the UI path);
* stable workspace-policy projection and per-monitor identify;
* Winit-compatible Wayland layer-shell/exclusive-zone support;
* click-through keypad input-region support;
* generic SNI;
* mapped-window steady RSS `<= 23 MiB`;
* a safe, isolated EWW cold-start baseline and the `< 0.8x` first-paint gate.
