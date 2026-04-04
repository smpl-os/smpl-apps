# Changelog

All notable changes to smpl-apps are documented here.

---

## v0.7.1 — 2026-04-04

### Fixed

- **settings: idle shutdown now respects user activity.** Previously,
  `schedule_shutdown()` fired a hard `shutdown -h +N` timer that would kill
  the session even if the user was actively typing. Shutdown is now handled
  by hypridle as a fourth listener tier (after lock, DPMS-off, suspend),
  so any keyboard or mouse activity resets the countdown.

- **settings: keyboard layout variants are validated before writing.**
  Added `validate_layout_variants()` which checks each layout:variant pair
  against available XKB layouts and rehomes orphaned variants. Added an XKB
  compile check via `xkbcli compile-keymap` before writing `input.conf`,
  preventing invalid configs from being written.

### Added

- **start-menu: Sleep option in power menu.** A new "Sleep" button sits
  between Lock and Restart, running `systemctl suspend`. Keyboard
  navigation indices updated accordingly.

---

## v0.7.0 — 2026-04-03

### Fixed

- **start-menu: Enter key now launches the top search result.**
  The search `FocusScope` intercepts all key events to handle arrow-key
  navigation, but had no `Key.Return` handler — pressing Enter while typing
  silently did nothing. Fixed by adding an explicit `Key.Return` case that
  calls `launch-app(selected-app)`, matching Windows/KDE/GNOME launcher
  behaviour.

- **settings: "Airplane Mode" and other WiFi/Bluetooth keywords added to
  search index.** `settings_search_index()` was missing "Airplane Mode",
  "Discoverable", and several other card-level keywords. Typing them in the
  start-menu search found nothing. All WiFi and Bluetooth card keywords are
  now present.

- **deploy-local.sh: calls `rebuild-app-cache` after exporting the settings
  search index.** Previously, `settings --export-index` wrote
  `~/.cache/smplos/settings_index` but `deploy-local.sh` never called
  `rebuild-app-cache` to merge it into `app_index` — the file start-menu
  actually reads. Settings keywords were therefore never searchable on
  freshly deployed machines.

### CI guardrails added

To prevent the above regressions from returning silently:

- `start-menu/ui/main.slint` must contain `Key.Return && root.is-searching`
  (Enter-key handler in search FocusScope).
- `settings/src/main.rs` must contain `"Airplane Mode"`, `"Wi-Fi"`, and
  `"Bluetooth"` in the search index.
- `deploy-local.sh` must contain `rebuild-app-cache`.

---

## v0.3.24 — 2026-03-XX

- fix(settings): move all WiFi/BT blocking calls off the main thread
- fix(bluetooth): add 4s timeout to bluetoothctl to prevent hang
- settings: add Bluetooth tab, fix airplane mode toggle
- settings: add Wi-Fi tab UI, WiFi backend, QR code support, expanded taskbar

## v0.3.23

- fix(start-menu): restore arrow-key navigation from search box

## v0.3.22

- fix: keyboard layout dropdown out-of-bounds crash

## v0.3.21

- fix(start-menu): splitn(5) so 5th field search_only is actually parsed

## v0.3.20

- fix(start-menu): settings browse shows only tabs+smpl apps, card keywords searchable

## v0.3.19

- fix(webapp-center): restore keybinding UI + fix slug parsing, missing flags, focus steal, regression guards

## v0.3.18

- fix: restore keybindings.rs deleted by rsync sync — smpl-common and settings stubs

## v0.3.17

- chore: sync from smplos, bump version
