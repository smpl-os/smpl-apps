//! # hints — Vim-style keyboard navigation for smplOS
//!
//! Provides Vimium-like "hint mode" navigation on the Linux desktop:
//! overlays 2-letter keyboard hints on every clickable widget in every
//! visible window, then lets the user type a hint to click / right-click /
//! hover / drag it. Also supports an hjkl cursor-motion mode and scroll.
//!
//! ## Architecture
//!
//! Two binaries:
//!
//! * **`smpl-hintsd`** — the daemon. Owns:
//!   - AT-SPI2 D-Bus connection (widget enumeration)
//!   - Wayland virtual pointer (click/scroll injection)
//!   - Slint transparent overlay window (hint labels)
//!   - Unix-socket command listener at `$XDG_RUNTIME_DIR/smpl-hintsd.sock`
//!
//! * **`smpl-hints`** — the CLI shim. Parses the subcommand
//!   (`click` / `right-click` / `hover` / `drag` / `cursor` / `scroll`),
//!   ensures the daemon is running (spawns it detached if not), then
//!   sends a JSON command over the Unix socket.
//!
//! The daemon model gives instant response — the AT-SPI tree walk of every
//! window can take 100-300 ms on a busy desktop, so we do it in-process
//! and reuse the connection across invocations.
//!
//! ## Modules
//!
//! * [`config`] — reads `~/.config/smplos/hints.toml` (enabled flag, tuning).
//! * [`ipc`]    — Unix-socket protocol between CLI and daemon.
//! * [`hint`]   — deterministic 2-letter hint label generation.
//! * [`mode`]   — mode state machine (Idle / Selecting / Cursor).
//! * [`atspi`]  — AT-SPI2 widget-tree enumeration.
//! * [`inject`] — Wayland `wlr_virtual_pointer` click/scroll injection.
//! * [`overlay`] — Slint transparent full-screen overlay window.

pub mod config;
pub mod hint;
pub mod ipc;
pub mod mode;

// Runtime modules — kept out of shared surface until the API stabilises.
pub mod atspi;
pub mod inject;
pub mod overlay;
