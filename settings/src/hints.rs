//! Settings integration for the `hints` app (Vim-style keyboard nav).
//!
//! Read/write `~/.config/smplos/hints.toml` and drive the on/off toggle
//! that the Settings UI exposes.
//!
//! The `hints` crate owns the config schema — we just re-export the
//! relevant bits so the UI thread stays decoupled from the runtime code.
//!
//! Both functions are wired up in the settings UI as soon as the Slint
//! tab is added (see the docblock below). `#![allow(dead_code)]` at the
//! module level keeps `cargo clippy -D warnings` clean until then.
//!
//! # Wiring the UI (still to do)
//!
//! In `settings/ui/main.slint`, alongside the other tab properties:
//!
//! ```slint
//! // ── Hints tab properties ──
//! in-out property <bool> hints-enabled: true;
//! callback hints-set-enabled(bool);
//! ```
//!
//! And in `settings/src/main.rs`, alongside the other tab setup blocks:
//!
//! ```rust,ignore
//! mod hints;
//! ...
//! let ui = MainWindow::new()?;
//! ui.set_hints_enabled(hints::is_enabled());
//! let ui_weak = ui.as_weak();
//! ui.on_hints_set_enabled(move |v| {
//!     if let Err(e) = hints::set_enabled(v) {
//!         debug_log!("hints toggle failed: {e}");
//!     }
//!     if let Some(ui) = ui_weak.upgrade() {
//!         ui.set_hints_enabled(hints::is_enabled());
//!     }
//! });
//! ```
//!
//! The rebind UI reuses `crate::keybindings::*` (already re-exported
//! from `smpl_common::keybindings`) — no new code needed there.

use std::path::PathBuf;
use std::process::Command;

/// Path to the shared config file.
fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("smplos")
        .join("hints.toml")
}

/// Is the hints daemon currently enabled?
///
/// Reads the on-disk config; defaults to `true` if the file is missing
/// (matches the crate default so a fresh install has hints on out of
/// the box).
pub fn is_enabled() -> bool {
    let path = config_path();
    let Ok(raw) = std::fs::read_to_string(&path) else { return true };
    // Tiny parser — same lenient key=value / JSON approach the daemon
    // uses; kept local to avoid pulling the whole `hints` crate into
    // the settings binary just for one bool.
    if let Some(v) = raw.split('#').next().and_then(|s| {
        s.lines()
            .filter_map(|l| l.split_once('='))
            .find_map(|(k, v)| (k.trim() == "\"enabled\"" || k.trim() == "enabled")
                .then(|| v.trim().trim_matches('"').trim_matches(',')
                    .parse::<bool>().ok())
                .flatten())
    }) {
        return v;
    }
    true
}

/// Toggle the daemon on or off, persisting to `~/.config/smplos/hints.toml`.
///
/// Writes a minimal TOML file (naive parser reads it back) and asks the
/// running daemon (if any) to reload. If no daemon is running, we leave
/// it to `smpl-hints` to spawn it on the next keybind press.
pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Preserve any other fields the user may have hand-tuned by reading
    // and re-writing, only touching the `enabled` line.
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 32);
    let mut wrote_enabled = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("enabled") || trimmed.starts_with("\"enabled\"") {
            out.push_str(&format!("enabled = {enabled}\n"));
            wrote_enabled = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !wrote_enabled {
        out.push_str(&format!("enabled = {enabled}\n"));
    }
    std::fs::write(&path, out)?;

    // Ask the daemon to reload (best-effort — silently ignore if not running).
    let _ = Command::new("smpl-hints").arg("reload").status();

    // If we just turned it OFF, tell any running daemon to quit so it
    // stops holding the AT-SPI connection and Wayland virtual pointer.
    if !enabled {
        let _ = Command::new("smpl-hints").arg("quit").status();
    }
    Ok(())
}
