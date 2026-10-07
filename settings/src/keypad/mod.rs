//! Settings > Keypad: CH552 macro keypads driven by the control-surface daemon.
//!
//! Device detection, layouts and firmware flashing go through smplOS's
//! `keypad-ctl` (sysfs only; flashing is a dry run unless explicitly enabled).
//! The mapping editor writes the daemon's JSONC config after validating it
//! with `control-surfaced check-config` and backing up the previous file.

pub mod config;
pub mod json;
pub mod ui;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use serde_json::Value;

pub const SERVICE: &str = "control-surface.service";
const BACKUPS_KEPT: usize = 10;

// ── Devices ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pad {
    pub path: String,
    pub firmware: String,
    pub firmware_label: String,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    pub board: String,
    pub keys: usize,
    pub knobs: usize,
    pub cols: usize,
    /// "device" (self-described), "user" (chosen board) or "default".
    pub layout_source: String,
    /// The daemon can address every control of this layout.
    pub supported: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub pads: Vec<Pad>,
    pub bootloaders: usize,
    pub service_installed: bool,
    pub service_state: String,
    /// keypad-ctl is missing (smplOS too old) or failed.
    pub helper_error: Option<String>,
    /// The daemon's Settings API is on the session bus.
    pub api: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub keys: usize,
    pub knobs: usize,
    pub cols: usize,
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn count(v: &Value, key: &str) -> usize {
    v.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

pub fn parse_status(json: &str) -> Result<Status, String> {
    let v: Value = serde_json::from_str(json.trim()).map_err(|e| format!("keypad-ctl: {e}"))?;
    let pads = v
        .get("devices")
        .and_then(Value::as_array)
        .map(|devices| {
            devices
                .iter()
                .map(|d| {
                    let layout = d.get("layout").cloned().unwrap_or(Value::Null);
                    Pad {
                        path: text(d, "path"),
                        firmware: text(d, "firmware"),
                        firmware_label: text(d, "firmware_label"),
                        manufacturer: text(d, "manufacturer"),
                        product: text(d, "product"),
                        serial: text(d, "serial"),
                        board: text(&layout, "board"),
                        keys: count(&layout, "keys"),
                        knobs: count(&layout, "knobs"),
                        cols: count(&layout, "cols").max(1),
                        layout_source: text(&layout, "source"),
                        supported: layout.get("supported").and_then(Value::as_bool).unwrap_or(true),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let service = v.get("service").cloned().unwrap_or(Value::Null);
    Ok(Status {
        pads,
        bootloaders: v.get("bootloaders").and_then(Value::as_array).map_or(0, Vec::len),
        service_installed: service.get("installed").and_then(Value::as_bool).unwrap_or(false),
        service_state: text(&service, "state"),
        helper_error: None,
        api: false,
    })
}

fn keypad_ctl() -> Command {
    Command::new(std::env::var("SMPLOS_KEYPAD_CTL").unwrap_or_else(|_| "keypad-ctl".into()))
}

fn run_keypad_ctl(args: &[&str]) -> Result<String, String> {
    let out = keypad_ctl()
        .args(args)
        .output()
        .map_err(|_| "keypad-ctl is not installed (update smplOS)".to_string())?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().last().unwrap_or("keypad-ctl failed").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn load_status() -> Status {
    let mut status = match run_keypad_ctl(&["status", "--service"]).and_then(|out| parse_status(&out)) {
        Ok(status) => status,
        Err(e) => Status { helper_error: Some(e), ..Status::default() },
    };
    status.api = api_available();
    status
}

pub fn load_boards() -> Vec<Board> {
    let Ok(out) = run_keypad_ctl(&["boards"]) else {
        return Vec::new();
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(out.trim()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|b| Board {
            id: text(b, "id"),
            name: text(b, "name"),
            keys: count(b, "keys"),
            knobs: count(b, "knobs"),
            cols: count(b, "cols").max(1),
        })
        .collect()
}

pub fn set_board(id: &str) -> Result<(), String> {
    run_keypad_ctl(&["set-board", id]).map(|_| ())
}

/// Keys first (row-major), then knobs in a column to the right.
pub fn control_ids(keys: usize, knobs: usize) -> Vec<String> {
    (1..=keys)
        .map(|k| format!("key{k}"))
        .chain((1..=knobs).map(|k| format!("knob{k}")))
        .collect()
}

pub fn service_action(action: &str) -> Result<(), String> {
    if !["start", "restart", "stop"].contains(&action) {
        return Err(format!("unsupported service action {action}"));
    }
    let out = Command::new("systemctl")
        .args(["--user", action, SERVICE])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ── Daemon and config file ──────────────────────────────────────────────────

pub fn daemon_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SMPLOS_CONTROL_SURFACED") {
        return Some(PathBuf::from(path)).filter(|p| p.is_file());
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join("control-surfaced")).find(|p| p.is_file())
}

pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join("control-surface/config.jsonc")
}

/// The daemon's built-in example: what it runs when no config file exists.
pub fn example_config(daemon: Option<&Path>) -> Option<String> {
    let out = Command::new(daemon?).arg("example-config").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[derive(Debug, PartialEq)]
pub enum Validation {
    Ok { warnings: Vec<String> },
    Invalid(String),
    /// The daemon is not installed; only Settings' own checks ran.
    Unchecked,
}

/// `control-surfaced check-config -c FILE`: exit 0 = valid; "warning: ..." lines.
pub fn check_config(daemon: Option<&Path>, file: &Path) -> Validation {
    let Some(daemon) = daemon else {
        return Validation::Unchecked;
    };
    let Ok(out) = Command::new(daemon).arg("check-config").arg("-c").arg(file).output() else {
        return Validation::Unchecked;
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    if out.status.success() {
        let warnings = stdout
            .lines()
            .filter_map(|l| l.strip_prefix("warning: "))
            .map(String::from)
            .collect();
        return Validation::Ok { warnings };
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let message = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("the keypad service rejected the config");
    // "config /path/.tmp: profile x: ..." -> "profile x: ..."
    let message = match message.strip_prefix("config ") {
        Some(rest) => rest.split_once(": ").map_or(rest, |(_, m)| m),
        None => message,
    };
    Validation::Invalid(message.to_string())
}

/// Whether the daemon accepts `{"mouse": ...}` bindings (API request R3).
pub fn probe_mouse(daemon: Option<&Path>) -> bool {
    let Some(daemon) = daemon else {
        return false;
    };
    let dir = std::env::temp_dir().join(format!("smplos-keypad-probe-{}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let file = dir.join("probe.jsonc");
    let probe = r#"{"hardware":"default:keys-then-knobs","profiles":[{"name":"p","bindings":{"key1":{"mouse":"left"}}}]}"#;
    let ok = std::fs::write(&file, probe).is_ok() && matches!(check_config(Some(daemon), &file), Validation::Ok { .. });
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

#[derive(Debug, PartialEq)]
pub struct Saved {
    pub backup: Option<PathBuf>,
    pub validation: Validation,
}

/// Local time as YYYYmmdd-HHMMSS.
fn timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: localtime_r only writes the provided tm.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return secs.to_string();
    }
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// Validates `text`, backs up the current file and atomically replaces it.
/// The daemon hot-reloads the file; an invalid file is never written, and
/// neither is one that changed on disk since Settings read it (`expected`:
/// the text read then, `None` if there was no file).
pub fn save_config(path: &Path, text: &str, daemon: Option<&Path>, expected: Option<&str>) -> Result<Saved, String> {
    if std::fs::read_to_string(path).ok().as_deref() != expected {
        return Err(
            "Not saved: the config file changed on disk since Settings read it. Revert to load it, then redo your changes."
                .into(),
        );
    }
    let dir = path.parent().ok_or("invalid config path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Same directory, so relative "hardware" paths resolve as they will live.
    let tmp = dir.join(format!(".config.jsonc.smplos-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let validation = check_config(daemon, &tmp);
    if let Validation::Invalid(message) = &validation {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Not saved: {message}"));
    }
    let mut backup = None;
    if path.exists() {
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).map_err(|e| e.to_string())?;
        let stamp = timestamp();
        let target = (0..)
            .map(|n| backups.join(format!("config-{stamp}-{n:03}.jsonc")))
            .find(|p| !p.exists())
            .expect("a free backup name");
        if let Err(e) = std::fs::copy(path, &target) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Not saved: could not back up the current config ({e})"));
        }
        prune_backups(&backups);
        backup = Some(target);
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Not saved: {e}")
    })?;
    Ok(Saved { backup, validation })
}

fn prune_backups(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("config-")))
        .collect();
    files.sort();
    let excess = files.len().saturating_sub(BACKUPS_KEPT);
    for old in files.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

/// Kdenlive's advertised actions, when a Kdenlive with its control-surface
/// interface enabled is running (read-only query, no lease).
pub fn running_kdenlive_actions(daemon: Option<&Path>) -> Vec<(String, String)> {
    let Some(daemon) = daemon else {
        return Vec::new();
    };
    let Ok(out) = Command::new(daemon).args(["list-capabilities", "--json"]).output() else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };
    let mut actions = Vec::new();
    for service in v.get("kdenlive").and_then(Value::as_array).into_iter().flatten() {
        parse_actions(service, &mut actions);
    }
    actions
}

// ── Firmware ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FirmwareImage {
    pub path: String,
    pub name: String,
    pub version: String,
    pub board: String,
    pub license: String,
    pub size: u64,
    pub sha256: String,
    pub verified: bool,
    pub mismatch: bool,
}

impl FirmwareImage {
    pub fn title(&self) -> String {
        let version = if self.version.is_empty() { String::new() } else { format!(" {}", self.version) };
        let check = if self.verified { "" } else if self.mismatch { " — checksum mismatch" } else { " — unverified" };
        format!("{}{version}{check}", self.name)
    }
}

pub fn parse_images(json: &str) -> Vec<FirmwareImage> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json.trim()) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|i| FirmwareImage {
            path: text(i, "path"),
            name: text(i, "name"),
            version: text(i, "version"),
            board: text(i, "board"),
            license: text(i, "license"),
            size: i.get("size").and_then(Value::as_u64).unwrap_or(0),
            sha256: text(i, "sha256"),
            verified: i.get("verified").and_then(Value::as_bool).unwrap_or(false),
            mismatch: i.get("mismatch").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect()
}

pub fn firmware_images() -> Vec<FirmwareImage> {
    run_keypad_ctl(&["firmware", "list"]).map(|o| parse_images(&o)).unwrap_or_default()
}

/// Real flashing stays off until the open firmware has been validated on
/// hardware; the wizard runs `keypad-ctl firmware flash` as a dry run.
pub fn real_flash_enabled() -> bool {
    std::env::var("SMPLOS_KEYPAD_REAL_FLASH").as_deref() == Ok("1")
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlashStep {
    pub step: String,
    pub status: String,
    pub detail: String,
}

pub fn parse_flash_line(line: &str) -> Option<FlashStep> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(FlashStep { step: text(&v, "step"), status: text(&v, "status"), detail: text(&v, "detail") })
}

/// Streams `keypad-ctl firmware flash IMAGE --json [--execute]`; true on success.
pub fn flash(image: &str, execute: bool, mut on_step: impl FnMut(FlashStep)) -> bool {
    let mut cmd = keypad_ctl();
    cmd.args(["firmware", "flash", image, "--json"]);
    if execute {
        cmd.arg("--execute");
    }
    let Ok(mut child) = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
        on_step(FlashStep { step: "error".into(), status: "failed".into(), detail: "keypad-ctl is not installed".into() });
        return false;
    };
    let mut done = false;
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(step) = parse_flash_line(&line) {
                done = done || (step.step == "done" && step.status == "ok");
                on_step(step);
            }
        }
    }
    child.wait().is_ok_and(|s| s.success()) && done
}

// ── Live input ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct InputEvent {
    /// "key7" or "knob2".
    pub control: String,
    /// "press", "release", "ccw" or "cw".
    pub event: String,
}

pub fn parse_event(line: &str) -> Option<InputEvent> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let slot = text(&v, "slot");
    let (control, event) = match slot.split_once('.') {
        Some((c, e)) => (c.to_string(), e.to_string()),
        None => (slot, text(&v, "event")),
    };
    let valid = (control.starts_with("key") || control.starts_with("knob"))
        && control.trim_start_matches(char::is_alphabetic).parse::<u32>().is_ok();
    valid.then_some(InputEvent { control, event })
}

// ── Daemon Settings API (org.smplos.ControlSurface1) ───────────────────────

pub const DBUS_SERVICE: &str = "org.smplos.ControlSurface";
pub const DBUS_PATH: &str = "/org/smplos/ControlSurface";
pub const DBUS_INTERFACE: &str = "org.smplos.ControlSurface1";

/// A shared session-bus connection for short queries, reopened after errors.
fn bus() -> Option<zbus::blocking::Connection> {
    static BUS: std::sync::Mutex<Option<zbus::blocking::Connection>> = std::sync::Mutex::new(None);
    let mut slot = BUS.lock().ok()?;
    if slot.is_none() {
        *slot = zbus::blocking::Connection::session().ok();
    }
    slot.clone()
}

/// Whether the daemon's Settings API is on the session bus.
pub fn api_available() -> bool {
    let Some(conn) = bus() else {
        return false;
    };
    conn.call_method(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        Some("org.freedesktop.DBus"),
        "NameHasOwner",
        &DBUS_SERVICE,
    )
    .ok()
    .and_then(|m| m.body().deserialize::<bool>().ok())
    .unwrap_or(false)
}

pub fn mock_events() -> Option<PathBuf> {
    std::env::var_os("SMPLOS_KEYPAD_EVENTS").map(PathBuf::from)
}

/// Delivers live input on a worker thread until `on_event` returns false:
/// the daemon's `InputEvent` signal, or JSON lines appended to
/// `SMPLOS_KEYPAD_EVENTS` (mock). This only listens; it never opens the keypad.
pub fn spawn_live_input(on_event: impl Fn(InputEvent) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        if let Some(file) = mock_events() {
            follow_file(&file, &on_event);
            return;
        }
        while listen_signals(&on_event) {
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Returns false when the receiver is gone, true to reconnect.
fn listen_signals(on_event: &impl Fn(InputEvent) -> bool) -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return true;
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(DBUS_INTERFACE)
        .and_then(|b| b.member("InputEvent"))
        .and_then(|b| b.path(DBUS_PATH))
        .map(|b| b.build());
    let Ok(rule) = rule else {
        return false;
    };
    let Ok(iter) = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, Some(64)) else {
        return true;
    };
    for message in iter {
        let Ok(message) = message else {
            return true;
        };
        if let Ok((slot, event, _delta)) = message.body().deserialize::<(String, String, i32)>() {
            let valid = parse_event(&serde_json::json!({ "slot": slot, "event": event }).to_string());
            if let Some(event) = valid {
                if !on_event(event) {
                    return false;
                }
            }
        }
    }
    true
}

/// Holds the daemon's identify mode (inputs are reported, nothing is
/// dispatched) on one long-lived connection. The daemon ends identify mode when
/// that connection closes, so a crashed Settings never leaves the keypad mute.
pub struct Identify {
    tx: std::sync::mpsc::Sender<bool>,
}

impl Identify {
    pub fn spawn() -> Self {
        use std::sync::mpsc::RecvTimeoutError;
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        std::thread::spawn(move || {
            let mut conn: Option<zbus::blocking::Connection> = None;
            let mut wanted = false;
            let mut held = false;
            loop {
                // Renew well before the daemon's 10-minute safety net; retry
                // soon if the daemon wasn't there.
                let wait = if wanted && !held { 5 } else { 240 };
                match rx.recv_timeout(Duration::from_secs(wait)) {
                    Ok(on) => wanted = on,
                    Err(RecvTimeoutError::Timeout) if wanted => {}
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                if conn.is_none() {
                    conn = zbus::blocking::Connection::session().ok();
                }
                held = conn.as_ref().is_some_and(|c| {
                    c.call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "SetIdentify", &wanted)
                        .is_ok()
                }) && wanted;
                if wanted && !held {
                    conn = None;
                }
            }
        });
        Self { tx }
    }

    pub fn set(&self, on: bool) {
        let _ = self.tx.send(on);
    }
}

// ── Daemon capabilities (`features`, `list-actions`) ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Features {
    pub max_keys: usize,
    pub max_knobs: usize,
    pub mouse: bool,
}

pub fn parse_features(json: &str) -> Option<Features> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let slots = v.get("slots")?;
    Some(Features {
        max_keys: slots.get("maxKeys").and_then(Value::as_u64)? as usize,
        max_knobs: slots.get("maxKnobs").and_then(Value::as_u64)? as usize,
        mouse: v.get("mouseNames").and_then(Value::as_array).is_some_and(|m| !m.is_empty()),
    })
}

