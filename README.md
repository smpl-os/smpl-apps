# smpl-apps

**Current workspace version: v0.8.27**

Rust GUI apps for [smplOS](https://github.com/smpl-os/smplos).

## What's new in v0.7.0

- **Start-menu Enter key works.** Pressing Enter while searching now launches
  the top result immediately. Previously the search FocusScope (needed for
  arrow-key navigation) had no Return handler so Enter silently did nothing.

- **"Airplane Mode", "WiFi", "Bluetooth" and other settings keywords are
  searchable.** The settings search index was missing several card-level
  keywords. Typing "airplane mode", "pair bluetooth", or "resolution" in the
  start-menu now finds the right settings card and highlights it when Settings
  opens.

- **Settings keywords survive redeployment.** `deploy-local.sh` now calls
  `rebuild-app-cache` after exporting the settings index, so all keywords are
  in the app_index that start-menu reads after every deploy.

- **CI guardrails** prevent the above three regressions from returning silently.
  See [CHANGELOG.md](CHANGELOG.md) for the full list.

All apps use [Slint](https://slint.dev) with the FemtoVG renderer + Winit/Wayland backend for composited transparency.

| App | Description |
|-----|-------------|
| `start-menu` | App launcher |
| `notif-center` | Notification center |
| `settings` | Settings panel |
| `app-center` | Package manager UI |
| `webapp-center` | Web-app manager |
| `sync-center` | File sync & backup |
| `smpl-calendar` | Compact calendar and detailed event view |
| `smpl-hints` / `smpl-hintsd` | Keyboard-navigation hint overlay and daemon |
| `control-surface` | Keypad daemon (`control-surfaced`) for CH552 macro keypads, its Kdenlive API plugin and the open keypad firmware; C++20/Qt6, see [control-surface/README.md](control-surface/README.md) |

The Rust apps use Slint. `control-surface` is a separate CMake project in
`control-surface/` (imported with its history; GPL-2.0-or-later, firmware
CC BY-SA 3.0, see `control-surface/firmware/README.md`). Settings > Keypad
talks to it over D-Bus (`org.smplos.ControlSurface1`).

## Building

```bash
cargo build --release --workspace
```

Requires Arch Linux (or equivalent) with: `fontconfig freetype2 libxkbcommon wayland gtk4 gtk4-layer-shell libadwaita`

control-surface (needs `cmake ninja qt6-base systemd-libs dbus`):

```bash
cmake -S control-surface -B target/control-surface -G Ninja -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON
cmake --build target/control-surface
# tst_uinput creates a real virtual keyboard on this host; CI skips it.
ctest --test-dir target/control-surface --output-on-failure -E '^uinput$'
```

Run the tests as a regular user (CI uses an unprivileged account): as root,
the checks that a file can't be written would pass.

Releases build control-surface on its runtime baseline, not on rolling Arch:
it links the system Qt, and a Qt program needs the Qt minor it was built
with or later. The supported baseline is **Qt 6.11 (smplOS's qt6-base
6.11.2) and glibc 2.44**. The release workflow's `control-surface` job pins
the Arch Linux Archive snapshot `2026/09/30` (qt6-base 6.11.2-3, glibc 2.44,
gcc 16.2.1), builds and tests there, then runs the packaged
`control-surfaced features --json` and `ch552-padprog --help` as a regular
user. `scripts/release_assets.py control-surface-check ASSET` rejects an
asset that needs a newer Qt or glibc, has no Qt version tag, uses Qt private
API or carries an RPATH; `control-surface-smoke ASSET` runs the binaries on
the current system. Raise the snapshot, `CONTROL_SURFACE_QT_PACKAGE` and
`CONTROL_SURFACE_MAX_QT` together, only once every supported smplOS has the
newer Qt.

`./check.sh` runs this too when CMake and Qt6 are installed; CI always does.

## Native themes and transparency

All GUI apps share the palette reader in `smpl-common::theme`. It reads
`$XDG_CONFIG_HOME/eww/theme-colors.scss` when the config directory is absolute,
falling back to the existing smplOS deployment at
`$HOME/.config/eww/theme-colors.scss` only when the preferred file is absent.
Without an absolute XDG directory it uses the HOME path directly. Every poll
rechecks the preferred path, so creating/removing that file switches sources;
an invalid or unreadable preferred file never silently selects the fallback.
Theme changes are polled every
two seconds; unchanged palettes are not reapplied. Atomic replacement is
supported. Read errors, incomplete palettes, invalid colors, and invalid
opacity values are reported to stderr without discarding the last good theme.
At startup, an unavailable or invalid theme uses an opaque built-in palette
until a valid theme arrives.

| GUI surface / Wayland app ID | Background opacity source |
|---|---|
| `settings`, `app-center`, `webapp-center`, `sync-center` | `$theme-app-background-opacity`, then legacy `$theme-popup-opacity`, then `1.0` |
| `start-menu`, `notif-center`, `smpl-calendar`, `smpl-calendar-details` | `$theme-popup-opacity`, then `1.0` |
| `hints-overlay` | Popup opacity for painted badges/toasts only; the fullscreen root stays transparent |

The `sync-center-gui` binary uses app ID `sync-center`; both calendar modes
come from `smpl-calendar`. The sync, calendar-alert, and hints daemons do not
add GUI app IDs. Compositor rules must keep these self-managed-alpha surfaces
at window opacity `1.0`, including inactive windows.

Opacity values must be finite numbers in `[0, 1]`. An explicitly invalid
value rejects the update rather than falling through to a different key.
Palette colors are opaque six-digit `#RRGGBB`; `bg`, `fg`, and `accent` are
required. Muted and disabled foregrounds use opaque semantic colors, not
ancestor opacity. The window paints the base alpha once; interior cards use
light background-only tints rather than stacked opaque sheets. Help views
replace covered content without another full-window fill. Menus and modal
panels may retain opaque readability backgrounds. Setting the base opacity
to `1.0` restores a solid surface; transparent space outside hint badges
remains transparent.

Do not replace the shared FemtoVG backend, enable software/Skia rendering,
add a second backend builder, or remove the no-decoration/app-ID setup in
`smpl_common::init`. Window background alpha is intentional and does not
fade its descendants. Intrinsic image transparency and glyph antialiasing
are normal; solid text/icon interiors must not inherit a window/control fade.

Focused headless checks:
`cargo test -p smpl-common --lib` and
`cargo test -p settings --bin settings ui_contract_tests`.
These cover parser/reload and UI structure, not compositor pixel output.
Native visual checks must use isolated fixtures rather than capturing the
user's desktop or running application actions against real data.

The optional native fixture reads RGBA from FemtoVG's actual default OpenGL
framebuffer on an authenticated private Xvfb display. It checks the opacity
round trip, opaque glyph/icon interiors, normal antialias edges, native focus
loss/regain, and the transparent hint-overlay case:

```bash
cargo build -p smpl-common --example femtovg_alpha_probe
python tests/run_femtovg_alpha_probe.py
```

This requires Xvfb and Mesa OpenGL support. `LIBGL_ALWAYS_SOFTWARE=1` selects
Mesa's CPU OpenGL driver for the fixture, **not** Slint's software renderer;
the renderer remains FemtoVG. Logs are written under `target/native-alpha-probe/`.
This is native framebuffer evidence, not proof of Wayland compositor blur,
compositor rules, or every live application layout.

Ship the matching smplOS palette-generator/compositor integration before the
app release so the new application token and all self-managed-alpha IDs are
available. Legacy themes without the new key remain compatible. Source commits
alone do not update installed binaries; use the normal release workflow.

## Calendar clocks and forecast

The calendar popup prefers 364 by 650 logical pixels, clamps its height on
shorter monitors, and scrolls its content rather than clipping controls.
Above the existing month
grid and events are a seven-day forecast and one to three LCD-style clocks
with seconds. The outline expand button still opens the standalone detailed
calendar; its hover/focus tooltip and accessible name identify the action.
The adjacent sliders button opens **Clock & weather** preferences.
Dates with events have a foreground outline around their day number instead
of a dot, in both the compact/sidebar calendar and Details. Selected dates
retain their filled highlight inside the event ring; today remains distinct.
The six-week grid includes event presence on neighboring-month dates and
every date overlapped by an event, excluding an end at exactly midnight.
Recurring occurrences are counted separately; overlapping month queries
do not duplicate them. Neighboring dates retain their navigate-to-month action.
The detailed window has a compact outlined inward-arrows button, with a
**Back to calendar** tooltip, accessible name and keyboard activation.
It focuses an existing compact popup on Hyprland
or launches the same executable in compact mode, then closes only the detailed
process after the popup is ready. A per-display instance lock prevents duplicate
compact processes. An open event editor must be saved or explicitly cancelled
before returning; navigation errors leave details open with a visible message.
Focus supports Lua-configured Hyprland and the legacy dispatcher, without
mistaking a successful child launch for a successful window handoff.

Details has a separate outline **Maximize/Restore** button and native edge
resizing. Its initial size is still 1100 by 700; the compact popup is unchanged.
On Hyprland, maximize state comes from the compositor's actual window layout,
not its sometimes-stale client maximized flag. Actions target only the current
process's window; other compositors use the native Winit controls.
The month grid now retains the full event list and derives visible row capacity
from each cell's available height. Enlarging the window reveals more events
without enlarging fonts. Smaller windows scroll the grid; overflow still opens
the selected day's panel. The existing views are the month grid and that
day panel; this does not add separate week, year or day-calendar modes.

Today's compact agenda separates all-day events from timed events, inserts
a labelled current-time divider, and highlights every ongoing timed event
with a theme-aware outline and a **Now** badge. This is an agenda, not a
proportional time grid. It scrolls to now when opened or when today is selected;
minute updates retain manual scrolling. Other dates have no current-time cue.

The event editor uses native Slint popup ownership for its date, time, repeat
and reminder menus, so they render and receive input above sibling controls.
The form body scrolls on short displays. Time options use five-minute steps;
existing off-grid values are preserved until explicitly changed.
Time dropdowns also support **type to jump**: `21`, `2130` and `21:30`
highlight matching options; `9`, `09`, `930` and `9:30` accept leading-zero
variants. This navigates the five-minute list, not a free-form time editor.
The popup shows the typed prefix and candidate. Enter or clicking commits;
typing alone does not change the event or auto-adjust its end. Off-grid or
invalid input has no selectable match rather than being rounded. Backspace
edits the prefix, a pause longer than 1.5 seconds starts a new prefix, and
reopening/switching fields resets it. Arrow/Home/End keys navigate;
Escape or Tab dismiss without committing, and Tab moves to the next field.
Changing the start date/time or switching to a timed event moves an end less
than twenty minutes later to exactly twenty minutes later. A later end is
preserved. Timed events have an explicit end date, including automatic
next-day rollover. Opening an existing event does not adjust its interval;
manually chosen positive durations shorter than twenty minutes remain valid.
Equal/reversed intervals and invalid or unresolved DST-local times produce
an inline error rather than silently substituting another timestamp.
Create and update also validate timed intervals before writing SQLite.

Before setup the system-local clock works without a network connection.
Search for a city explicitly in preferences, select a result, and add up to
three cities. The **Primary** city supplies the forecast; **Use** changes it.
Remove cities independently and choose Celsius or Fahrenheit. City searches
and forecasts use Open-Meteo, not IP-based geolocation. Saved cities include
their IANA timezones; clocks use timezone/DST rules and show each city's date.
No city or forecast is invented when the service is unavailable.

City search keeps exact names first and can suggest close spellings for names
of at least five letters, including omitted/repeated letters and adjacent
swaps: `Lafayete`, `Laffayette`, `Lafyaette`, `Bengalru`,
`Thiruvanathapuram`, and `Puduchery`. Latin accents, case, spaces and hyphens
are normalized for comparison (for example `Saint Étiene`).
Open-Meteo itself uses prefix matching, so the app retrieves additional real
records using at most two narrower/broader prefixes, then filters by edit
distance. It does not manufacture corrected locations. Suggestions retain
the provider's region, country, timezone and coordinates and require **Add**.
Ambiguous duplicate labels also show coordinates for explicit selection.
An optional qualifier such as `Lafayete, Louisiana` is preserved exactly.

This is deliberately bounded, not universal spell correction: errors in the
first few letters or obscure places outside the provider's first 100 prefix
results can still return no close match. Check the first letters or add a
country/region in that case. Each explicit search makes at most three requests
within one ten-second deadline, returns at most ten places, and accepts at
most 100 characters. Starts are limited to one every five seconds; there is
only one search in flight. Editing the query or leaving preferences cancels
remaining fallback requests and discards stale results; an in-flight HTTP
request can take until its deadline to finish. Offline/errors remain visible,
including when a broader search fails after returning initial results.

Preferences live in `$XDG_CONFIG_HOME/smplos/calendar.json` (normally
`~/.config/smplos/calendar.json`); the forecast cache is in
`$XDG_CACHE_HOME/smplos/calendar-weather.json`. Preferences and cache use
atomic replacement. They are separate from the existing calendar events
database, which is not migrated or replaced by this feature.
Matching cached weather is populated before the first frame. If it is less
than one hour old, opening the popup makes no forecast request and schedules
refresh at that cache's original one-hour expiry, not an hour after opening.
Older usable data stays visible while it is refreshed in the background;
failed refreshes keep that data with a subtle truthful stale/offline status.
Loading appears only when there is no applicable forecast, such as the first
launch or a new primary city with no matching cache.

Weather requests run off the UI thread with ten-second timeouts, a one-hour
refresh interval and five-minute retries after failure. Rapid primary-city/unit
changes coalesce behind a single forecast worker. Search has a separate bounded
worker and only runs on an explicit Search action.
The popup distinguishes loading, unconfigured, unavailable and cached/stale
forecasts. Precipitation percentages are daily maximum **chance**, not
precipitation volume. The forecast is network-backed; saved clocks,
month navigation, events and bundled graphics/fonts remain usable offline.
Cache identity includes coordinates, location label, IANA timezone and units;
data from another selection is never displayed. The previous cache format
without timezone identity is refetched once; saved cities/events are unchanged.
Cached forecasts at least one hour old are marked stale; caches older than
seven days are rejected. Past forecast dates are removed, and a remaining
partial forecast is labelled rather than padded with invented data.

The original DSEG7 Modern Regular font is embedded from
[DSEG v0.46](https://github.com/keshikan/DSEG/releases/tag/v0.46), by keshikan,
under SIL OFL 1.1. Its unmodified license is in
`calendar/ui/assets/DSEG-LICENSE.txt` and is also available from the binary
with `smpl-calendar --font-license`. Outline SVGs are original project assets.
No runtime font/icon downloads or external asset installation are needed.
Glow and gradients tint only their own backgrounds/segments; the popup's
shared native theme alpha is still painted once, with opaque glyph interiors.

Focused checks:

```bash
cargo test -p smpl-calendar --bin smpl-calendar
python -m unittest discover -s calendar/tests -v
cargo build -p smpl-calendar --example dashboard_probe
python calendar/tests/run_dashboard_probe.py
cargo build -p smpl-calendar --example agenda_probe
python calendar/tests/run_agenda_probe.py
cargo build -p smpl-calendar --example editor_probe
python calendar/tests/run_dashboard_probe.py --editor
python calendar/tests/run_dashboard_probe.py --typeahead
python calendar/tests/run_dashboard_probe.py --rings
cargo build -p smpl-calendar --example details_probe
python calendar/tests/run_details_probe.py
```

The dashboard fixture renders synthetic 1/2/3-clock, weather, preferences and
light/dark fixtures on an authenticated private Xvfb using actual FemtoVG.
It never opens the user's events database or captures their desktop.
Screenshots are written under `target/calendar-probe/`. This is native
rendering evidence, not proof of compositor blur.
The agenda fixture checks small-screen current-time scrolling and preservation
of manually chosen offsets across model/minute updates.
The editor fixture selects time/date/repeat options across overlapping controls
using native pointer events, exercises keyboard selection and dismissal,
checks invalid/overnight intervals and edit roundtrips in a private SQLite
database, and verifies scrolling and controls in a 364-by-360 window.

With explicit permission to briefly open fixture windows on the current
Hyprland desktop, run
`python calendar/tests/run_wayland_navigation_probe.py --allow-host-window-fixture`.
This uses private data, separate fixture app IDs and temporary floating rules;
it does not capture the desktop or operate on production calendar windows.
It verifies details window disappearance and process exit after both launch
and repeated focus, exactly one compact window, and a mapped details window
with an error on failure. It cleans up its exact PIDs, disables its rules and
restores the previous focus when that window still exists.
Add `--editor` to this opt-in Wayland command to run the editor interactions
on the actual compositor. Its short-window check resizes only the fixture's
exact owned window; production calendar windows and editors are not touched.
Use `--typeahead` instead for actual Wayland typed-time navigation, explicit
commit, no-match handling, keyboard focus transfer and scroll-recovery checks.
Use `--details` to check real event-row hit targets and overflow at different
compositor sizes, including maximize and restoration of the previous size.

For a calendar-only local test deployment, build
`cargo build --release -p smpl-calendar --bin smpl-calendar`, then run
`bash scripts/deploy-calendar-local.sh`. It requests graphical authentication
through `pkexec`, verifies a timestamped binary backup under
`/var/backups/smpl-calendar/`, and atomically replaces only
`/usr/local/bin/smpl-calendar`. Embedded assets travel with that binary.
It does not kill any process, remove PATH shadows, deploy other apps, or touch
events/preferences. Close an existing popup and reopen from the taskbar (or
run `/usr/local/bin/smpl-calendar`) to use the installed build.

### Calendar reminders

In New/Edit Event, choose **Reminder** (15 or 30 minutes, or 1, 8 or 24 hours
before), optionally choose **Repeat**, and save. Reminders are desktop
notifications: the calendar does not request an audible alarm, snooze controls
or notification action buttons. The enabled `smpl-calendar-alertd.service`
starts with the user session and keeps working when the calendar window is
closed. It does not wake a sleeping computer.

The daemon checks about every 30 seconds. Startup, resume and failed-delivery
retries use the same inclusive window: reminders due between **now minus five
minutes and now**. Nothing fires early; older missed reminders are skipped.
Delivery failures/timeouts are not recorded as sent and can retry within that
window. Successful receipts are persisted per occurrence and schedule revision,
including across daemon restarts. Existing legacy receipts remain valid.

Daily/weekly repeats retain local wall-clock start time across DST. Generated
nonexistent spring-gap times are skipped; repeated autumn times use the earlier
offset once. Monthly/yearly dates clamp to the month's last day while preserving
the original anchor, so a 31st does not permanently drift to the 28th.
Timed repeats retain elapsed duration; all-day repeats retain their local dates
and end wall time. Stored recurrence end dates include occurrences starting on
that date. Editing/deleting a recurring item changes the series; there is no
individual-occurrence exception editor.

For a backed-up local update of **both** calendar binaries and the login service:

```bash
cargo build --release -p smpl-calendar --bins
bash scripts/deploy-calendar-local.sh --with-reminders
```

This snapshots the live SQLite database through its backup API, checks integrity,
and stores it with private permissions under
`$XDG_STATE_HOME/smplos/calendar/backups/` (normally `~/.local/state/...`).
It uses `pkexec` to verify and atomically replace both binaries, backs up prior
binaries/unit under `/var/backups/smpl-calendar/`, installs the tracked unit from
`calendar/systemd/`, then terminates only revalidated old daemon identities and
enables/starts one user service. Calendar windows/editors are not terminated.
Existing masks, disabled units and custom unit definitions are surfaced rather
than silently overridden. No existing receipt history is bulk-deleted: schema
changes are additive, with targeted invalidation only when a user changes or
deletes a schedule. A second verified snapshot precedes service initialization.
The general `deploy-local.sh` routes these two binaries through this same path.

Check status with `systemctl --user status smpl-calendar-alertd.service`.
`smpl-calendar-alertd --version` and `--check` are read-only in this version;
do not pass them to an older daemon that may ignore arguments. Only the explicit
`--foreground` service entry enters the database runtime. Ordinary launches,
including launches from an old calendar window, cannot create unmanaged copies
or override a stopped/masked service.

Focused scheduler policy tests:
`cargo test -p smpl-calendar --bin smpl-calendar-alertd` and
`cargo test -p smpl-calendar --test reminder_policy policy_`.
They use synthetic databases and injected clocks, not real events or desktop
notifications.

## Start-menu icons and pins

Named icons are resolved **root-first**, so a user icon wins even when a system
icon has a more preferred size or format. The search order is:

1. `$XDG_DATA_HOME/icons` (default `~/.local/share/icons`), then legacy `~/.icons`.
2. User Flatpak exports under `$XDG_DATA_HOME/flatpak/exports/share/icons`, then user pixmaps.
3. Each `$XDG_DATA_DIRS` entry's icons and pixmaps in order (default `/usr/local/share:/usr/share`), then system Flatpak exports if not already searched.

XDG data paths must be absolute; empty variables use their defaults. Each icon
root prefers hicolor, then other installed themes in deterministic name order,
then unthemed icons. This is not an active desktop-theme/inheritance selector.
Within a theme, scalable and common menu sizes are preferred, with SVG before
PNG in each directory. Explicit SVG/PNG filenames, local/AppImage icons,
Flatpak exports and valid absolute paths are supported. Missing or undecodable
images retain the menu's first-letter fallback.

Pins remain literal commands in `~/.config/smplos/pinned-apps.txt`, in their
existing order. Metadata, icon preloading and pin toggles also recognize
equivalent quoting of **simple literal argv** (for example, `/path/app` and
`"/path/app"`). Argument boundaries, escapes and empty arguments are preserved.
Commands with shell operators, expansion, assignments or reserved words use
exact matching; comparisons never invoke a shell. Startup does not migrate or
rewrite pins, and unknown custom commands are not removed.

The cache contract remains `~/.cache/smplos/app_index` with
`Name;Exec;Category;Icon[;1]` rows; this change does not migrate cache paths.
Focused, headless regression checks: `cargo test -p start-menu --bin start-menu`.
Icon fixtures use temporary directories, not the user's HOME.

## Start-menu resident mode

Plain `start-menu` is unchanged: it shows the menu and exits on close or
launch. `start-menu --resident` keeps one process alive, so later opens skip
process startup, the system font scan and NVIDIA EGL initialization.

| Command | Effect |
|---|---|
| `start-menu --resident` | Become the resident and show; if one is running, ask it to show and exit 0. |
| `start-menu --resident --hidden` | Become the resident without showing (login preload); if one is running, exit 0. |
| `start-menu --version` | Prints `start-menu vX.Y.Z`, then `features: resident`. |

Probe support with `--version` only: older builds print just the first line
and open the menu for any other argument.

Once it can handle signals, the resident writes its PID atomically (temp file
and rename) to
`${SMPL_START_MENU_PIDFILE:-$XDG_RUNTIME_DIR/smplos/start-menu.pid}`, using
`/run/user/<uid>` when `XDG_RUNTIME_DIR` is unset or empty and creating a
missing directory with mode 0700. It removes the file as soon as SIGTERM or
SIGINT arrives (or on any other exit), and only while the file still holds its
PID. A running resident is a live process named there whose `/proc/<pid>/comm`
is `start-menu`; stale, dead, zombie and other PIDs are ignored. `flock` on
the neighbouring `start-menu.pid.lock` serializes startup: a launch racing a
resident that is still starting or already exiting waits up to 5 s on the lock
or pidfile, then hands off or becomes the next resident, never a second one.
If the pidfile cannot be written, `--hidden` exits 1 and a shown menu falls
back to exiting on close, like plain `start-menu`. That menu still handles the
signals below (for example `pkill -x start-menu`), but keeps the startup lock
until it exits, so other `--resident` launches meanwhile wait 5 s and fail.

| Signal | Effect |
|---|---|
| `SIGUSR1` | Toggle: show if hidden, hide if shown. |
| `SIGUSR2` | Hide (click-outside). Idempotent; never shows. |
| `SIGRTMIN` (`kill -s RTMIN`) | Show. Idempotent; never hides. Used for the single-instance handoff. |
| `SIGTERM`, `SIGINT` | Clean exit; a wedged event loop is force-exited after 2 s. |

Signals are blocked in every thread, read by one thread and handled on the UI
event loop. Every process the menu starts (apps, actions, `rebuild-app-cache`)
gets an empty signal mask and the default action for these six signals, even
if the menu itself was started with SIGINT ignored, as a script's background
jobs are. Without this, `std::process::Command` would pass the blocked mask
on, and launched apps would ignore Ctrl+C and SIGTERM. `signals::command` is
the only way to build such a process in start-menu; `start-menu/clippy.toml`
rejects `std::process::Command::new` elsewhere.

While the compositor sends no frame callbacks (outputs off or being
reconfigured), a pending vsync'd frame blocks the UI loop; queued requests run
once rendering resumes. If a toggle, show or hide request stays unhandled for
20 s, the resident exits so the next launch replaces it. Reaping exited apps
never counts toward that limit, and termination has its own 2 s limit.
Otherwise a resident never exits on its own: Escape, a compositor close,
launching an app or pin, Web App Center, Sync Center and power actions do
their work and then hide. Hiding resets the view to its startup state: no
search or category, empty list, nothing selected, menus closed, list at the
top; hover and held-modifier state are cleared too. Each show re-reads
`~/.cache/smplos/app_index` if its size, mtime or inode changed, then rebuilds
categories and clears icon caches. A missing index keeps the last list. Pins
and usage reload the same way; the search field gets focus. The theme watcher
keeps running; hidden, the process only wakes for it every two seconds.

Wayland cannot unmap a window, so Slint destroys it on hide and recreates it on
show. Fonts and loaded GL libraries stay in the process, and the resident
reapplies the `start-menu` app ID to each new window. `--hidden` also creates
the unmapped window and loads the font collection during the preload, so the
first show is as fast as later ones. Launched apps are children of the
resident; it reaps them when they exit. They inherit its environment, so start
it once the session environment is complete. Under a systemd unit that stops
its whole cgroup, launched apps would stop with the menu.

On Hyprland 0.56.2 with the NVIDIA 615 open driver, a show took a median of
13–25 ms from signal to compositor map, depending on system load (window and
EGL context about 7 ms, first frame about 14 ms). A hide took about 2 ms. The
first show after `--hidden` took about 15 ms, versus about 180 ms without the
font preload. A cold `--resident` start took about 235 ms on an idle machine.
Hidden, it used no measurable CPU over 10 s. `SMPL_START_MENU_TRACE=1` prints
`CLOCK_MONOTONIC` timestamps for signals, show/hide and frames to stderr.

Headless checks: `cargo test -p start-menu --bin start-menu`. These cover the
pidfile and instance detection (temporary directories and a fake `/proc`),
reload decisions, signal mapping, view reset, version text, the stall
watchdog's accounting, and the signal state of launched processes (in a
separate test process).

## Releases

### Power preferences and OS rollout

Settings reads exact saved idle delays from
`$XDG_CONFIG_HOME/hypr/hypridle.conf` (normally
`~/.config/hypr/hypridle.conf`). A stock 330-second screen-off delay is shown
as `5 min 30 s` in the custom chip, not rounded to five minutes. Preset values
select their matching chip without an extra saved-value label. Changing one timer re-reads the
latest file and changes only that timer, preserving other values, comments,
general settings and listener properties. `Never` retains the listener as
reversible `# smpl-settings-disabled: ` comments. Manual `lock_cmd` remains
active even when every timer is Never; changing lock to Never also removes
recognized stock suspend/resume lock hooks.

This is a user-owned preferences file, not a generated compositor config.
Settings refuses ambiguous listeners, includes (`source`), variables,
unsupported syntax and custom timeout commands with a visible error rather
than dropping them or guessing. The smplOS updater preserves these custom
and source-included configurations even though the Settings UI cannot edit
them; this is not support for editing arbitrary Hypridle configurations.
Custom sleep hooks cannot be changed through
the lock timer. Symlinked/hardlinked configs require manual editing. Edits
are atomically replaced after a conflict check and isolated native Hypridle
parser validation. A non-cooperating editor can still race between the final
comparison and rename; do not edit the same file simultaneously in two tools.
The legacy `~/.config/smplos/power.conf` shutdown copy is no longer written
or used as a second source of truth.

Ship the **smplOS power-preservation update first**, then the Settings release.
That OS update must preserve user `hypridle.conf`, narrowly migrate known
generated commands, and install `smplos-hypr-dpms on|off` with a read-only
`--check`. Settings uses this helper for new/restored screen-off rules and
normalizes only recognized legacy/Lua DPMS commands when it is installed.
Without it, existing direct commands survive scoped timer edits, but creating
or restoring a screen-off rule requires the OS update. The helper probes the
running compositor's capabilities, rather than assuming its parser from the
installed package version, and adapts across the next login. Unknown future
syntax/capabilities must fail explicitly until a targeted migration/helper
update is provided. Normal compositor configurations still receive OS updates;
Hypridle's separate preferences grammar must not be injected into Hyprland.
Both repositories' changes are needed: older updaters can still overwrite
preferences and older Settings can still regenerate them.

The Power tab refreshes while open and reports actionable save, validation,
profile and restart errors in plain language. Successful operations and
routine daemon diagnostics stay in logs, not persistent status banners;
the UI never claims that a running daemon proves timers were applied.
Background polling leaves controls interactive. User selections are queued
in order ahead of further polling, with no optimistic saved-value changes;
repeated identical pending selections are coalesced.
Opening Settings never starts/restarts Hypridle or cancels shutdown jobs.
Only Hyprland sessions can edit idle timers. Changes attempt a systemd user
service restart, checking the session/configuration, new PID and stability;
unmanaged daemons are never killed or replaced. Successful restart is not
proof of active rules (Hypridle has no rule acknowledgement API). Application
failures retain the saved preferences and explain that timers could not be
activated; select the desired timeout again after resolving the error to retry.
Power profiles are confirmed by command status and readback, not optimistic
chip selection.

Headless checks (no real lock, suspend, shutdown or service actions):

```bash
cargo test -p settings --bin settings power
cargo check -p settings --bin settings
# Optional installed-hypridle parser check; isolated HOME/runtime/Wayland/D-Bus:
cargo test -p settings --bin settings power::tests::native_parser_isolated_from_real_session -- --ignored
```

### Settings readability

Settings uses a consistent sans-serif hierarchy: 14px body/control text,
13px secondary text, 12px compact annotations, and 16–18px headings.
It prefers Noto Sans when available and uses the platform fallback otherwise;
no additional font package is required. Compact controls are 32px tall and
two-line choices have additional room. The original 500×350 minimum and
900×560 preferred window sizes are unchanged. The sidebar scrolls at short
heights; narrow tabs expose a horizontal scroll thumb instead of shrinking
text or making controls unreachable. The Display page also scrolls as a whole.
Wi-Fi, Bluetooth, and Hints sidebar icons use matching theme-tinted outline
assets; connection/status glyphs elsewhere keep their existing meanings.

### Taskbar workspace preferences

Settings and the OS taskbar helpers share `~/.config/smplos/bar.conf` (the
canonical HOME-based path, not an app-specific XDG override). Workspace count
is a total across monitors, not a count per monitor. Automatic placement keeps
at least one workspace per connected monitor and retains occupied or visible
workspaces when the target is reduced. Settings never moves windows or switches
workspaces to enforce a count; the OS workspace controller reconciles it.

All Taskbar and clock changes use a shared, validated, atomic single-key writer,
preserving comments and unknown preferences. Reading settings never writes clock
defaults. Duplicate or invalid managed values and linked/nonregular files need
manual correction rather than a destructive rewrite. A final conflict check
protects against external edits, but a non-cooperating editor can still race the
comparison and rename. Avoid simultaneous edits in multiple tools.

Workspace changes run bounded `bar-ctl apply` commands off the UI thread.
Requests are serialized with clock changes; rapid slider choices coalesce to
the latest pending value. Controls display saved readback, refresh on tab entry
and after changes, and show actionable save/apply failures without routine
success banners. Publish the paired OS update first: it makes monitor-owned
workspace widgets honor numbers/squares and spacing, and makes `bar-ctl apply`
validate and report EWW failures. An older helper may conceal those failures.

Headless regression checks: `cargo test -p settings --bin settings taskbar`.

### Display settings

Settings → Display is a projection of verified Hyprland state. Rows come only
from `hyprctl -j monitors all`; enabled, physical, non-mirroring outputs are
drawn and written, while disabled, virtual (`HEADLESS-*`, `WL-*`, `FALLBACK`)
and mirroring outputs are named in the status line and never written. Every
control reads the selected row, so the canvas, mode, scale and orientation
always match what Hyprland reports. Mirrored transforms 4–7 appear as an extra,
exactly named orientation. Outside Hyprland, or when `hyprctl` fails, the tab
explains why it is unavailable; simulated displays need `settings --demo`.

Edits (drag, mode, scale, orientation) are normalized immediately so the canvas
shows exactly what will be saved: an overlapping or detached display moves to
the nearest position sharing an edge with the arrangement (on the side facing
it), and the arrangement is translated to 0,0. Displays that are already
attached never move, and edits that keep every size (a refresh rate, a 180°
turn) leave the layout alone; a loaded live layout is never altered. Scales
snap to values Hyprland keeps for the mode; it silently replaces others (1.5
becomes 1.6 on 2560×1440).

Apply runs off the UI thread. It refuses when the displays or the saved file
changed since editing began, rewrites `$HOME/.config/hypr/monitors.conf` (never
an `XDG_CONFIG_HOME` copy), stops the bar, reloads Hyprland, waits up to 3 s for
the result, restarts the bar, centers off-screen floating windows, reads config
errors and reloads the tab from the live state. It reports "Applied and
verified" only when every written field matches and otherwise lists each
difference.

The writer replaces only lines whose selector matches a written display,
carries their `vrr`, `bitdepth`, `cm`, `sdrbrightness`, `sdrsaturation` and
`icc` options and their `addreserved` lines, and keeps everything else verbatim:
comments, the catch-all, and lines for displays that are disconnected,
disabled, virtual or mirroring. Comments follow hyprlang exactly (`##` is a
literal `#` and the character after it is never a comment start), and carried
values are re-escaped so they read back unchanged. A display is keyed by `desc:<make model
serial>` when that description is unique, so a connector change (DP-3 becoming
DP-4) keeps its rule; otherwise by connector. Writes are atomic, preserve the
file mode, replace a symlink's target, skip byte-identical content and keep the
newest five `monitors.conf.bak-YYYYmmdd-HHMMSS` copies for the OS monitors
guard. The grammar and rule semantics (last matching rule wins; standalone
`transform` and `addreserved` forms) are shared with the smplOS Lua loader.
Publish the matching OS update first.

While the tab is open it checks for changes every two seconds. Without pending
edits it reloads automatically; with pending edits Apply stays blocked until
Refresh or Revert. When a display's saved rule differs from what is active, a
notice offers **Use saved** (reload the file) or **Keep current** (save the
active layout). "Primary" marks the display showing workspace 1. With the
workspace policy enabled (`workspace-ctl enabled`), placement follows the
arrangement and the button becomes a note; otherwise **Move workspace 1 here**
moves it now without saving. Dispatches use the running config provider's
syntax (Lua `hl.dsp.*` or legacy) and succeed only on an `ok` reply. Identify
shows each name on its own screen.

Headless regression checks (temporary directories and mocked command runners):
`cargo test -p settings --bin settings display`.

### Dictation models

Settings → Dictation configures voxtype, whose Whisper engine is whisper.cpp.
Each model is one GGML file, `ggml-<model>.bin`, in
`$XDG_DATA_HOME/voxtype/models` (normally `~/.local/share/voxtype/models`).
`voxtype setup --download --model <model>` fetches it from
`huggingface.co/ggerganov/whisper.cpp`. The download needs an internet
connection: Base is about 150 MB, Small 500 MB, Medium 1.5 GB and
Large Turbo 1.6 GB. After that, dictation runs offline. The smplOS image copies
faster-whisper (CTranslate2) models into `~/.cache/huggingface`. Those are a
different format, and voxtype doesn't use them.

Every two seconds, Settings checks the file voxtype loads for the configured
model. Only a file with the GGML header counts as downloaded. While the model is
missing, the page says so. It shows where the file comes from, its size, where
it goes and the terminal command, and it offers **Download model**. It also hides
the "Service is stopped" hint, because starting the service can't work without
a model. Reconfigure names the selected model when it still has to be
downloaded. The download runs in a terminal:

- An invalid existing file is moved aside to `*.invalid`; otherwise voxtype
  would skip the download.
- An HTTP error page saved in place of the model counts as a failed download.
- Every failure explains how to retry.

A model given as an absolute path or a custom `.bin` file name can't be
downloaded by voxtype, so the page shows the path voxtype looks for instead.
Settings doesn't detect a truncated file that still has a valid header; voxtype
reports it when it loads the model.

Headless checks, using temporary directories and a stub `voxtype`:
`cargo test -p settings --bin settings dictation`.

### Publishing

Pre-built binaries are published to [Releases](../../releases) and consumed by the smplOS ISO builder.
Run the **Release** workflow (`.github/workflows/release.yml`, `workflow_dispatch`)
from `main` after the changes are merged. It increments the workspace patch
version itself, commits the version/lockfile change, and publishes tag
`v<VERSION>`, bundle `smpl-apps-<VERSION>-x86_64.tar.gz`, and individual binaries
including `start-menu`. It also builds and tests control-surface and publishes
`control-surface-<VERSION>-x86_64.tar.gz`: a `usr/` tree with `control-surfaced`,
`ch552-padprog`, the example config, example unit and udev rule, the current
firmware images with their manifests, docs and licences (GPL-2.0 text and the
firmware's CC BY-SA 3.0 notice and attribution). Superseded firmware images
stay in the source tree only. It is not in the app bundle: smplOS installs it
with pacman (its `control-surface` package pins this asset's SHA-256) and ships
its own user unit and udev rules. Do not manually bump or tag to prepare that workflow.
The workflow only runs from `main` and serializes releases. It refuses to publish
if main advances during the build, validates all 11 mandatory binaries plus
the canonical `smpl-calendar-alertd.service`, and checks bundle contents against
the standalone assets. The service is a root-level archive member and standalone
asset with mode `0644`, never an executable. Its `--foreground` entry and
`default.target` login enablement are shared with local deployment. The smplOS
ISO/updater must retain this asset and install it in the systemd user-unit
directory; extracting executable files alone does not enable reminders.
Uploads remain a draft
until every required remote asset's size and SHA256 matches its local payload.
Only then is the release published as latest. A failed upload leaves a draft,
not an incomplete latest release; inspect the failed run before retrying.

App Center's **Update OS** confirmation launches `smplos-update --mode full`.
The OS updater fetches the published smpl-apps release bundle; pushing source
alone does not deliver new app binaries. The v0.8.23 release packages the
previously unreleased start-menu icon/pinning fix, Settings power persistence
and polling fixes, readable typography, and monitor-owned Taskbar integration.
Publish the corresponding OS helper/renderer changes before this app release.

Headless delivery checks:
`cargo test -p app-center --bin app-center os_update` and
`python -m unittest discover -s tests -p 'test_release_assets.py' -v`.

```bash
# Download a published version's bundle for the ISO build:
VERSION=0.8.27
curl -fSL "https://github.com/smpl-os/smpl-apps/releases/download/v${VERSION}/smpl-apps-${VERSION}-x86_64.tar.gz" \
  | tar -xz -C ~/.cache/smpl-apps/
```
