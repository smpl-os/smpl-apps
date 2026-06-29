// XR glasses (xr-workspace) integration for the Settings app.
//
// The Settings tab is config-driven: it reads and writes the renderer's
// config.json so changes persist even when the renderer is not running. When
// the renderer IS running, it also pokes the control socket so changes apply
// live (the renderer's `reload`/`recenter`/`head_tracking` commands).
//
// No environment variables drive behaviour here beyond locating XDG dirs and
// the per-session runtime socket — the config file is the source of truth.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// VITURE USB vendor id (covers One / Pro / Lite — they share the vendor).
const VITURE_VID: &str = "35ca";

/// Snapshot of the renderer config + live detection, pushed into the UI.
#[derive(Clone, Debug)]
pub struct XrState {
    pub glasses_connected: bool,
    pub running: bool,
    pub live: bool,
    pub enabled: bool,
    pub monitor_count: i32,
    pub layout_index: i32, // 0=arc, 1=flat
    pub radius: f32,
    pub fov: f32,
    pub curvature: f32,
    pub smoothing: f32,
    pub stereo: bool,
    pub capture_index: i32, // 0=auto, 1=ext, 2=wlr
    pub prefer_dmabuf: bool,
    pub headtracking: bool,
}

impl Default for XrState {
    fn default() -> Self {
        XrState {
            glasses_connected: false,
            running: false,
            live: false,
            enabled: false,
            monitor_count: 3,
            layout_index: 0,
            radius: 2.0,
            fov: 46.0,
            curvature: 0.0,
            smoothing: 0.0,
            stereo: false,
            capture_index: 0,
            prefer_dmabuf: true,
            headtracking: false,
        }
    }
}

// ── Paths ────────────────────────────────────────────────────────────────────

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("xr-workspace/config.json")
}

fn hotplug_conf_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("xr-workspace/hotplug.conf")
}

fn socket_path() -> PathBuf {
    let run = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(run).join("xr-workspace.sock")
}

// ── Detection ────────────────────────────────────────────────────────────────

/// True if a VITURE USB device is currently enumerated.
pub fn glasses_connected() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else { return false };
    for e in entries.flatten() {
        let vid = e.path().join("idVendor");
        if let Ok(s) = std::fs::read_to_string(&vid) {
            if s.trim().eq_ignore_ascii_case(VITURE_VID) {
                return true;
            }
        }
    }
    false
}

/// True if the renderer process is alive.
pub fn renderer_running() -> bool {
    Command::new("pgrep")
        .args(["-x", "xr-workspace"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// True if the control socket is reachable.
pub fn socket_live() -> bool {
    let p = socket_path();
    p.exists() && UnixStream::connect(&p).is_ok()
}

/// True if the auto-launch user service is enabled.
pub fn service_enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", "xr-glasses.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false)
}

// ── Control socket IPC ───────────────────────────────────────────────────────

/// Send one JSON command and return the parsed reply (if any).
pub fn send_command(cmd: &Value) -> Option<Value> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();
    stream.set_write_timeout(Some(Duration::from_millis(500))).ok();
    let mut line = cmd.to_string();
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    serde_json::from_str(buf.trim()).ok()
}

/// Tell a running renderer to re-read config.json.
fn reload_if_live() {
    if socket_live() {
        let _ = send_command(&json!({ "cmd": "reload" }));
    }
}

// ── Config read/write ────────────────────────────────────────────────────────

