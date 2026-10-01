# smpl-apps

**Current workspace version: v0.8.23**

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

## Building

```bash
cargo build --release --workspace
```

Requires Arch Linux (or equivalent) with: `fontconfig freetype2 libxkbcommon wayland gtk4 gtk4-layer-shell libadwaita`

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

### Publishing

Pre-built binaries are published to [Releases](../../releases) and consumed by the smplOS ISO builder.
Run the **Release** workflow (`.github/workflows/release.yml`, `workflow_dispatch`)
from `main` after the changes are merged. It increments the workspace patch
version itself, commits the version/lockfile change, and publishes tag
`v<VERSION>`, bundle `smpl-apps-<VERSION>-x86_64.tar.gz`, and individual binaries
including `start-menu`. Do not manually bump or tag to prepare that workflow.
The workflow only runs from `main` and serializes releases. It refuses to publish
if main advances during the build, validates all 11 mandatory binaries, and
checks bundle contents against the standalone assets. Uploads remain a draft
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
VERSION=0.8.23
curl -fSL "https://github.com/smpl-os/smpl-apps/releases/download/v${VERSION}/smpl-apps-${VERSION}-x86_64.tar.gz" \
  | tar -xz -C ~/.cache/smpl-apps/
```
