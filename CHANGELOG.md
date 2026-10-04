# Changelog

All notable changes to smpl-apps are documented here.

---

## Unreleased

### Added

- **start-menu: resident mode opens the menu in about 20 ms.** Each Super
  press used to start a new process, taking 260–300 ms to map, mostly for
  the system font scan and NVIDIA EGL initialization.
  `start-menu --resident [--hidden]` keeps one process instead. It publishes
  `$XDG_RUNTIME_DIR/smplos/start-menu.pid` (override:
  `SMPL_START_MENU_PIDFILE`), toggles on SIGUSR1, hides on SIGUSR2, shows on
  SIGRTMIN and exits cleanly on SIGTERM/SIGINT. A second `--resident` hands
  off to the running one. Closing, launching and the toolbar and power actions
  hide the menu and reset its view instead of exiting. Each show reloads a
  changed app index, pins and usage. `--hidden` preloads the window and fonts,
  so even the first show is fast. `start-menu --version` adds a
  `features: resident` line for the OS probe. Plain `start-menu` is
  unchanged. Takes effect with the matching smplOS toggle/autostart update.

### Fixed

- **settings: Dictation says when its model isn't downloaded and how to get it.**
  voxtype loads whisper.cpp model files from `~/.local/share/voxtype/models`.
  Setup and Reconfigure looked instead for smplOS's faster-whisper copies in the
  Hugging Face cache, treated them as "already cached" and skipped the download.
  The daemon then stopped with "Model not found", and Settings only said
  "Service is stopped". Settings now checks the file voxtype actually loads,
  including its GGML header. While that file is missing, Dictation says so and
  shows the source (huggingface.co/ggerganov/whisper.cpp), the size, where the
  file goes and the terminal command, with a **Download model** button.
  Reconfigure names the selected model when it still needs downloading.
  Downloads run `voxtype setup --download --model <model>`, move invalid files
  aside, treat a saved HTTP error page as a failure and explain how to retry.
  Large Turbo is listed as ~1.6 GB instead of ~3 GB.

- **settings: Display always shows, saves and verifies the real configuration.**
  Selecting a rotated display could show "Landscape" because the controls kept
  separate copies of the selected display's values that some paths never
  updated, and controls dropped their bindings after the first interaction.
  Controls now read the selected row of the verified model (selection follows
  the connector across reloads), and transforms 0–7 are kept exactly. Apply no
  longer re-packs displays into one row: the canvas shows the normalized layout
  that is saved. It refuses stale edits, rewrites only the managed lines of
  `$HOME/.config/hypr/monitors.conf` (description selectors, carried options,
  backups, atomic replace), reloads Hyprland off the UI thread and reports
  "Applied and verified" only when Hyprland matches. Lua-config dispatches use
  `hl.dsp.*` and require an `ok` reply. Settings shows saved/active drift with
  Use saved / Keep current, follows external changes, snaps scales to values
  Hyprland keeps, identifies displays on the right screen, marks the display
  showing workspace 1 as primary, and never shows invented displays outside
  Hyprland. Disabled, virtual and mirroring outputs are listed, not written.
  Requires the matching smplOS monitors loader update.

- **Native GUI transparency preserves opaque foregrounds.** Settings and all
  other owned GUI apps share validated palette loading and last-good theme
  refresh. Regular apps use `app_background_opacity`, with legacy popup opacity
  compatibility; popup apps retain their popup role. Translucent background
  paints replace opaque content sheets, disabled controls use opaque semantic
  colors, and fullscreen Help views avoid duplicate background layers. Hint
  overlays remain transparent outside themed badges and toasts. FemtoVG,
  undecorated windows, and existing app IDs are unchanged.

- **start-menu: User icons take precedence over system icons.** Icon roots are
  searched user-first before considering size/format preferences, honoring
  XDG data directories, legacy `~/.icons`, Flatpak exports and local icons.
  Absolute paths and the first-letter fallback remain supported.

- **start-menu: Quoting-only Exec changes no longer lose pinned metadata or
  icons.** Literal argv equivalence is shared by metadata lookup, icon
  preloading, pin indicators and toggling. Shell constructs retain conservative
  exact matching, and startup never rewrites or reorders the pin file.

- **settings: Search now tolerates typos and adjacent-letter transpositions.**
  The previous `fuzzy_match` was a pure left-to-right subsequence check, so
  `pwoer` failed against `Power` (the `o` has to come *after* the `w` in the
  target, but the query had them swapped). The matcher now runs three
  strategies in order: (1) case-insensitive substring, (2) subsequence (the
  old behavior — kept so abbreviations like `ppr` still hit `Power Profile`),
  and (3) Damerau–Levenshtein edit distance against each word in the label,
  capped at `1 + query.len()/4` edits. That last strategy handles the reported
  case (`pwoer` → `Power`, distance 1 via one adjacent transposition) plus
  substitutions (`poqer`), deletions (`powr`), and insertions (`poweer`).
  The threshold scales with query length so noise like `wifi` never matches
  `Power`. Covered by 8 unit tests in `settings::fuzzy_tests`.

---

## v0.8.14

### Fixed

- **calendar: "Details" button now actually opens the full calendar view.**
  The previous implementation flipped the layout to details mode and then
  resized the compact popup in place via
  `hyprctl dispatch resizewindowpixel/movewindowpixel …,class:…`. Hyprland
  0.55's Lua-config parser routes `hyprctl dispatch <legacy args>` through
  the Lua parser too, and rejects the positional `exact W H,class:…` syntax
  with `')' expected near 'exact'`. Result: `is-details` flipped but the
  window stayed at 250×390 → the 1100×680 details view had no room and
  looked broken/invisible. The Details button now spawns a **separate**
  window (`smpl-calendar --details`, Wayland app_id
  `smpl-calendar-details`) that Hyprland places centered, floating and
  freely resizable via a dedicated windowrule. No in-place resize, no
  dispatcher fragility. The compact popup dismisses itself after spawning.
  Requires the matching smplOS windowrule (`smpl-calendar-details`) which
  ships in smplos v0.8.34.

### Added

- **app-center: Update OS button now logs every invocation.** Presses used
  to `let _ = Command::spawn()` — every possible failure (binary missing,
  exec denied, ENOENT) was swallowed with no trail. Now writes
  `~/.local/share/smplos/logs/update-<unix>.log` on every press with spawn
  status, pid or error message. Makes "did the button work?" auditable.

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

- **settings: display rotation/scale now sticks.** Applying a monitor change in
  Settings → Display previously used a live `hyprctl keyword monitor` batch,
  which Hyprland can silently drop when a rotation is bundled with a
  repositioning — so portrait mode "applied" but never took effect. Apply now
  writes `monitors.conf` first, then re-sources it via `hyprctl reload` (the
  same mechanism keybindings/layout/idle already use), so the persisted
  transform/scale is authoritative and survives.

- **settings: idle DPMS (sleep) uses the current dispatch syntax.** The idle
  settings writer emitted the legacy `hyprctl dispatch dpms off/on`, which the
  pinned Hyprland no longer honors, so changing idle timeouts broke
  screen-off/resume. It now writes `hyprctl dispatch "hl.dsp.dpms({state=...})"`,
  matching the shipped `hypridle.conf` and `lock-screen`.

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
