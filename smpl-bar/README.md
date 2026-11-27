# smpl-bar

Resident native Rust/Slint taskbar for the smplOS EWW migration. It remains
an opt-in, side-by-side backend; EWW and its release/fallback contracts are not
changed. Start Menu and the other smpl-apps remain separate processes.

## Runtime architecture

The Slint surface still uses `smpl_common::init("smpl-bar", ...)`, which selects
the patched alpha-safe Skia CPU renderer from the first frame. The bar paints
alpha only in backing rectangles; foreground colors remain opaque. Hiding and
showing a Wayland window destroys/recreates the Winit adapter while retaining the
same Slint model, `smpl-bar` application ID, renderer and compositor-rule
contract.

Wayland uses a split panel architecture rather than treating Winit's
`xdg-toplevel` as an exclusive panel:

* the resident Slint window owns rendering and pointer callbacks;
* a transparent, click-through `zwlr_layer_surface_v1` reserves the configured
  edge with an exclusive zone when the panel is pinned;
* direct Hyprland IPC or Niri IPC moves the Slint surface to the selected output
  and exact edge coordinates;
* floating mode keeps the layer reservation out of the work area;
* hotplug/output changes recreate the reservation on the connector selected by
  the saved smplOS workspace policy.

The layer reservation protocol is real. Its namespace is
`smpl-bar-reservation`. The visible Slint surface remains `smpl-bar` for the
alpha, blur and window-rule contracts; it is not represented as an untested
custom Skia platform.

On X11, smpl-bar sets `_NET_WM_WINDOW_TYPE_DOCK`,
`_NET_WM_STATE_STICKY`/`ABOVE` and the matching `_NET_WM_STRUT` and
`_NET_WM_STRUT_PARTIAL` ranges for a pinned panel. Floating mode uses a normal
window and clears the strut.

All four edges and pinned/floating behavior are configured by:

```json
{
  "version": 1,
  "edge": "bottom",
  "mode": "pinned",
  "margin": 2,
  "height": 32
}
```

Older `bar.conf` files can use `bar_edge=top|bottom|left|right`,
`bar_mode=pinned|floating`, and `bar_margin=0..24`. Unknown legacy keys remain
untouched. When both files exist, native `bar.json` remains the owner of
module layout and native-only options, while `bar.conf` values written by
Settings are authoritative for workspaces, clock formatting, edge, mode and
margin.

## Run

```bash
cargo build --release -p smpl-bar
target/release/smpl-bar --check
target/release/smpl-bar --resident --hidden
target/release/smpl-bar show
target/release/smpl-bar status
target/release/smpl-bar quit
```

