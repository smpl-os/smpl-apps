# smpl-apps

**Current workspace version: v0.8.22**

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

Pre-built binaries are published to [Releases](../../releases) and consumed by the smplOS ISO builder.
Run the **Release** workflow (`.github/workflows/release.yml`, `workflow_dispatch`)
from `main` after the changes are merged. It increments the workspace patch
version itself, commits the version/lockfile change, and publishes tag
`v<VERSION>`, bundle `smpl-apps-<VERSION>-x86_64.tar.gz`, and individual binaries
including `start-menu`. Do not manually bump or tag to prepare that workflow.
Starting from v0.8.22, its next version would be v0.8.23.

```bash
# Download a published version's bundle for the ISO build:
VERSION=0.8.22
curl -fSL "https://github.com/smpl-os/smpl-apps/releases/download/v${VERSION}/smpl-apps-${VERSION}-x86_64.tar.gz" \
  | tar -xz -C ~/.cache/smpl-apps/
```
