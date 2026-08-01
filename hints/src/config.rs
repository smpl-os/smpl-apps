//! Config: `~/.config/smplos/hints.toml`
//!
//! The Settings app writes this file when the user toggles hints on/off
//! or tunes anything. The daemon watches for changes (via a periodic
//! reload on every socket command — cheap) and re-reads on modify.
//!
//! All fields are optional so an empty file just falls back to defaults.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Full config. Defaults chosen so a fresh install is comfortable.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// Master enable switch. When `false`, the CLI still runs but the
    /// daemon refuses commands (returns `disabled`). The settings-app
    /// toggle flips this and triggers `systemctl --user restart smpl-hintsd`.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Hint label characters. Only these keys are typed to select hints.
    /// Default is the Vimium home-row-friendly set. Order matters — more
    /// frequent letters go first for shorter labels.
    #[serde(default = "default_hint_chars")]
    pub hint_chars: String,

    /// Minimum widget size (px) to be worth hinting. Skips 1px separators.
    #[serde(default = "default_min_size")]
    pub min_widget_size: u32,

    /// Speed of hjkl cursor mode, in pixels per keystroke.
    #[serde(default = "default_cursor_step")]
    pub cursor_step: u32,

    /// Fast-mode multiplier when Shift is held in cursor mode.
    #[serde(default = "default_cursor_fast_mult")]
    pub cursor_fast_mult: u32,

    /// Scroll wheel notches per keystroke in cursor mode's j/k for scroll.
    #[serde(default = "default_scroll_step")]
    pub scroll_step: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            hint_chars: default_hint_chars(),
            min_widget_size: default_min_size(),
            cursor_step: default_cursor_step(),
            cursor_fast_mult: default_cursor_fast_mult(),
            scroll_step: default_scroll_step(),
        }
    }
}

fn default_true() -> bool { true }
fn default_hint_chars() -> String { "fjdkslaghurieowpnvbcm".to_string() }
fn default_min_size() -> u32 { 8 }
fn default_cursor_step() -> u32 { 20 }
fn default_cursor_fast_mult() -> u32 { 5 }
fn default_scroll_step() -> u32 { 3 }

impl Config {
    /// Read `~/.config/smplos/hints.toml`; return defaults if missing.
    ///
    /// TOML parsing is intentionally lenient — malformed keys fall back
    /// silently rather than crashing the daemon. That's a deliberate choice
    /// because the daemon must NEVER fail because of a user typo in a
    /// config file.
    pub fn load() -> Self {
        let path = Self::path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        // Prefer serde_json if the file happens to be JSON (some users edit
        // via the settings app which writes JSON). Fall back to a naive
        // key=value parse for the TOML case so we don't drag in a full
        // toml crate for four fields.
        if let Ok(cfg) = serde_json::from_str::<Config>(&raw) {
            return cfg;
        }
        parse_naive(&raw)
    }

    /// Write the config (settings app calls this after a toggle).
    ///
    /// Always writes JSON — it round-trips cleanly through serde and the
    /// naive parser also happily reads it back. Keeps the on-disk format
    /// stable regardless of which tool wrote it last.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let s = serde_json::to_string_pretty(self)?;
        std::fs::write(path, s)?;
        Ok(())
    }

    /// Canonical path for the config file.
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("smplos")
            .join("hints.toml")
    }
}

/// Very small key=value parser so we don't need a toml crate for six fields.
/// Accepts `key = value` and `key = "quoted string"`.
fn parse_naive(raw: &str) -> Config {
    let mut cfg = Config::default();
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() { continue; }
        let (k, v) = match line.split_once('=') {
            Some(x) => x,
            None => continue,
        };
        let k = k.trim();
        let v = v.trim().trim_matches('"');
        match k {
            "enabled"          => if let Ok(b) = v.parse() { cfg.enabled = b; },
            "hint_chars"       => cfg.hint_chars = v.to_string(),
            "min_widget_size"  => if let Ok(n) = v.parse() { cfg.min_widget_size = n; },
            "cursor_step"      => if let Ok(n) = v.parse() { cfg.cursor_step = n; },
            "cursor_fast_mult" => if let Ok(n) = v.parse() { cfg.cursor_fast_mult = n; },
            "scroll_step"      => if let Ok(n) = v.parse() { cfg.scroll_step = n; },
            _ => {}
        }
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = Config::default();
        assert!(c.enabled);
        assert!(!c.hint_chars.is_empty());
        assert!(c.min_widget_size > 0);
    }

    #[test]
    fn naive_parser_reads_key_value() {
        let raw = r#"
            enabled = false
            hint_chars = "asdf"
            cursor_step = 42
        "#;
        let c = parse_naive(raw);
        assert!(!c.enabled);
        assert_eq!(c.hint_chars, "asdf");
        assert_eq!(c.cursor_step, 42);
    }

    #[test]
    fn json_round_trip() {
        let c = Config { enabled: false, cursor_step: 33, ..Default::default() };
        let s = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&s).unwrap();
        assert!(!back.enabled);
        assert_eq!(back.cursor_step, 33);
    }
}
