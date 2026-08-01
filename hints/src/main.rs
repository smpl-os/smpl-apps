//! `smpl-hints` — thin CLI shim.
//!
//! Parses the subcommand, ensures the daemon is running (spawns it
//! detached if not), then sends a single JSON command over the Unix
//! socket at `$XDG_RUNTIME_DIR/smpl-hintsd.sock`.
//!
//! All actual work lives in `smpl-hintsd`. This binary is intentionally
//! tiny so it can be re-invoked ~instantly on every hint-mode keybind.

use anyhow::{Context, Result};
use hints::ipc::{Command, Reply, socket_path};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command as Proc, Stdio};
use std::time::Duration;

fn print_help() {
    eprintln!(
        "smpl-hints — Vim-style keyboard navigation for smplOS\n\
         \n\
         USAGE:\n    smpl-hints <MODE>\n\
         \n\
         MODES:\n\
             click         Click a hint (single left-click)\n\
             right-click   Right-click a hint\n\
             hover         Move cursor over hint (no click)\n\
             drag          Pick source hint, then destination hint\n\
             cursor        Enter hjkl cursor motion mode\n\
             scroll        Enter j/k scroll mode\n\
             ping          Health check (0 if daemon running)\n\
             reload        Ask daemon to re-read config\n\
             quit          Stop the daemon"
    );
}

fn parse_command(arg: &str) -> Option<Command> {
    Some(match arg {
        "click"       => Command::Click,
        "right-click" => Command::RightClick,
        "hover"       => Command::Hover,
        "drag"        => Command::Drag,
        "cursor"      => Command::Cursor,
        "scroll"      => Command::Scroll,
        "ping"        => Command::Ping,
        "reload"      => Command::Reload,
        "quit"        => Command::Quit,
        _             => return None,
    })
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode_arg = match args.next() {
        Some(a) => a,
        None => { print_help(); std::process::exit(2); }
    };
    if matches!(mode_arg.as_str(), "-h" | "--help") { print_help(); return Ok(()); }

    let cmd = match parse_command(&mode_arg) {
        Some(c) => c,
        None => { eprintln!("unknown mode: {mode_arg}"); print_help(); std::process::exit(2); }
    };

    // 1. Try to connect. If refused, spawn the daemon and retry.
    let mut stream = match try_connect() {
        Ok(s) => s,
        Err(_) => {
            spawn_daemon().context("spawn smpl-hintsd")?;
            // Give the daemon a moment to bind the socket. 300 ms is more
            // than enough on any modern box; empirically ~50 ms is typical.
            for _ in 0..30 {
                if let Ok(s) = try_connect() { break_stream(s, &cmd); return Ok(()); }
                std::thread::sleep(Duration::from_millis(10));
            }
            anyhow::bail!("daemon didn't start in time")
        }
    };

    break_stream_ref(&mut stream, &cmd);
    Ok(())
}

fn try_connect() -> Result<UnixStream> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .with_context(|| format!("connect to {}", path.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    Ok(stream)
}

/// Consume `stream` and issue one command (owning variant).
fn break_stream(mut stream: UnixStream, cmd: &Command) {
    break_stream_ref(&mut stream, cmd);
}

/// Issue one command over `stream`, print any error to stderr, exit
/// non-zero on `Reply::Error` / `Reply::Disabled` so keybind chains can
/// react (e.g. show a notification when hints are off).
fn break_stream_ref(stream: &mut UnixStream, cmd: &Command) {
    let wire = cmd.to_wire();
    if let Err(e) = stream.write_all(wire.as_bytes()) {
        eprintln!("smpl-hints: write failed: {e}");
        std::process::exit(1);
    }
    let mut buf = String::new();
    if let Err(e) = stream.read_to_string(&mut buf) {
        // A partial write from the daemon before it drops the connection
        // is normal for one-shot RPC; ignore.
        if buf.is_empty() {
            eprintln!("smpl-hints: read failed: {e}");
            std::process::exit(1);
        }
    }
    match serde_json::from_str::<Reply>(buf.trim()) {
        Ok(Reply::Ok) => {},
        Ok(Reply::Disabled) => {
            eprintln!("smpl-hints: hints are disabled — enable in Settings → Hints");
            std::process::exit(3);
        }
        Ok(Reply::Error { message }) => {
            eprintln!("smpl-hints: {message}");
            std::process::exit(1);
        }
        Err(_) if buf.trim().is_empty() => {} // daemon closed silently after ack
        Err(e) => {
            eprintln!("smpl-hints: malformed reply ({e}): {buf}");
            std::process::exit(1);
        }
    }
}

/// Fork + detach a fresh `smpl-hintsd`.
///
/// Uses `nohup`-equivalent by ignoring SIGHUP via stdin/stdout/stderr
/// redirection to /dev/null. The daemon inherits the environment
/// (WAYLAND_DISPLAY, DBUS_SESSION_BUS_ADDRESS, XDG_RUNTIME_DIR) which
/// is exactly what it needs.
fn spawn_daemon() -> Result<()> {
    Proc::new("smpl-hintsd")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn smpl-hintsd — is it in PATH?")?;
    Ok(())
}
