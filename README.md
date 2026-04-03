# smpl-apps

**Latest release: v0.7.0**

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

All apps use [Slint](https://slint.dev) with the software renderer + Winit/Wayland backend for composited transparency.

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

## Releases

Pre-built binaries are published to [Releases](../../releases) and consumed by the smplOS ISO builder.

```bash
# Download all binaries for the ISO build:
curl -fSL https://github.com/smpl-os/smpl-apps/releases/latest/download/smpl-apps-x86_64.tar.gz \
  | tar -xz -C ~/.cache/smpl-apps/
```