fn load_config_value() -> Value {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn save_config_value(v: &Value) -> std::io::Result<()> {
    let path = config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let pretty = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
    // Atomic-ish write: temp + rename.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, pretty.as_bytes())?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Mutate the config with `f`, persist it, then reload a running renderer.
fn edit_config<F: FnOnce(&mut Value)>(f: F) {
    let mut v = load_config_value();
    if !v.is_object() {
        v = json!({});
    }
    f(&mut v);
    let _ = save_config_value(&v);
    reload_if_live();
}

fn as_f32(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(default)
}

fn as_bool(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(default)
}

/// Read the launch monitor count from hotplug.conf (XR_WORKSPACE_ARGS).
fn read_monitor_count() -> i32 {
    if let Ok(s) = std::fs::read_to_string(hotplug_conf_path()) {
        for line in s.lines() {
            if let Some(idx) = line.find("--monitors") {
                let rest = &line[idx + "--monitors".len()..];
                if let Some(tok) = rest.split_whitespace().next() {
                    if let Ok(n) = tok.trim_matches(|c| c == '"' || c == '\'').parse::<i32>() {
                        return n;
                    }
                }
            }
        }
    }
    3
}

// ── Aggregate state for the UI ───────────────────────────────────────────────

pub fn load_state() -> XrState {
    let cfg = load_config_value();
    let layout = cfg.get("layout").cloned().unwrap_or_else(|| json!({}));
    let layout_mode = layout.get("mode").and_then(|x| x.as_str()).unwrap_or("arc");
    let capture = cfg.get("capture_protocol").and_then(|x| x.as_str()).unwrap_or("auto");

    // Curvature is per-monitor; surface the first monitor's value as the global.
    let curvature = cfg
        .get("monitors")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .map(|m| m.get("curvature").and_then(|c| c.as_f64()).unwrap_or(0.0) as f32)
        .unwrap_or(0.0);

    let ht_enabled = cfg
        .get("head_tracking")
        .and_then(|h| h.get("enabled"))
        .and_then(|e| e.as_bool())
        .unwrap_or(false);

    XrState {
        glasses_connected: glasses_connected(),
        running: renderer_running(),
        live: socket_live(),
        enabled: service_enabled(),
        monitor_count: read_monitor_count(),
        layout_index: if layout_mode == "flat" { 1 } else { 0 },
        radius: layout.get("radius").and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(2.0),
        fov: as_f32(&cfg, "fov_deg", 46.0),
        curvature,
        smoothing: as_f32(&cfg, "smoothing", 0.0),
        stereo: as_bool(&cfg, "stereo", false),
        capture_index: match capture {
            "ext" => 1,
            "wlr" => 2,
            _ => 0,
        },
        prefer_dmabuf: as_bool(&cfg, "prefer_dmabuf", true),
        headtracking: ht_enabled,
    }
}

// ── Setters (config + live) ──────────────────────────────────────────────────

pub fn set_fov(deg: f32) {
    edit_config(|v| { v["fov_deg"] = json!(deg); });
}

pub fn set_smoothing(s: f32) {
    edit_config(|v| { v["smoothing"] = json!(s); });
}

pub fn set_stereo(on: bool) {
    edit_config(|v| { v["stereo"] = json!(on); });
}

pub fn set_prefer_dmabuf(on: bool) {
    edit_config(|v| { v["prefer_dmabuf"] = json!(on); });
}

pub fn set_capture(index: i32) {
    let proto = match index {
        1 => "ext",
        2 => "wlr",
        _ => "auto",
    };
    edit_config(|v| { v["capture_protocol"] = json!(proto); });
}

pub fn set_layout(index: i32) {
    let mode = if index == 1 { "flat" } else { "arc" };
    edit_config(|v| {
        if !v.get("layout").map(|l| l.is_object()).unwrap_or(false) {
            v["layout"] = json!({});
        }
        v["layout"]["mode"] = json!(mode);
    });
}

pub fn set_radius(r: f32) {
    edit_config(|v| {
        if !v.get("layout").map(|l| l.is_object()).unwrap_or(false) {
            v["layout"] = json!({});
        }
        v["layout"]["radius"] = json!(r);
    });
}

pub fn set_curvature(c: f32) {
    edit_config(|v| {
        if let Some(arr) = v.get_mut("monitors").and_then(|m| m.as_array_mut()) {
            for m in arr.iter_mut() {
                if m.is_object() {
                    m["curvature"] = json!(c);
                }
            }
        }
    });
}

pub fn set_headtracking(on: bool) {
    edit_config(|v| {
        if !v.get("head_tracking").map(|h| h.is_object()).unwrap_or(false) {
            v["head_tracking"] = json!({});
        }
        v["head_tracking"]["enabled"] = json!(on);
    });
    // Apply live without a full reload if possible.
    if socket_live() {
        let _ = send_command(&json!({ "cmd": "head_tracking", "enabled": on }));
    }
}

/// Monitor count is a launch-time arg consumed by the hotplug watcher; persist
/// it to hotplug.conf so the next connect uses it.
pub fn set_monitors(n: i32) {
    let n = if n == 1 { 1 } else { 3 };
    let path = hotplug_conf_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = format!("XR_WORKSPACE_ARGS=\"--monitors {n}\"\n");
    let _ = std::fs::write(&path, line);
}

pub fn recenter() {
    if socket_live() {
        let _ = send_command(&json!({ "cmd": "recenter" }));
    }
}

/// Enable/disable the auto-launch service and start/stop the renderer now.
pub fn set_enabled(on: bool) {
    if on {
        let _ = Command::new("systemctl")
            .args(["--user", "enable", "--now", "xr-glasses.service"])
            .status();
        // Start immediately if glasses are already connected.
        let _ = Command::new("xr-glasses-hotplugd").arg("--once").spawn();
    } else {
        let _ = Command::new("systemctl")
            .args(["--user", "disable", "--now", "xr-glasses.service"])
            .status();
        let _ = Command::new("pkill").args(["-x", "xr-workspace"]).status();
    }
}
