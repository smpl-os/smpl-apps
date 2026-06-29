# Changelog

All notable changes to smpl-apps are documented here.

---

## Unreleased

### Added

- **start-menu: fuzzy search matching.** Search now uses a Smith–Waterman
  fuzzy matcher (`nucleo-matcher`, the engine behind Helix and Zellij) on
  top of the existing frecency ranking. Typos and acronyms like `vs` →
  `Visual Studio Code`, `firfox` → `Firefox`, `dol` → `Dolphin` now find
  their target. Results are still sorted by frecency first, then fuzzy
  match quality, then name — so your most-used app stays on top even when
  the query also matches other things.

- **settings: Wi-Fi network detail page.** Tapping a network now opens a
  dedicated detail page with the Connect/Forget/Share-QR controls, replacing
  the inline-expand row. The list view stays clean; the detail view has
  room for richer per-network status (signal, security, saved-state).

### Fixed

- **app-center: Installed-tab updates now actually work.** "Update Selected"
  upgrades only the chosen apps via `pacman -Sy --needed <pkg>` (and
  `flatpak update`), instead of a full `-Syu` that fails when the pinned hypr
  stack can't satisfy newer sonames. A pkexec dialog prompts for the password
  (same as OS update). Adds an internet check, fast mirror refresh
  (`reflector`, 12s cap), a live char-wrapped log box with Copy Log, an
  Unselect All button, and writes the run log to `/tmp/app-center-update.log`.

- **app-center: prevent partial-upgrade breakage on install.** Both repo
  (`pacman`) and AUR install paths now run `-Syu --noconfirm <pkg>` instead
  of `-S --noconfirm <pkg>`. Installing a single package against an
  out-of-date system can pull in a binary that links a newer shared-library
  soname than what's installed (e.g. Blender 5.1 needs `libopenjph.so.0.28`
  but the frozen ISO offline mirror still has `0.27`); the install reports
  success, the app appears in the start menu, but clicking it silently does
  nothing because the binary aborts on a missing library. `-Syu` keeps the
  whole system consistent so this can't happen. First install of a session
  is now slower (because the full system syncs first) but subsequent
  installs are normal speed.

- **settings: Wi-Fi connect now reports real errors.** `nmcli connect` now
  runs with `-w 45` (matching nmcli's own internal timeout) and a closed
  stdin so it can never hang waiting for terminal input. Failures now
  surface the actual stderr message (e.g. "Secrets were required, but not
  provided", "No network with SSID 'foo' found") instead of just an opaque
  exit code. Applied to both WPA/WPA2 (`connect`) and open networks
  (`connect_open`).

### Earlier in v0.7.3

- **start-menu: frecency-ranked search results.** The menu tracks how often
  and how recently you launch each app; search results are sorted by a
  frecency score (`count × 0.5^(days_since_last_use / 14)`) before match
  quality and alphabetical tiebreakers. Typing `code` and pressing Enter
  launches your most-used "code" app. State is persisted as TSV at
  `$XDG_STATE_HOME/smplos/app-usage.tsv` (defaults to
  `~/.local/state/smplos/app-usage.tsv`); delete the file to reset.

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
