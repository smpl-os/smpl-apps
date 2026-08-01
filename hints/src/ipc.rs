//! Unix-socket command protocol between `smpl-hints` (CLI) and
//! `smpl-hintsd` (daemon).
//!
//! Socket lives at `$XDG_RUNTIME_DIR/smpl-hintsd.sock`. One request per
//! connection; server sends one JSON reply and closes.
//!
//! Wire format: single line of JSON, terminated by `\n`.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A command sent from the CLI to the daemon.
///
/// The `mode` variants correspond 1:1 to the six user-visible actions:
/// click, right-click, hover, drag, cursor mode, scroll.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Command {
    /// Enter hint mode; on selection, single left-click the target.
    Click,
    /// Enter hint mode; on selection, right-click the target.
    RightClick,
    /// Enter hint mode; on selection, move cursor over target (no click).
    Hover,
    /// Enter drag mode: pick a source hint, then a destination hint;
    /// synthesize a press-move-release. Wayland reliability caveat: some
    /// compositors reject synthesized button-hold events.
    Drag,
    /// Enter hjkl cursor motion mode. Overlay shows a small status bar.
    Cursor,
    /// Enter scroll mode. j/k → scroll down/up on the widget under cursor.
    Scroll,
    /// Non-mode commands (settings-app plumbing).
    Reload,
    /// Health check — daemon replies `Ok`. Used by the CLI to decide
    /// whether it needs to spawn the daemon.
    Ping,
    /// Ask the daemon to gracefully exit. Used by `systemctl --user stop`.
    Quit,
}

/// Daemon → CLI reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Reply {
    /// Command accepted; work will proceed asynchronously.
    Ok,
    /// Hints are turned off in `~/.config/smplos/hints.toml`.
    Disabled,
    /// Command failed with a human-readable reason.
    Error { message: String },
}

/// Canonical socket path.
pub fn socket_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("smpl-hintsd.sock")
}

impl Command {
    /// Serialize to the wire format (single JSON line + newline).
    pub fn to_wire(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Command is always serializable");
        s.push('\n');
        s
    }
}

impl Reply {
    pub fn to_wire(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Reply is always serializable");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip() {
        for cmd in [
            Command::Click,
            Command::RightClick,
            Command::Hover,
            Command::Drag,
            Command::Cursor,
            Command::Scroll,
            Command::Ping,
            Command::Reload,
            Command::Quit,
        ] {
            let wire = cmd.to_wire();
            let back: Command = serde_json::from_str(wire.trim()).unwrap();
            // Compare by JSON so we don't have to derive PartialEq for the
            // enum (which would tie us to a serde detail).
            assert_eq!(
                serde_json::to_string(&cmd).unwrap(),
                serde_json::to_string(&back).unwrap(),
            );
        }
    }

    #[test]
    fn reply_error_carries_message() {
        let r = Reply::Error { message: "boom".to_string() };
        let wire = r.to_wire();
        assert!(wire.contains("\"boom\""));
        assert!(wire.contains("error"));
    }
}
