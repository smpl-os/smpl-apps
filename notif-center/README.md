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
- Open: `gtk-launch <desktop_entry>` fallback to app name command
- Summary actions:
	- `System Update` → `smplos-update`
	- `Web App Launch Error` → open `~/.cache/smplos/launch-webapp.log` via `xdg-open`

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