Compatible `bar-ctl` commands:

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
close-popups|telemetry-sample
quit|kill
```

The resident owns `${XDG_RUNTIME_DIR:-/run/user/$UID}/smplos/smpl-bar.sock`.
IPC accepts bounded 128 KiB newline JSON and returns bounded newline JSON.
Socket permissions are `0600`; the directory is `0700`.

## Features

| Area | Implementation |
|---|---|
| Start menu | Closes native/OS popups, then signals the resident Start Menu PID with `SIGUSR1`; hidden state, position and cache remain resident |
| Workspaces/title | Hyprland direct sockets, direct Niri socket event stream, X11 EWMH PropertyNotify |
| Geometry | Per-output top/bottom/left/right placement, direct compositor positioning, Wayland exclusive reservation, X11 EWMH struts, workspace-policy primary monitor |
| Monitor overview | Native monitor geometry plus active/focused workspace state |
| Monitor identify | One independent overlay on every connected output, placed by Hyprland/Niri IPC or X11 positioning |
| USB/keypad/dictation/sync/keyboard | Native sysfs/procfs plus D-Bus event providers; bounded fallback probes |
| Wi-Fi/network/VPN | NetworkManager session/system D-Bus properties and active-connection events |
| Bluetooth | BlueZ ObjectManager and `Powered` property events |
| Volume | One bounded `pactl subscribe` process; no process per UI frame |
| Notifications | Native sync/D-Bus provider state plus bounded `dunstctl` refresh |
| Battery/brightness/clock | Native sysfs/state and UI-local formatting |
| Generic tray | Real `org.kde.StatusNotifierWatcher` fallback, host registration, item lifecycle, owner restart/removal, icon name/pixmap/attention/overlay variants, monochrome theme tinting, activation, scroll and native DBusMenu |
| 24 px edge layout | Fixed controls use a 20 px Slint item inside the 24 px surface with explicit 2 px vertical insets; start and right-edge glyphs cannot paint against the clipped window edge |
| Keyboard | The switcher follows compositor-reported switchability and appears only when more than one distinct keyboard layout is configured |
| Focus title | The monitor connector is confined to the workspace-overview tooltip; bar/popup self-focus cannot replace the last real focused-client title |
| Tray formats | SNI and DBusMenu; legacy XEmbed/arrow-key tray emulation is intentionally not claimed |
| Calendar/notification center | Delegates to the existing canonical `popup-toggle` grammar, retaining mutual exclusion and app ownership |
| Quick Settings | Theme-token command-center popup with a 3/2/1-column responsive grid, bounded viewport, vertical fallback scroll for short/narrow sizes, and the existing connectivity, airplane, night light, DND, dictation, audio, brightness, settings and power actions |
| Keypad cheatsheet | Typed JSON UI plus Control Surface `CheatsheetChanged`/`CheatsheetVisibilityChanged`; real X11 input-shape pass-through |
| Keypad safety | Wayland click-through requests are rejected unless a real compositor input region is available; the JSON flag never makes an unsafe claim |

## StatusNotifier host

smpl-bar first registers as a host with an existing watcher. If no watcher owns
`org.kde.StatusNotifierWatcher`, the bar owns that name and serves the watcher
interface itself. Registrations are limited to 32 items. Icon buffers are
limited to 128×128 pixels and 64 KiB per variant. The host observes
`NameOwnerChanged`, SNI property signals, registration/unregistration and watcher
restarts, so an item owner restart does not leave a stale icon.

Left click calls `Activate`; `ItemIsMenu` and secondary click use DBusMenu.
Menu data is rendered natively, with bounded flattened depth, visibility,
enabled, separator and checked state. Menu activation is sent through
`com.canonical.dbusmenu.Event`. Icon-name, IconPixmap, attention and overlay
variants are supported. No XEmbed parity is fabricated.

Monochrome pixmaps and symbolic/grayscale SVG files are rendered with the
application foreground color. Non-symbolic colored tray artwork is retained
unchanged. Status output names each loaded icon that could not safely be
recolored in `sni.unrecolored_icons`; this is informational rather than an
SNI-host failure.

## Popup and lifecycle contracts

Calendar and notification surfaces remain owned by the resident
`smpl-calendar` and `notif-center` processes. The native bar closes its own
popups, then uses `popup-toggle`; that script closes external resident popups
only when their exact process exists. A global close does not launch a hidden
application.

Start Menu is different: the bar validates the private
`$XDG_RUNTIME_DIR/smplos/start-menu.pid`, confirms the exact process name and
UID, and sends POSIX `SIGUSR1`. It does not spawn another Start Menu and does
not replace the window, therefore hidden state and position survive.

## Typed customization

The Settings-owned values in `bar.conf`—workspace count/position/spacing/style,
clock formatting, edge, mode and margin—remain authoritative. A fully typed
native overlay may still own module layout and native-only options:

```json
{
  "version": 1,
  "workspace-count": 4,
  "workspace-position": "left",
  "workspace-spacing": 2,
  "workspace-style": "numbers",
  "height": 32,
  "edge": "bottom",
  "mode": "pinned",
  "margin": 2,
  "preview-offset-y": 36,
  "clock-format": "time",
  "clock-24h": true,
  "clock-date-format": "M/D",
  "left": ["start", "workspaces", "window-title"],
  "right": [
    "usb", "keypad", "sync", "dictation", "keyboard",
    "bluetooth", "wifi", "network", "volume", "battery",
    "sni", "clock", "notifications"
  ]
}
```

Unknown native fields and duplicate modules are errors. Managed module order is
validated because UI spacing and notification-area semantics depend on it.
Applying configuration reloads the layout and panel contract without replacing
the resident process.

## Telemetry and benchmark

Telemetry is disabled by default. `SMPL_BAR_TELEMETRY=1` enables bounded JSON
events; `SMPL_BAR_TELEMETRY_FILE` selects an append-only `0600` file.
Milestones cover:

* process/config/backend/IPC start;
* compositor and panel transport setup;
* direct Niri socket and monitor/panel actions;
* SNI watcher/host/item and DBusMenu failures;
* provider changes and bounded-probe failures;
* first map, renderer setup and first Slint snapshot paint;
* popup open and first paint;
* RSS, PSS, HWM and thread count;
* loop and UI-poll stalls;
* wayland adapter teardown/recreation across hide/show.

Private measurements never connect to the user's compositor or installed
bar:

```bash
smpl-bar/scripts/benchmark.sh
smpl-bar/scripts/benchmark.sh --duration 12 --output target/my-bar-bench
smpl-bar/scripts/benchmark.sh --recreate
smpl-bar/scripts/benchmark.sh --hidden
```

The script samples an explicitly supplied EWW PID (or the one installed EWW
process), then starts smpl-bar in a private Weston headless compositor. It
retains raw telemetry and compares RSS, PSS, HWM, threads and release binary
size. It does not invent an EWW cold-start paint ratio from an already-running
process. Its private Wayland and D-Bus sockets live below the short
`XDG_RUNTIME_DIR` to remain below the Unix socket path limit in deep worktrees.

The absolute stretch target remains 23 MiB mapped RSS. The alpha-safe patched
Skia/softbuffer CPU stack, Slint scene, icon/SNI surfaces and resident desktop
adapters can exceed that. The renderer is never weakened to meet RSS or paint
targets, and current measurements do not turn an adverse RSS comparison into a
pass.

Current private fixture (2026-11-21, resident SNI/provider buses, same-session
EWW sample):

| State | RSS | PSS | HWM | Threads | Binary | First paint |
|---|---:|---:|---:|---:|---:|---:|
| EWW baseline | 34,324 KiB | 14,711 KiB | 70,508 KiB | 73 | 17,982,992 B | not isolated |
| smpl-bar mapped | 39,124 KiB | 30,352 KiB | 40,064 KiB | 9 | 16,438,152 B | 456,929 us |
| Bar + Quick Settings | 44,748 KiB | 34,316 KiB | 44,748 KiB | 9 | 16,438,152 B | popup 34,904 us |

The hide/show fixture recorded one destroy and two creates, including a
post-recreate `smpl-bar` app ID. The 0.7-alpha snapshot contained 37,833
partial-alpha pixels and 567 opaque foreground pixels; it did not alpha-fade
the full image. The bar release binary remains smaller, HWM is 56% of EWW and
threads are 12% of EWW. On this installed EWW build, mapped RSS is 14.0%
higher (39,124 vs 34,324 KiB), so no RSS comparison pass is claimed.
Meeting 23 MiB with the required alpha-safe Skia presentation, Slint scene and
resident D-Bus services was not feasible without weakening that renderer or
removing accepted native providers.

## Compositor/OS contracts

The app owns direct placement, monitor selection and strut/exclusive behavior.
The smplOS integration still owns:

* opt-in `bar-ctl` backend selection and EWW fallback;
* `self-managed-alpha`, blur and neutral-opacity rules for `smpl-bar`;
* popup/title rules for non-panel windows and `popup-toggle` routing;
* no-focus and no-shadow treatment for the reservation's Winit panel;
* the resident user service/autostart switch only after explicit backend opt-in.

Quick Settings remains open while the pointer leaves its surface. It closes
only through its close button, an IPC/toggle close (including the OS click-away
bridge), or a mutually exclusive popup request. Settings launches
`smplos-settings`; launch failures are shown in the panel. Logout opens an
in-app confirmation and then executes only
`smplos-session-logout --confirmed`.

Niri implementation uses its documented native socket protocol; no `niri msg`
helper process runs in the event or input path. Exact new app-side panel and
identify titles must be included in smplOS compositor rules when their source
is qualified.

## Remaining qualification work

No application gate remains for SNI, panel reservation/strut, providers,
resident Start Menu IPC, keypad D-Bus state, or monitor placement. The
remaining measurements are environmental: compositor-specific blur/pixel
capture, an isolated EWW cold-start fixture for a fair `0.8×` first-paint
comparison, and proving 23 MiB with real SNI traffic. These are reported by
the benchmark rather than converted into passing claims.
