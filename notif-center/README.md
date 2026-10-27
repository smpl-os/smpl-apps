# notif-center (Rust + Slint)

Notification center application for smplOS.

## Build

From this directory:

- `cargo build`
- `cargo run`

## Backend actions

- Fetch: `dunstctl history`
- Dismiss one: `dunstctl history-rm <id>`
- Clear all: `dunstctl history-clear`

### Open (row click / Enter)

Activation goes through the shared dispatch function
(`dispatch_notification`), the same one the Slint `activate` callback
calls:

1. The clicked id is looked up in the current notification snapshot.
   An id no longer in the list reports `Stale` and runs no effect.
2. The effect is resolved by `decide_dispatch`, in the order: special
   action, named action, desktop entry, app name.
   - `Web App Launch Error` (special `__OPEN_WEBAPP_LOG__`): when
     `/usr/bin/hyprctl` exists, stale terminal windows are killed
     first and the log is opened in a floating terminal via
     `hyprctl dispatch exec`; the handler returns the command
     invocation result. When that binary is absent, the existing
     `open-log-file` script is used if available, otherwise a zenity
     message; that script/zenity choice depends on helper
     availability, not on a proven fallback after any hyprctl
     failure. Not `xdg-open`.
   - `System Update` (named action `smplos-update`): run via
     `sh -lc`.
   - Default: the existing installed helper
     `/usr/local/bin/focus-or-launch`, targeting the desktop entry
     (`.desktop` suffix stripped) when present, else the app name.
     The helper focuses an existing window or launches from the
     .desktop file; the center does not run `gtk-launch` or an
     app-name command directly.
   - No action, desktop entry, or app name: `Failed`, nothing runs.
3. The outcome is debug-logged as `Dispatched`, `Failed`, or `Stale`.
   `Dispatched` only proves the resolved command was started; it is
   not proof the app was focused, activated, or finished its work.

### Session environment for spawn-detached children

Only the `spawn_detached` helper/default child and the relevant
fallback children are spawned after `env_clear` with a fixed
`PATH`/`HOME` plus the non-empty session variables
`WAYLAND_DISPLAY`, `DISPLAY`, `XDG_RUNTIME_DIR`,
`HYPRLAND_INSTANCE_SIGNATURE`, and `DBUS_SESSION_BUS_ADDRESS`. The
explicit named action (`sh -lc <action>`) and the `open_webapp_log`
direct `hyprctl` commands inherit their environment instead. The
helper's `IS_HYPRLAND` check needs the Hyprland signature (its
envs.conf fallback may lack the runtime signature), and `gtk-launch`
D-Bus activation needs the session bus address.

### Pointer trace

Row pointer events trace `down`/`up` per id into the debug log so a
live click can be correlated with the activation outcome. `move` and
other events are not logged, so mouse movement does not add
per-event filesystem writes.

### Synthetic test limitations

The ten unit tests exercise `decide_dispatch`,
`dispatch_notification`, and `session_env_passthrough` with every
process effect faked through an injected `run` closure. They do not
simulate Slint or user pointer events, and they do not touch the real
helper, bus, display, apps, or history. A successful dispatch does
not prove focus or activation; no desktop/live behavior is verified
here, and the tests install and publish nothing.

## Theme source

Reads EWW theme variables from:

- `$XDG_CONFIG_HOME/eww/theme-colors.scss` (default `~/.config/eww/theme-colors.scss`)

The shared `smpl_common::theme` watcher applies the initial theme and changes
every two seconds. Invalid/unreadable updates keep the last good palette.
Notification history polling continues independently.

Supported variables:

- `$theme-bg`
- `$theme-fg`
- `$theme-fg-dim`
- `$theme-accent`
- `$theme-bg-light`
- `$theme-bg-lighter`
- `$theme-danger`
- `$theme-success`
- `$theme-warning`
- `$theme-info`
- `$theme-popup-opacity`

Popup opacity defaults to 1 independently of application-background opacity.
Only the window background uses that opacity. Notification and help cards use
low-alpha background tints; text/icons stay opaque. Help hides the covered list
while keeping header controls and keyboard navigation available.
