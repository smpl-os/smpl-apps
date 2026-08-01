# hints

Vim-style keyboard-only navigation for smplOS — overlays 2-letter hints on every
clickable widget on screen (like Vimium in the browser, but for the entire
desktop) and lets you click / right-click / hover / drag / cursor-move / scroll
without ever touching the mouse.

## Status

**MVP complete — ready for on-hardware testing.** Every module is wired up end-to-end;
the crate builds, passes clippy `-D warnings`, and 14 unit tests pass. What remains
is empirical: verify AT-SPI enumeration coverage on real apps and the wlr virtual
pointer path on your compositor.

| Module                | Status              | Notes                                                                                              |
| --------------------- | ------------------- | -------------------------------------------------------------------------------------------------- |
| `hint.rs`             | ✅ done + tested    | Deterministic prefix-free hint label generator.                                                    |
| `mode.rs`             | ✅ done + tested    | State machine (Idle / Selecting / Cursor / Scroll) with drag two-stage flow.                       |
| `ipc.rs`              | ✅ done + tested    | Unix-socket JSON protocol.                                                                         |
| `config.rs`           | ✅ done + tested    | `~/.config/smplos/hints.toml` — enable flag + tuning.                                              |
| `src/main.rs`         | ✅ done             | CLI (`smpl-hints click`, etc.) — spawns daemon on demand.                                          |
| `src/bin/daemon.rs`   | ✅ done             | Slint event loop + IPC thread + key dispatch + drag two-stage + Escape all wired.                  |
| `atspi.rs`            | ✅ done             | Iterative walk of the a11y tree, Component::get_extents(Screen) for hitbox, MAX_NODES safety cap.  |
| `inject.rs`           | ✅ done             | Full wlr virtual pointer client: motion_absolute, button, axis, frame. Background thread.          |
| `overlay.rs` + Slint  | ✅ done             | Full-screen transparent overlay + hint pills + cursor crosshair + FocusScope key capture.          |

## Architecture

```
┌─────────────────┐   ~/.config/smplos/hints.toml
│  smpl-hints CLI │────────────────────────┐
└────────┬────────┘                        │
         │ UNIX socket                     ▼
         │ ($XDG_RUNTIME_DIR/           ┌──────────────┐
         │  smpl-hintsd.sock)           │ Config       │
         │                              └──────┬───────┘
         ▼                                     │
┌────────────────────────────────┐             │
│         smpl-hintsd            │◀────────────┘
│                                │
│  ┌──────────────────────────┐  │       ┌─────────────────────────┐
│  │  IPC listener            │  │       │ AT-SPI2 bus             │
│  │  (JSON one-shot RPC)     │──┼──────▶│ (widget enumeration)    │
│  └───────────┬──────────────┘  │       └─────────────────────────┘
│              │                 │
│  ┌───────────▼──────────────┐  │       ┌─────────────────────────┐
│  │  Mode state machine      │  │       │ wlr_virtual_pointer_v1  │
│  │  Idle/Selecting/…        │──┼──────▶│ (click/scroll injection)│
│  └───────────┬──────────────┘  │       └─────────────────────────┘
│              │                 │
│  ┌───────────▼──────────────┐  │       ┌─────────────────────────┐
│  │  Slint overlay window    │──┼──────▶│ Hyprland compositor     │
│  │  (transparent, layer)    │  │       │ (windowrulev2 + layer)  │
│  └──────────────────────────┘  │       └─────────────────────────┘
└────────────────────────────────┘
```

## Usage (once modes are wired up)

```
smpl-hints click        # Vimium's `f`  — click a hint
smpl-hints right-click  # Vimium's `F`  — right-click a hint
smpl-hints hover        #                — move cursor to hint, no click
smpl-hints drag         #                — pick source hint, then destination
smpl-hints cursor       #                — enter hjkl motion mode
smpl-hints scroll       #                — enter j/k scroll mode
```

## Default keybindings (see smplos `bindings.conf`)

| Combo             | Action                            |
| ----------------- | --------------------------------- |
| `Super+;`         | Hint mode — click                 |
| `Super+Shift+;`   | Hint mode — right-click           |
| `Super+Ctrl+;`    | Hint mode — hover                 |
| `Super+Alt+;`     | Hint mode — drag                  |
| `Super+H`         | Cursor motion mode (hjkl)         |
| `Super+Shift+H`   | Scroll mode                       |

Rebindable via the Settings app.

## Runtime prerequisites

* `at-spi2-core` must be installed (`pacman -S at-spi2-core` — already a
  dep of gtk3/gtk4 which smplOS uses everywhere).
* Environment: `GTK_MODULES=gail:atk-bridge` for GTK apps (usually default).
* Qt apps: `QT_ACCESSIBILITY=1` (add to `/etc/environment` if used heavily).
* Electron apps: `--force-renderer-accessibility` in the wrapper (webapp-center
  can do this by default).
* Compositor must support `zwlr_virtual_pointer_manager_v1` — Hyprland does.

## Development

```bash
cd smpl-apps
cargo check -p hints        # fast type-check
cargo test  -p hints        # runs unit tests (hint gen, ipc, mode SM, config)
cargo build -p hints --release
```

## Next steps

1. Implement `atspi::walk_all_apps` — live iteration on the AT-SPI proxy API.
2. Implement `inject::VirtualPointer::connect` — bind
   `zwlr_virtual_pointer_manager_v1`, dispatch Motion/Frame/Button/Axis.
3. Wire the Slint event loop in the daemon (currently the daemon skeleton
   enumerates + selects labels but doesn't paint; requires running the Slint
   backend on the main thread and driving key input from the Wayland grab).
4. Add Hyprland `layerrule` + `windowrulev2` for `hints-overlay` in
   `smplos/src/shared/configs/hypr/windows.conf`.
5. Settings-app tab: on/off toggle + rebind UI (reuse the existing
   `smpl_common::keybindings` module).

Copyright © smpl-os · MIT.