/// `control-surfaced features --json`; `None` for daemons that predate it.
pub fn features(daemon: Option<&Path>) -> Option<Features> {
    let out = Command::new(daemon?).args(["features", "--json"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_features(&String::from_utf8_lossy(&out.stdout))
}

fn parse_actions(v: &Value, into: &mut Vec<(String, String)>) {
    for a in v.get("actions").and_then(Value::as_array).into_iter().flatten() {
        let id = text(a, "id");
        if !id.is_empty() && !into.iter().any(|(i, _)| *i == id) {
            into.push((id.clone(), text(a, "text").replace('&', "")));
        }
    }
}

/// The curated Kdenlive catalog, offline (`list-actions --json`).
pub fn kdenlive_catalog(daemon: Option<&Path>) -> Vec<(String, String)> {
    let mut actions = Vec::new();
    if let Some(out) = daemon.and_then(|d| Command::new(d).args(["list-actions", "--json"]).output().ok()) {
        if let Ok(v) = serde_json::from_slice::<Value>(&out.stdout) {
            parse_actions(&v, &mut actions);
        }
    }
    actions
}

fn follow_file(path: &Path, on_event: &impl Fn(InputEvent) -> bool) {
    use std::io::{Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    // A regular file is followed from its end like `tail -f`; a FIFO blocks.
    let _ = file.seek(SeekFrom::End(0));
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => std::thread::sleep(Duration::from_millis(100)),
            Ok(_) => {
                if let Some(event) = parse_event(&line) {
                    if !on_event(event) {
                        return;
                    }
                }
            }
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn parses_keypad_ctl_status() {
        let status = parse_status(
            r#"{"present":"yes","state":"ready","count":"1","tooltip":"",
                "devices":[{"path":"1-5","vid":"1189","pid":"8890","manufacturer":"OpenMacroPad",
                  "product":"Control Surface 15+3","serial":"key153","bcd":"0200","firmware":"open",
                  "firmware_label":"Open firmware (control-surface)",
                  "layout":{"board":"15+3","keys":15,"knobs":3,"cols":5,"source":"device","supported":true}}],
                "bootloaders":[],"service":{"installed":true,"state":"active"}}"#,
        )
        .unwrap();
        assert_eq!(status.pads.len(), 1);
        let pad = &status.pads[0];
        assert_eq!((pad.keys, pad.knobs, pad.cols), (15, 3, 5));
        assert_eq!((pad.firmware.as_str(), pad.layout_source.as_str()), ("open", "device"));
        assert!(status.service_installed);
        assert_eq!(status.service_state, "active");
        assert!(parse_status("not json").is_err());
    }

    #[test]
    fn control_ids_are_keys_then_knobs() {
        assert_eq!(control_ids(3, 1), ["key1", "key2", "key3", "knob1"]);
    }

    #[test]
    fn parses_live_events_and_flash_steps() {
        assert_eq!(
            parse_event(r#"{"slot":"knob2","event":"cw","delta":1}"#),
            Some(InputEvent { control: "knob2".into(), event: "cw".into() })
        );
        assert_eq!(parse_event(r#"{"slot":"key7.press"}"#).unwrap().event, "press");
        assert!(parse_event(r#"{"slot":"mouse1","event":"press"}"#).is_none());
        assert!(parse_event("garbage").is_none());
        let step = parse_flash_line(r#"{"step":"verify","status":"simulated","detail":"x","dry_run":true}"#).unwrap();
        assert_eq!((step.step.as_str(), step.status.as_str()), ("verify", "simulated"));
        let images = parse_images(
            r#"[{"path":"/fw/a.bin","name":"Open firmware","version":"2.0.0","size":6144,"sha256":"ab","verified":true,"mismatch":false}]"#,
        );
        assert_eq!(images[0].title(), "Open firmware 2.0.0");
    }

    #[test]
    fn parses_daemon_features() {
        let f = parse_features(
            r#"{"daemonVersion":"0.2.0","slots":{"maxKeys":16,"maxKnobs":3},"mouseNames":["left","right"]}"#,
        )
        .unwrap();
        assert_eq!(f, Features { max_keys: 16, max_knobs: 3, mouse: true });
        assert!(parse_features(r#"{"slots":{}}"#).is_none());
        let mut actions = Vec::new();
        parse_actions(
            &serde_json::json!({"actions":[{"id":"mark_in","text":"Set &In Point"},{"id":"mark_in","text":"dup"}]}),
            &mut actions,
        );
        assert_eq!(actions, [("mark_in".to_string(), "Set In Point".to_string())]);
    }

    fn fake_daemon(dir: &Path, script: &str) -> PathBuf {
        let path = dir.join("control-surfaced");
        let tmp = dir.join(".control-surfaced.tmp");
        std::fs::write(&tmp, format!("#!/bin/sh\n{script}")).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        // A test thread that forked while the file was open for writing keeps
        // it "busy" until its child execs; wait that out.
        for _ in 0..200 {
            match Command::new(&path).arg("--help").output() {
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => std::thread::sleep(Duration::from_millis(10)),
                _ => break,
            }
        }
        path
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smplos-keypad-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_validates_backs_up_and_replaces_atomically() {
        let dir = temp_dir("save");
        let config = dir.join("cfg/config.jsonc");
        let daemon = fake_daemon(
            &dir,
            "if grep -q BAD \"$3\"; then echo \"config $3: profile g: unknown key 'BAD'\" >&2; exit 2; fi\n\
             echo ok; echo 'warning: profile g key1: action bindings only work in a profile with \"kdenlive\": true'\n",
        );
        let first = save_config(&config, "{\"a\": 1}\n", Some(&daemon), None).unwrap();
        assert_eq!(first.backup, None);
        assert!(matches!(first.validation, Validation::Ok { ref warnings } if warnings.len() == 1));

        let err = save_config(&config, "BAD", Some(&daemon), Some("{\"a\": 1}\n")).unwrap_err();
        assert_eq!(err, "Not saved: profile g: unknown key 'BAD'");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{\"a\": 1}\n");

        let conflict = save_config(&config, "{\"a\": 3}\n", Some(&daemon), Some("{\"a\": 0}\n")).unwrap_err();
        assert!(conflict.contains("changed on disk"), "{conflict}");
        assert!(save_config(&config, "{}\n", None, None).unwrap_err().contains("changed on disk"));
        let second = save_config(&config, "{\"a\": 2}\n", Some(&daemon), Some("{\"a\": 1}\n")).unwrap();
        let backup = second.backup.unwrap();
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "{\"a\": 1}\n");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{\"a\": 2}\n");
        let leftovers: Vec<_> = std::fs::read_dir(config.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".config"))
            .collect();
        assert!(leftovers.is_empty());

        let mut current = "{\"a\": 2}\n".to_string();
        for i in 0..12 {
            let next = format!("{{\"n\": {i}}}\n");
            save_config(&config, &next, None, Some(&current)).unwrap();
            current = next;
        }
        assert_eq!(std::fs::read_dir(dir.join("cfg/backups")).unwrap().count(), BACKUPS_KEPT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mouse_support_is_probed_through_check_config() {
        let dir = temp_dir("probe");
        let rejecting = fake_daemon(&dir, "echo 'config x: key1: binding object needs one of keys' >&2; exit 2\n");
        assert!(!probe_mouse(Some(&rejecting)));
        let accepting = fake_daemon(&dir, "echo ok\n");
        assert!(probe_mouse(Some(&accepting)));
        assert!(!probe_mouse(None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
