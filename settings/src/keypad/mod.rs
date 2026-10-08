//! Settings > Keypad: CH552 macro keypads driven by the control-surface daemon.
//!
//! Device detection, layouts and firmware flashing go through smplOS's
//! `keypad-ctl` (sysfs only; flashing is a dry run unless explicitly enabled).
//! The mapping editor writes the daemon's JSONC config after validating it
//! with `control-surfaced check-config` and backing up the previous file.

pub mod config;
pub mod icons;
pub mod json;
pub mod ui;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use serde_json::Value;

pub const SERVICE: &str = "control-surface.service";
const BACKUPS_KEPT: usize = 10;

// ── Devices (sysfs only: the keypad is never opened) ─────────────────────────

const PAD_ID: (&str, &str) = ("1189", "8890");
const BOOTLOADER_IDS: [(&str, &str); 2] = [("4348", "55e0"), ("1a86", "55e0")];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pad {
    pub path: String,
    /// The daemon's names: `control-surface` (ours), `openmacropad`, `stock`, `unknown`.
    pub firmware: String,
    pub firmware_label: String,
    /// From bcdDevice for our firmware, e.g. "2.1".
    pub version: String,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    /// Board profile the keypad names itself ("Control Surface 15+3" -> sy181-15k3e).
    pub board: String,
    /// Keys and knobs the keypad describes, if it does (our firmware).
    pub described: Option<(usize, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    /// `control-surfaced` is installed (PATH, ~/.local/bin or /usr/bin).
    pub installed: bool,
    /// control-surface.service: LoadState, ActiveState, ConditionResult.
    pub load: String,
    pub active: String,
    pub condition_failed: bool,
    /// org.smplos.ControlSurface is on the session bus (systemd or not).
    pub bus: bool,
    /// A `control-surfaced run` process exists (e.g. started by hand).
    pub process: bool,
}

impl AppState {
    pub fn running(&self) -> bool {
        self.bus || self.process || self.active == "active"
    }
}

/// The daemon's effective layout, from its Settings API.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DaemonLayout {
    pub id: String,
    pub name: String,
    /// firmware | config | hardware-map | measured | template
    pub source: String,
    pub keys: usize,
    pub knobs: usize,
    pub columns: usize,
    /// Controls the pad reads one at a time (`oneAtATime`; sy181: keys 2–15
    /// and the knob presses): pressing one reports another held one released.
    pub one_at_a_time: Vec<String>,
}

/// How the running keypad app reads the keypad (`GetStatus().input`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InputStatus {
    /// `device.input` of the config it runs: auto | evdev | raw.
    pub configured: String,
    /// raw | evdev-chords
    pub mode: String,
    /// Raw-mode counters since it started.
    pub raw_events: u64,
    pub seq_gaps: u64,
    pub lost_events: u64,
    pub restored: u64,
    pub heartbeat_misses: u64,
    pub raw_drops: u64,
    /// Keymap (evdev) events, and those that arrived while raw mode was on.
    pub keymap_events: u64,
    pub keymap_while_raw: u64,
    /// Config warnings about the layout (raw input stays off on a mismatch).
    pub layout_warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub pads: Vec<Pad>,
    pub bootloaders: usize,
    pub app: AppState,
    pub layout: Option<DaemonLayout>,
    /// The running app pushes the cheatsheet into the bar (GetStatus).
    pub sheet_push: Option<bool>,
    /// Input mode and its health; `None` for keypad apps that predate it.
    pub input: Option<InputStatus>,
    /// The config file the running keypad app uses (`GetStatus().config.path`).
    pub config_path: Option<String>,
}

fn sysfs_root() -> PathBuf {
    std::env::var_os("SMPLOS_KEYPAD_SYSFS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/sys/bus/usb/devices"))
}

fn attr(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Same rules as control-surfaced (usbinfo.cpp classifyFirmware).
pub fn classify_pad(manufacturer: &str, product: &str, bcd: &str) -> (String, String, String, String) {
    let (kind, label) = if manufacturer == "OpenMacroPad" && product.starts_with("Control Surface") {
        ("control-surface", "Open firmware (control-surface)")
    } else if manufacturer == "SY181" {
        ("openmacropad", "Open firmware (OpenMacroPad)")
    } else if manufacturer == "wch.cn" {
        ("stock", "Stock firmware")
    } else {
        ("unknown", "Unknown firmware")
    };
    let mut version = String::new();
    let mut board = String::new();
    if kind == "control-surface" {
        let bcd = format!("{bcd:0>4}");
        let major = bcd[..2].parse::<u32>().unwrap_or(0);
        let minor = bcd[2..4].parse::<u32>().unwrap_or(0);
        version = format!("{major}.{minor}");
        if product == "Control Surface 15+3" {
            board = "sy181-15k3e".into();
        }
    }
    (kind.into(), label.into(), version, board)
}

/// "Control Surface 12+2" -> (12, 2).
pub fn described_layout(kind: &str, product: &str) -> Option<(usize, usize)> {
    if kind != "control-surface" {
        return None;
    }
    let tail = product.rsplit(' ').next()?;
    let (keys, knobs) = tail.split_once('+')?;
    let (keys, knobs) = (keys.parse::<usize>().ok()?, knobs.parse::<usize>().ok()?);
    (keys > 0 && keys <= 32 && knobs <= 4).then_some((keys, knobs))
}

/// Connected keypads and WCH bootloaders, from USB device attributes.
pub fn scan_sysfs(root: &Path) -> (Vec<Pad>, usize) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return (Vec::new(), 0);
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| !n.contains(':')))
        .collect();
    dirs.sort();
    let (mut pads, mut loaders) = (Vec::new(), 0);
    for dir in dirs {
        let id = (attr(&dir, "idVendor").to_lowercase(), attr(&dir, "idProduct").to_lowercase());
        if (id.0.as_str(), id.1.as_str()) == PAD_ID {
            let manufacturer = attr(&dir, "manufacturer");
            let product = attr(&dir, "product");
            let (firmware, firmware_label, version, board) =
                classify_pad(&manufacturer, &product, &attr(&dir, "bcdDevice"));
            pads.push(Pad {
                path: dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                described: described_layout(&firmware, &product),
                firmware,
                firmware_label,
                version,
                serial: attr(&dir, "serial"),
                manufacturer,
                product,
                board,
            });
        } else if BOOTLOADER_IDS.iter().any(|(v, p)| (id.0.as_str(), id.1.as_str()) == (*v, *p)) {
            loaders += 1;
        }
    }
    (pads, loaders)
}

/// A `control-surfaced` process running the daemon (not a one-shot command).
fn daemon_process_running(proc_root: &Path) -> bool {
    let Ok(procs) = std::fs::read_dir(proc_root) else {
        return false;
    };
    procs.flatten().any(|p| {
        // /proc/PID/comm is cut to 15 bytes ("control-surface"); match argv[0].
        let cmdline = std::fs::read(p.path().join("cmdline")).unwrap_or_default();
        let mut argv = cmdline.split(|b| *b == 0);
        let program = argv.next().unwrap_or_default();
        if program.rsplit(|b| *b == b'/').next() != Some(&b"control-surfaced"[..]) {
            return false;
        }
        let mut args = argv.filter(|a| !a.is_empty());
        match args.find(|a| !a.starts_with(b"-")) {
            None => true,
            Some(cmd) => cmd == b"run",
        }
    })
}

fn unit_state() -> (String, String, bool) {
    let Ok(out) = Command::new("systemctl")
        .args(["--user", "show", SERVICE, "-p", "LoadState", "-p", "ActiveState", "-p", "ConditionResult"])
        .output()
    else {
        return (String::new(), String::new(), false);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_default()
            .to_string()
    };
    let active = get("ActiveState");
    let condition_failed = get("ConditionResult") == "no" && active != "active";
    (get("LoadState"), active, condition_failed)
}

pub fn load_status() -> Status {
    let (pads, bootloaders) = scan_sysfs(&sysfs_root());
    let (load, active, condition_failed) = unit_state();
    let bus = api_available();
    let app = AppState {
        installed: daemon_binary().is_some(),
        load,
        active,
        condition_failed,
        bus,
        process: !bus
            && daemon_process_running(&std::env::var_os("SMPLOS_KEYPAD_PROC").map_or_else(|| PathBuf::from("/proc"), PathBuf::from)),
    };
    let mut status = Status { pads, bootloaders, app, layout: None, sheet_push: None, input: None, config_path: None };
    if status.app.bus {
        status.layout = daemon_layout();
        if let Some(daemon) = daemon_status() {
            apply_daemon_status(&mut status, &daemon);
        }
    }
    status
}

fn daemon_status() -> Option<Value> {
    let reply = bus()?
        .call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "GetStatus", &())
        .ok()?;
    serde_json::from_str(&reply.body().deserialize::<String>().ok()?).ok()
}

/// The daemon knows the firmware's full version (GET_INFO: "2.0.1"; sysfs
/// only has "2.0") and whether its cheatsheet push to the bar is on.
fn apply_daemon_status(status: &mut Status, v: &Value) {
    let version = v.get("device").and_then(|d| d.get("firmware")).map(|f| text(f, "version")).unwrap_or_default();
    if let (Some(pad), false) = (status.pads.first_mut(), version.is_empty()) {
        if pad.firmware == "control-surface" {
            pad.version = version;
        }
    }
    status.sheet_push = v
        .get("cheatsheet")
        .and_then(|c| c.get("eww"))
        .and_then(|e| e.get("enabled"))
        .and_then(Value::as_bool);
    status.config_path = v.get("config").map(|c| text(c, "path")).filter(|p| !p.is_empty());
    status.input = v.get("input").filter(|i| i.is_object()).map(|i| {
        let n = |o: Option<&Value>, k: &str| o.and_then(|o| o.get(k)).and_then(Value::as_u64).unwrap_or(0);
        let raw = i.get("raw");
        let evdev = i.get("evdev");
        InputStatus {
            configured: text(i, "configured"),
            mode: text(i, "mode"),
            raw_events: n(raw, "events"),
            seq_gaps: n(raw, "seqGaps"),
            lost_events: n(raw, "lostEvents"),
            restored: n(raw, "reconciledDowns") + n(raw, "reconciledUps") + n(raw, "reconciledDetents"),
            heartbeat_misses: n(raw, "heartbeatMisses"),
            raw_drops: n(raw, "rawDrops"),
            keymap_events: n(evdev, "events"),
            keymap_while_raw: n(evdev, "whileRaw"),
            layout_warnings: v
                .get("config")
                .and_then(|c| c.get("warnings"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|w| w.starts_with("layout:"))
                .map(String::from)
                .collect(),
        }
    });
}

/// What the Advanced section says about input: (what's in use now, a health
/// line, whether it looks healthy). `firmware` is the pad's firmware type and
/// version, `configured` the mode chosen in Settings (maybe not saved yet).
pub fn input_summary(input: Option<&InputStatus>, running: bool, firmware: Option<(&str, &str)>, configured: &str) -> (String, String, bool) {
    let Some(i) = input.filter(|_| running) else {
        let why = if running { "the keypad app is too old to say" } else { "the keypad app isn't running" };
        return (format!("In use now: unknown ({why})."), String::new(), true);
    };
    let raw = i.mode == "raw";
    let now = if raw {
        "In use now: Raw (the firmware's own events).".to_string()
    } else {
        "In use now: Keymap (the keys the keypad types).".to_string()
    };
    if raw {
        let mut parts = vec![format!("{} events", i.raw_events)];
        // Losses the snapshots restored are the design working; unrestored
        // losses or leaving raw mode are worth a look.
        let healthy = i.lost_events <= i.restored && i.raw_drops == 0;
        if i.seq_gaps == 0 && i.lost_events == 0 {
            parts.push("none lost".into());
        } else {
            parts.push(format!("{} lost, {} restored from snapshots", i.lost_events, i.restored));
        }
        if i.raw_drops > 0 {
            parts.push(format!("left raw mode {} times (the keymap took over meanwhile)", i.raw_drops));
        }
        if i.heartbeat_misses > 0 {
            parts.push(format!("{} late heartbeats", i.heartbeat_misses));
        }
        return (now, format!("Since the keypad app started: {}.", parts.join(", ")), healthy);
    }
    let mut health = format!("Since the keypad app started: {} events.", i.keymap_events);
    // "auto" means raw on the open firmware 2.0.2+ (control-surface 7f06731).
    let raw_capable = firmware.is_some_and(|(kind, version)| {
        kind == "control-surface" && config::version_at_least(version, config::RAW_MIN_FIRMWARE) == Some(true)
    });
    let wants_raw = configured == "raw" || i.configured == "raw" || (raw_capable && i.configured == "auto" && configured == "auto");
    if wants_raw {
        let reason = match firmware {
            _ if !i.layout_warnings.is_empty() => format!(
                "Raw stays off because the layout chosen here doesn't match the keypad's firmware ({}).",
                i.layout_warnings[0].trim_start_matches("layout:").trim()
            ),
            Some((kind, _)) if kind != "control-surface" => "Raw needs the open control-surface firmware; this keypad uses its keymap.".into(),
            Some((_, version)) if config::version_at_least(version, config::RAW_MIN_FIRMWARE) == Some(false) => {
                format!("Raw needs firmware 2.0.2 or newer; this keypad has {version}, so the keymap is used.")
            }
            _ if configured == "raw" && i.configured != "raw" => "Raw starts once you save.".into(),
            _ => "Raw isn't on yet; the keypad app tries again when the keypad reconnects.".into(),
        };
        health = format!("{health} {reason}");
    }
    (now, health, !wants_raw || configured != i.configured)
}

/// What to say about the keypad app, and whether to offer Start.
pub fn app_summary(app: &AppState, keypad_present: bool) -> (String, bool) {
    if app.running() {
        let how = if app.active == "active" {
            ""
        } else {
            " (started outside systemd)"
        };
        let api = if app.bus {
            ""
        } else {
            " This version has no Settings interface, so live input is unavailable."
        };
        return (format!("Keypad app: running{how}.{api}"), false);
    }
    if !app.installed {
        return ("Keypad app: not installed (package control-surface).".into(), false);
    }
    match app.active.as_str() {
        "activating" | "reloading" => return ("Keypad app: starting…".into(), false),
        "failed" => {
            return (
                "Keypad app: failed. Details: journalctl --user -u control-surface".into(),
                keypad_present,
            )
        }
        _ => {}
    }
    if app.condition_failed {
        return (
            "Keypad app: not started: /usr/bin/control-surfaced is missing (package control-surface).".into(),
            false,
        );
    }
    if app.load == "not-found" {
        return ("Keypad app: not running; control-surface.service is missing (update smplOS).".into(), false);
    }
    if keypad_present {
        ("Keypad app: not running.".into(), true)
    } else {
        ("Keypad app: not running. It starts by itself when a keypad is plugged in.".into(), false)
    }
}

pub fn start_app() -> Result<(), String> {
    let out = Command::new("systemctl")
        .args(["--user", "start", SERVICE])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Keys first (row-major), then knobs in a column to the right.
pub fn control_ids(keys: usize, knobs: usize) -> Vec<String> {
    (1..=keys)
        .map(|k| format!("key{k}"))
        .chain((1..=knobs).map(|k| format!("knob{k}")))
        .collect()
}

// ── Variants (the daemon's board profiles) ──────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub keys: usize,
    pub knobs: usize,
    /// Key columns (knobs sit in one extra column to the right).
    pub columns: usize,
}

/// control-surfaced's built-in board profiles (boardprofile.cpp), in menu order.
pub const BUILTIN_VARIANTS: &[(&str, &str, usize, usize, usize)] = &[
    ("sy181-15k3e", "15 keys, 3 knobs (SY181 \"12+3\" board)", 15, 3, 5),
    ("generic-3k", "3 keys", 3, 0, 3),
    ("generic-3k1e", "3 keys, 1 knob", 3, 1, 3),
    ("generic-6k1e", "6 keys, 1 knob", 6, 1, 3),
    ("generic-10k", "10 keys", 10, 0, 5),
    ("generic-12k2e", "12 keys, 2 knobs", 12, 2, 4),
    ("generic-12k3e", "12 keys, 3 knobs", 12, 3, 4),
    ("generic-16k3e", "16 keys, 3 knobs", 16, 3, 4),
];

/// The daemon's column rule for grids it builds itself (profileForControls).
pub fn default_columns(keys: usize) -> usize {
    if keys >= 15 || keys == 10 {
        5
    } else if keys >= 7 {
        4
    } else {
        keys.clamp(1, 3)
    }
}

pub fn builtin_variants() -> Vec<Variant> {
    BUILTIN_VARIANTS
        .iter()
        .map(|(id, name, keys, knobs, columns)| Variant {
            id: id.to_string(),
            name: name.to_string(),
            keys: *keys,
            knobs: *knobs,
            columns: *columns,
        })
        .collect()
}

/// Board profiles from `features --json` (`layouts.builtin`), in the built-in
/// menu order, with the built-in column counts; new ids are appended.
pub fn parse_variants(features: &str) -> Option<Vec<Variant>> {
    let v: Value = serde_json::from_str(features.trim()).ok()?;
    let listed = v.get("layouts")?.get("builtin")?.as_array()?;
    let builtin = builtin_variants();
    let mut out: Vec<Variant> = Vec::new();
    for b in &builtin {
        if listed.iter().any(|l| text(l, "id") == b.id) {
            out.push(b.clone());
        }
    }
    for l in listed {
        let id = text(l, "id");
        if id.is_empty() || out.iter().any(|v| v.id == id) {
            continue;
        }
        let keys = count(l, "keys");
        out.push(Variant { name: text(l, "name"), keys, knobs: count(l, "knobs"), columns: default_columns(keys), id });
    }
    (!out.is_empty()).then_some(out)
}

pub fn variants(daemon: Option<&Path>) -> Vec<Variant> {
    daemon
        .and_then(|d| Command::new(d).args(["features", "--json"]).output().ok())
        .filter(|o| o.status.success())
        .and_then(|o| parse_variants(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_else(builtin_variants)
}

pub fn parse_daemon_layout(json: &str) -> Option<DaemonLayout> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let l = v.get("layout").filter(|l| l.is_object()).unwrap_or(&v);
    let keys = l.get("keys")?.as_array()?;
    let knobs = l.get("knobs").and_then(Value::as_array).map_or(0, Vec::len);
    let columns = keys
        .iter()
        .filter_map(|k| k.get("column").and_then(Value::as_u64))
        .max()
        .map_or(1, |c| c as usize + 1);
    Some(DaemonLayout {
        id: text(l, "id"),
        name: text(l, "name"),
        source: text(l, "source"),
        keys: keys.len(),
        knobs,
        columns,
        one_at_a_time: strings_at(l, "oneAtATime"),
    })
}

fn strings_at(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(String::from).collect()
}

/// A warning `check-config --json` gives about one place in the config.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LintWarning {
    pub profile: String,
    /// Empty for the profile's own bindings.
    pub layer: String,
    /// Empty for the layer (or profile) as a whole.
    pub slot: String,
    pub message: String,
}

/// What the keypad app says about an (unsaved) config.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lint {
    /// Controls of the layout in effect read one at a time.
    pub one_at_a_time: Vec<String>,
    pub warnings: Vec<LintWarning>,
}

pub fn parse_lint(json: &str) -> Option<Lint> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    if !v.is_object() {
        return None;
    }
    let warnings = v
        .get("warningDetails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|w| LintWarning { profile: text(w, "profile"), layer: text(w, "layer"), slot: text(w, "slot"), message: text(w, "message") })
        .filter(|w| !w.message.is_empty())
        .collect();
    Some(Lint { one_at_a_time: v.get("layout").map(|l| strings_at(l, "oneAtATime")).unwrap_or_default(), warnings })
}

/// `control-surfaced check-config --json` on a temporary copy of `config_text`
/// next to the config (relative paths resolve the same). `None` without a
/// keypad app or for one that predates `--json`.
pub fn lint(daemon: Option<&Path>, scratch: &Path, config_text: &str) -> Option<Lint> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let daemon = daemon?;
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = scratch.join(format!(".config.jsonc.check-{}-{n}", std::process::id()));
    std::fs::create_dir_all(scratch).ok()?;
    std::fs::write(&file, config_text).ok()?;
    let out = Command::new(daemon).args(["check-config", "--json", "-c"]).arg(&file).output();
    let _ = std::fs::remove_file(&file);
    parse_lint(&String::from_utf8_lossy(&out.ok()?.stdout))
}

fn daemon_layout() -> Option<DaemonLayout> {
    let reply = bus()?
        .call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "GetLayout", &())
        .ok()?;
    parse_daemon_layout(&reply.body().deserialize::<String>().ok()?)
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn count(v: &Value, key: &str) -> usize {
    v.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
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
        .unwrap_or("the keypad app rejected the config");
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

// ── Cheatsheet preview ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetEntry {
    pub bound: bool,
    pub active: bool,
    pub label: String,
    pub state: String,
    /// The icon name the keypad app resolved (explicit or automatic); empty
    /// for none or an app that predates icons.
    pub icon: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetControl {
    pub control: String,
    pub row: usize,
    pub column: usize,
    /// Keys: one entry. Knobs: ccw, press, cw.
    pub entries: Vec<SheetEntry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SheetPreview {
    pub title: String,
    pub layers: Vec<String>,
    pub notice: String,
    pub keys: Vec<SheetControl>,
    pub knobs: Vec<SheetControl>,
}

fn sheet_entry(v: Option<&Value>) -> SheetEntry {
    let Some(v) = v else {
        return SheetEntry::default();
    };
    SheetEntry {
        bound: v.get("bound").and_then(Value::as_bool).unwrap_or(false),
        active: v.get("active").and_then(Value::as_bool).unwrap_or(false),
        label: text(v, "label"),
        state: text(v, "state"),
        icon: text(v, "icon"),
    }
}

/// The daemon's cheatsheet content (GetCheatsheet / cheatsheet --json).
pub fn parse_sheet(json: &str) -> Option<SheetPreview> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    if v.get("ok").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let place = |c: &Value| (count(c, "row"), count(c, "column"));
    let keys = v
        .get("keys")?
        .as_array()?
        .iter()
        .map(|k| {
            let (row, column) = place(k);
            SheetControl { control: text(k, "control"), row, column, entries: vec![sheet_entry(Some(k))] }
        })
        .collect();
    let knobs = v
        .get("knobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|k| {
            let (row, column) = place(k);
            let entries = ["ccw", "press", "cw"].iter().map(|e| sheet_entry(k.get(*e))).collect();
            SheetControl { control: text(k, "control"), row, column, entries }
        })
        .collect();
    Some(SheetPreview {
        title: text(&v, "title"),
        layers: v
            .get("layers")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        notice: text(&v, "notice"),
        keys,
        knobs,
    })
}

/// What the cheatsheet shows for `window_class` with `config_text` (unsaved
/// edits included): `control-surfaced cheatsheet --json --window CLASS -c FILE`
/// on a temporary copy, else the running app's `GetCheatsheetFor` (saved
/// config). `context` is a Kdenlive context JSON object, or "". The copy is
/// written to `scratch` (the config's directory, so relative paths resolve).
pub fn sheet_preview(
    daemon: Option<&Path>,
    scratch: &Path,
    config_text: &str,
    window_class: &str,
    context: &str,
) -> Result<SheetPreview, String> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(daemon) = daemon {
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file = scratch.join(format!(".config.jsonc.preview-{}-{n}", std::process::id()));
        if std::fs::create_dir_all(scratch).is_ok() && std::fs::write(&file, config_text).is_ok() {
            let mut cmd = Command::new(daemon);
            cmd.args(["cheatsheet", "--json", "--window", window_class]).arg("-c").arg(&file);
            if !context.is_empty() {
                cmd.args(["--context", context]);
            }
            let out = cmd.output();
            let _ = std::fs::remove_file(&file);
            if let Ok(out) = out {
                if out.status.success() {
                    if let Some(sheet) = parse_sheet(&String::from_utf8_lossy(&out.stdout)) {
                        return Ok(sheet);
                    }
                } else {
                    let err = String::from_utf8_lossy(&out.stderr);
                    let line = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").to_string();
                    if line.starts_with("config ") {
                        return Err(line.split_once(": ").map_or(line.clone(), |(_, m)| m.to_string()));
                    }
                }
            }
        }
    }
    let reply = bus()
        .ok_or("no session bus")?
        .call_method(
            Some(DBUS_SERVICE),
            DBUS_PATH,
            Some(DBUS_INTERFACE),
            "GetCheatsheetFor",
            &(window_class, "", context),
        )
        .map_err(|_| "the cheatsheet preview needs a newer keypad app".to_string())?;
    let json: String = reply.body().deserialize().map_err(|e| e.to_string())?;
    parse_sheet(&json).ok_or_else(|| "the keypad app sent no cheatsheet".into())
}

/// Shows the real overlay (the running keypad app draws it through the bar).
pub fn show_sheet() -> Result<(), String> {
    bus()
        .ok_or("no session bus")?
        .call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "ShowCheatsheet", &())
        .map(|_| ())
        .map_err(|_| "the keypad app isn't running, or is too old to show the cheatsheet".into())
}

/// What `SetOption` did with one option.
#[derive(Clone, Debug, PartialEq)]
pub enum OptionResult {
    /// In the file (written, or already so) and applied. `backup` is the
    /// previous file's copy when it was written.
    Set { changed: bool, backup: Option<String> },
    /// Written, but the running keypad app refused to apply it.
    Unapplied(String),
    /// The keypad app can't set it (not on the bus, too old, unknown option):
    /// Settings writes the file itself.
    Unsupported,
    /// Refused (invalid-value, invalid-config, io); nothing written.
    Refused(String),
}

pub fn parse_set_option(json: &str) -> OptionResult {
    let Ok(v) = serde_json::from_str::<Value>(json.trim()) else {
        return OptionResult::Refused("the keypad app sent no answer".into());
    };
    if v.get("ok").and_then(Value::as_bool) == Some(true) {
        let backup = Some(text(&v, "backup")).filter(|b| !b.is_empty());
        return OptionResult::Set { changed: v.get("changed").and_then(Value::as_bool).unwrap_or(true), backup };
    }
    let error = v.get("error");
    let message = error.map(|e| text(e, "message")).filter(|m| !m.is_empty()).unwrap_or_else(|| "refused".into());
    match error.map(|e| text(e, "code")).as_deref() {
        Some("apply") => OptionResult::Unapplied(message),
        Some("unknown-option") => OptionResult::Unsupported,
        _ => OptionResult::Refused(message),
    }
}

/// `SetOption(key, value)`: the keypad app changes one value in place in its
/// config (comments kept, backup, validated) and applies it.
pub fn set_option(key: &str, value: &str) -> OptionResult {
    let Some(conn) = bus() else { return OptionResult::Unsupported };
    match conn.call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "SetOption", &(key, value)) {
        Ok(reply) => match reply.body().deserialize::<String>() {
            Ok(json) => parse_set_option(&json),
            Err(e) => OptionResult::Refused(e.to_string()),
        },
        // Not on the bus, or a keypad app without SetOption.
        Err(_) => OptionResult::Unsupported,
    }
}

/// Takes the overlay down (what a click on it does, too).
pub fn hide_sheet() -> Result<(), String> {
    bus()
        .ok_or("no session bus")?
        .call_method(Some(DBUS_SERVICE), DBUS_PATH, Some(DBUS_INTERFACE), "HideCheatsheet", &())
        .map(|_| ())
        .map_err(|_| "the keypad app isn't running".into())
}

/// A preview context with `held` (`"key1"`, `"key1+knob3"`) counted as held
/// down: control-surface's `"$held"` in GetCheatsheetFor / `--context`.
pub fn with_held(context: &str, held: &str) -> String {
    let mut v: serde_json::Map<String, Value> = serde_json::from_str(context).unwrap_or_default();
    v.insert("$held".into(), Value::String(held.into()));
    Value::Object(v).to_string()
}

/// Kdenlive contexts for the preview (the example config's layers).
pub const KDENLIVE_CONTEXTS: &[(&str, &str)] = &[
    ("No particular focus", ""),
    ("Timeline", r#"{"focus":"timeline"}"#),
    ("Clip monitor", r#"{"focus":"clipMonitor"}"#),
    ("Project monitor", r#"{"focus":"projectMonitor"}"#),
    ("Colour wheels", r#"{"colorWheels":true}"#),
    ("Effect parameter", r#"{"focus":"effectStack","param":{"target":"1"}}"#),
];

// ── Daemon capabilities (`features`, `list-actions`) ─────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Features {
    pub max_keys: usize,
    pub max_knobs: usize,
    pub mouse: bool,
    /// `{"cheatsheet": ...}` bindings, options and previews.
    pub cheatsheet: bool,
    /// What the daemon uses for cheatsheet options the config leaves out.
    pub sheet_defaults: config::SheetDefaults,
    /// The cheatsheet shows binding icons (`cheatsheet.icons`).
    pub icons: bool,
    /// What each `device.input` mode does, in the keypad app's words.
    pub input_modes: Vec<(String, String)>,
    /// Options the keypad app sets in place (`SetOption`; `features.options`).
    pub options: Vec<String>,
    /// The firmware reports turns while a knob is pressed ("shift" bindings);
    /// `None` when the keypad app doesn't say.
    pub shift_supported: Option<bool>,
    /// Layers that apply while a key is held (`"when": {"held": ...}`).
    pub held_layers: bool,
}

/// `cheatsheet.defaults` when the daemon has it, else read from its option
/// descriptions ("0.05..1, default 0.85"; "...; unset = 8000, ..."). Before
/// "unset = N" existed an unset autoHideMs meant until hidden.
fn sheet_defaults(sheet: Option<&Value>) -> config::SheetDefaults {
    let mut d = config::SheetDefaults::default();
    let Some(sheet) = sheet else { return d };
    let structured = sheet.get("defaults");
    let described = |key: &str, after: &str| {
        let text = sheet.get("options")?.get(key)?.as_str()?;
        let rest = text.split(after).nth(1).map(str::trim_start);
        Some(rest.map(|r| r.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>()))
    };
    if let Some(o) = structured.and_then(|s| s.get("opacity")).and_then(Value::as_f64) {
        d.opacity = o;
    } else if let Some(Some(o)) = described("opacity", "default") {
        d.opacity = o.parse().unwrap_or(d.opacity);
    }
    if let Some(ms) = structured.and_then(|s| s.get("autoHideMs")).and_then(Value::as_u64) {
        d.auto_hide_ms = ms as u32;
    } else if let Some(ms) = described("autoHideMs", "unset =") {
        d.auto_hide_ms = ms.and_then(|ms| ms.parse().ok()).unwrap_or(0);
    }
    d
}

pub fn parse_features(json: &str) -> Option<Features> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let slots = v.get("slots")?;
    Some(Features {
        max_keys: slots.get("maxKeys").and_then(Value::as_u64)? as usize,
        max_knobs: slots.get("maxKnobs").and_then(Value::as_u64)? as usize,
        mouse: v.get("mouseNames").and_then(Value::as_array).is_some_and(|m| !m.is_empty()),
        cheatsheet: v.get("cheatsheet").is_some_and(Value::is_object),
        sheet_defaults: sheet_defaults(v.get("cheatsheet").filter(|s| s.is_object())),
        icons: v.get("cheatsheet").and_then(|s| s.get("icons")).is_some_and(Value::is_object),
        input_modes: v
            .get("device")
            .and_then(|d| d.get("inputModes"))
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, d)| Some((k.clone(), d.as_str()?.to_string()))).collect())
            .unwrap_or_default(),
        options: v
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|o| o.get("key").and_then(Value::as_str).map(String::from))
            .collect(),
        shift_supported: slots.get("shiftSupported").and_then(Value::as_bool),
        held_layers: v.get("heldLayers").is_some_and(Value::is_object),
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

    fn fake_sysfs(name: &str) -> PathBuf {
        let dir = temp_dir(name);
        for (dev, attrs) in [
            ("1-1", &[("idVendor", "0c45"), ("idProduct", "760a"), ("manufacturer", "SONiX")][..]),
            ("1-1:1.0", &[][..]),
            ("1-5", &[("idVendor", "1189"), ("idProduct", "8890"), ("manufacturer", "OpenMacroPad"),
                      ("product", "Control Surface 15+3"), ("serial", "key153"), ("bcdDevice", "0201")][..]),
            ("1-6", &[("idVendor", "1189"), ("idProduct", "8890"), ("manufacturer", "wch.cn"),
                      ("product", "CH552"), ("serial", "key153"), ("bcdDevice", "0100")][..]),
            ("3-1", &[("idVendor", "4348"), ("idProduct", "55e0")][..]),
        ] {
            let d = dir.join(dev);
            std::fs::create_dir_all(&d).unwrap();
            for (k, v) in attrs {
                std::fs::write(d.join(k), format!("{v}\n")).unwrap();
            }
        }
        dir
    }

    #[test]
    fn sysfs_detection_matches_the_daemon_and_ignores_other_devices() {
        let dir = fake_sysfs("sysfs");
        let (pads, loaders) = scan_sysfs(&dir);
        assert_eq!(loaders, 1);
        assert_eq!(pads.len(), 2, "the 0c45 keyboard is not a keypad");
        let open = &pads[0];
        assert_eq!((open.firmware.as_str(), open.version.as_str(), open.board.as_str()), ("control-surface", "2.1", "sy181-15k3e"));
        assert_eq!(open.described, Some((15, 3)));
        let stock = &pads[1];
        assert_eq!((stock.firmware.as_str(), stock.board.as_str(), stock.described), ("stock", "", None));
        assert_eq!(described_layout("control-surface", "Control Surface 12+2"), Some((12, 2)));
        assert_eq!(described_layout("openmacropad", "Macropad 12+3"), None);
        assert_eq!(scan_sysfs(&dir.join("missing")), (Vec::new(), 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_a_daemon_started_by_hand_but_not_its_one_shot_commands() {
        let dir = temp_dir("proc");
        // The kernel keeps 15 bytes of the name in comm, as here.
        let process = |pid: &str, comm: &str, argv: &[&str]| {
            let d = dir.join(pid);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(d.join("cmdline"), argv.iter().map(|a| format!("{a}\0")).collect::<String>()).unwrap();
        };
        process("10", "bash", &["bash"]);
        process("11", "control-surface", &["control-surfaced", "monitor", "--json"]);
        process("14", "control-surface", &["/usr/bin/control-surfaced-old", "run"]);
        assert!(!daemon_process_running(&dir));
        process("12", "control-surface", &["/home/u/.local/bin/control-surfaced", "--quiet", "run"]);
        assert!(daemon_process_running(&dir));
        std::fs::remove_dir_all(dir.join("12")).unwrap();
        process("13", "control-surface", &["control-surfaced"]);
        assert!(daemon_process_running(&dir));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn app(installed: bool, active: &str, bus: bool, process: bool) -> AppState {
        AppState { installed, load: "loaded".into(), active: active.into(), condition_failed: false, bus, process }
    }

    #[test]
    fn app_status_is_accurate_and_start_only_with_a_keypad() {
        let (text, start) = app_summary(&app(true, "active", true, false), true);
        assert_eq!((text.as_str(), start), ("Keypad app: running.", false));
        let (text, start) = app_summary(&app(true, "inactive", true, false), true);
        assert!(text.contains("started outside systemd") && !start, "{text}");
        let (text, _) = app_summary(&app(true, "inactive", false, true), true);
        assert!(text.contains("no Settings interface"), "{text}");
        assert_eq!(app_summary(&app(true, "inactive", false, false), true), ("Keypad app: not running.".into(), true));
        let (text, start) = app_summary(&app(true, "inactive", false, false), false);
        assert!(text.contains("starts by itself") && !start, "{text}");
        let (text, start) = app_summary(&app(true, "failed", false, false), true);
        assert!(text.contains("journalctl") && start, "{text}");
        let (text, start) = app_summary(&app(false, "inactive", false, false), true);
        assert!(text.contains("not installed") && !start, "{text}");
        let mut missing = app(true, "inactive", false, false);
        missing.condition_failed = true;
        assert!(app_summary(&missing, true).0.contains("/usr/bin/control-surfaced is missing"));
        assert!(!app_summary(&missing, true).1);
    }

    #[test]
    fn variants_follow_the_daemon_and_keep_the_menu_order() {
        let ids: Vec<String> = builtin_variants().into_iter().map(|v| v.id).collect();
        assert_eq!(ids, ["sy181-15k3e", "generic-3k", "generic-3k1e", "generic-6k1e", "generic-10k",
                         "generic-12k2e", "generic-12k3e", "generic-16k3e"]);
        let parsed = parse_variants(r#"{"layouts":{"builtin":[
            {"id":"generic-3k","name":"3 keys","keys":3,"knobs":0},
            {"id":"sy181-15k3e","name":"x","keys":15,"knobs":3},
            {"id":"generic-8k1e","name":"8 keys, 1 knob","keys":8,"knobs":1}]}}"#).unwrap();
        let summary: Vec<(String, usize)> = parsed.iter().map(|v| (v.id.clone(), v.columns)).collect();
        assert_eq!(summary, [("sy181-15k3e".to_string(), 5), ("generic-3k".to_string(), 3), ("generic-8k1e".to_string(), 4)]);
        assert!(parse_variants("{}").is_none());
        let layout = parse_daemon_layout(r#"{"ok":true,"layout":{"id":"sy181-15k3e","name":"n","source":"firmware",
            "keys":[{"control":"key1","row":0,"column":0},{"control":"key5","row":0,"column":4}],
            "knobs":[{"control":"knob1","row":0,"column":5}]}}"#).unwrap();
        assert_eq!((layout.source.as_str(), layout.keys, layout.knobs, layout.columns), ("firmware", 2, 1, 5));
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
        let defaults = config::SheetDefaults::default();
        assert_eq!(
            f,
            Features {
                max_keys: 16,
                max_knobs: 3,
                mouse: true,
                cheatsheet: false,
                sheet_defaults: defaults,
                icons: false,
                input_modes: Vec::new(),
                options: Vec::new(),
                shift_supported: None,
                held_layers: false,
            }
        );
        let with_sheet = parse_features(r#"{"slots":{"maxKeys":16,"maxKnobs":3},"cheatsheet":{"modes":["toggle","hold"]}}"#).unwrap();
        assert!(with_sheet.cheatsheet && !with_sheet.icons);
        let icons = r#"{"slots":{"maxKeys":16,"maxKnobs":3},"cheatsheet":{"icons":{"set":"tabler-outline","auto":["volume"]}}}"#;
        assert!(parse_features(icons).unwrap().icons);
        assert!(parse_features(r#"{"slots":{}}"#).is_none());
        let mut actions = Vec::new();
        parse_actions(
            &serde_json::json!({"actions":[{"id":"mark_in","text":"Set &In Point"},{"id":"mark_in","text":"dup"}]}),
            &mut actions,
        );
        assert_eq!(actions, [("mark_in".to_string(), "Set In Point".to_string())]);
    }

    #[test]
    fn cheatsheet_defaults_follow_the_daemon() {
        let sheet = |options: &str| {
            let json = format!(r#"{{"slots":{{"maxKeys":16,"maxKnobs":3}},"cheatsheet":{{"options":{options}}}}}"#);
            parse_features(&json).unwrap().sheet_defaults
        };
        let d = |opacity, auto_hide_ms| config::SheetDefaults { opacity, auto_hide_ms };
        // 738e334: unset autoHideMs = 8000.
        assert_eq!(sheet(r#"{"opacity":"0.05..1, default 0.85","autoHideMs":"0..600000; unset = 8000, 0 = until hidden"}"#), d(0.85, 8000));
        // 2dc06f5..ebeabe8: unset meant until hidden.
        assert_eq!(sheet(r#"{"opacity":"0.05..1, default 0.85","autoHideMs":"0..600000, 0 = until hidden; restarted by pad input"}"#), d(0.85, 0));
        // Requested: structured defaults win over the descriptions.
        let json = r#"{"slots":{"maxKeys":16,"maxKnobs":3},"cheatsheet":{"defaults":{"opacity":0.35,"autoHideMs":8000},"options":{"opacity":"default 0.85"}}}"#;
        assert_eq!(parse_features(json).unwrap().sheet_defaults, d(0.35, 8000));
    }

    #[test]
    fn input_status_and_its_health_line() {
        let mut status = Status::default();
        apply_daemon_status(&mut status, &serde_json::json!({
            "config": {"warnings": ["layout: the config's 12+2 has 15 slots, the firmware 18", "other"]},
            "input": {"configured": "raw", "mode": "evdev-chords",
                      "raw": {"events": 0, "seqGaps": 0, "lostEvents": 0, "reconciledDowns": 0},
                      "evdev": {"events": 42, "whileRaw": 0}}}));
        let i = status.input.clone().unwrap();
        assert_eq!((i.configured.as_str(), i.mode.as_str(), i.keymap_events), ("raw", "evdev-chords", 42));
        assert_eq!(i.layout_warnings.len(), 1);
        let open = Some(("control-surface", "2.0.2"));
        let (now, health, ok) = input_summary(Some(&i), true, open, "raw");
        assert!(now.contains("Keymap") && health.contains("42 events") && health.contains("doesn't match"), "{health}");
        assert!(!ok, "asked for raw, got the keymap");

        let old = InputStatus { layout_warnings: Vec::new(), ..i.clone() };
        let (_, health, _) = input_summary(Some(&old), true, Some(("control-surface", "2.0.1")), "raw");
        assert!(health.contains("2.0.2 or newer") && health.contains("2.0.1"), "{health}");
        let (_, health, _) = input_summary(Some(&old), true, Some(("stock", "1.0")), "raw");
        assert!(health.contains("open control-surface firmware"), "{health}");
        let auto = InputStatus { configured: "auto".into(), ..old.clone() };
        let (_, health, ok) = input_summary(Some(&auto), true, open, "raw");
        assert!(health.contains("once you save") && ok, "{health}");

        let raw = InputStatus { mode: "raw".into(), raw_events: 900, ..old.clone() };
        let (now, health, ok) = input_summary(Some(&raw), true, open, "raw");
        assert!(now.contains("Raw") && health.contains("900 events, none lost") && ok, "{health}");
        let restored = InputStatus { seq_gaps: 1, lost_events: 1, restored: 1, ..raw.clone() };
        let (_, health, ok) = input_summary(Some(&restored), true, open, "raw");
        assert!(health.contains("1 lost, 1 restored") && ok, "restored losses are fine: {health}");
        let unrestored = InputStatus { lost_events: 2, restored: 1, ..restored.clone() };
        assert!(!input_summary(Some(&unrestored), true, open, "raw").2);
        let rough = InputStatus { seq_gaps: 2, lost_events: 3, restored: 3, raw_drops: 1, heartbeat_misses: 4, ..raw.clone() };
        let (_, health, ok) = input_summary(Some(&rough), true, open, "raw");
        assert!(health.contains("3 lost, 3 restored") && health.contains("left raw mode 1 times") && health.contains("4 late"), "{health}");
        assert!(!ok);

        assert!(input_summary(None, false, open, "auto").0.contains("isn't running"));
        assert!(input_summary(None, true, open, "auto").0.contains("too old"));
    }

    #[test]
    fn check_config_json_gives_the_pads_limits_and_where_warnings_are() {
        let json = r#"{"ok":true,"layout":{"id":"sy181-15k3e","oneAtATime":["key2","key3","knob1"]},
            "warnings":["x"],"warningDetails":[
              {"profile":"global","layer":"hold-key5","slot":"key2","message":"profile global layer hold-key5: key2: never fires"},
              {"profile":"global","layer":"","slot":"knob1.shift.cw","message":"shift"},
              {"profile":"g","message":""}]}"#;
        let lint = parse_lint(json).unwrap();
        assert_eq!(lint.one_at_a_time, ["key2", "key3", "knob1"]);
        assert_eq!(lint.warnings.len(), 2, "empty messages are dropped");
        assert_eq!((lint.warnings[0].layer.as_str(), lint.warnings[0].slot.as_str()), ("hold-key5", "key2"));
        assert_eq!(parse_lint("not json"), None);
        let layout = parse_daemon_layout(r#"{"ok":true,"layout":{"id":"sy181-15k3e","keys":[{"column":4}],"knobs":[],"oneAtATime":["key2"]}}"#).unwrap();
        assert_eq!(layout.one_at_a_time, ["key2"]);
    }

    #[test]
    fn held_previews_add_held_to_the_context() {
        assert_eq!(with_held("", "key1"), r#"{"$held":"key1"}"#);
        let v: Value = serde_json::from_str(&with_held(r#"{"focus":"timeline"}"#, "key1+knob3")).unwrap();
        assert_eq!(v, serde_json::json!({"focus": "timeline", "$held": "key1+knob3"}));
    }

    #[test]
    fn set_option_answers() {
        assert_eq!(
            parse_set_option(r#"{"ok":true,"key":"input","old":"raw","new":"evdev","changed":true,"backup":"/c/config.jsonc.bak"}"#),
            OptionResult::Set { changed: true, backup: Some("/c/config.jsonc.bak".into()) }
        );
        assert_eq!(parse_set_option(r#"{"ok":true,"changed":false}"#), OptionResult::Set { changed: false, backup: None });
        assert_eq!(parse_set_option(r#"{"ok":false,"error":{"code":"unknown-option","message":"x"}}"#), OptionResult::Unsupported);
        assert_eq!(
            parse_set_option(r#"{"ok":false,"error":{"code":"invalid-value","message":"input: auto, evdev or raw"}}"#),
            OptionResult::Refused("input: auto, evdev or raw".into())
        );
        assert_eq!(
            parse_set_option(r#"{"ok":false,"changed":true,"error":{"code":"apply","message":"no uinput"}}"#),
            OptionResult::Unapplied("no uinput".into())
        );
        assert!(matches!(parse_set_option("garbage"), OptionResult::Refused(_)));
    }

    #[test]
    fn features_list_settable_options_and_shift_support() {
        let json = r#"{"slots":{"maxKeys":16,"maxKnobs":3,"shiftSupported":false},
                       "options":[{"key":"input","type":"enum"},{"key":"settings.keyRateHz","type":"integer"}]}"#;
        let f = parse_features(json).unwrap();
        assert_eq!(f.options, ["input", "settings.keyRateHz"]);
        assert_eq!(f.shift_supported, Some(false));
        assert!(!f.held_layers);
        let held = parse_features(r#"{"slots":{"maxKeys":16,"maxKnobs":3},"heldLayers":{"when":"\"held\": \"key1\""}}"#).unwrap();
        assert!(held.held_layers);
    }

    #[test]
    fn features_describe_the_input_modes() {
        let json = r#"{"slots":{"maxKeys":16,"maxKnobs":3},"device":{"inputModes":{"evdev":"the pad's keymap","raw":"the firmware's events","auto":"evdev for now"}}}"#;
        let modes = parse_features(json).unwrap().input_modes;
        assert!(modes.contains(&("raw".to_string(), "the firmware's events".to_string())));
        assert_eq!(modes.len(), 3);
    }

    #[test]
    fn daemon_status_supplies_the_full_version_and_the_push_state() {
        let dir = fake_sysfs("status");
        let (pads, _) = scan_sysfs(&dir);
        let mut status = Status { pads, ..Status::default() };
        apply_daemon_status(
            &mut status,
            &serde_json::json!({"device":{"firmware":{"version":"2.0.1"}},"cheatsheet":{"eww":{"enabled":true}}}),
        );
        assert_eq!(status.pads[0].version, "2.0.1");
        assert_eq!(status.pads[1].version, "", "stock firmware has no version");
        assert_eq!(status.sheet_push, Some(true));
        apply_daemon_status(&mut status, &serde_json::json!({}));
        assert_eq!(status.sheet_push, None, "older daemons don't say");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_the_daemons_cheatsheet() {
        let sheet = parse_sheet(
            r#"{"ok":true,"visible":false,"title":"Kdenlive · Wheels","layers":["color-wheels"],"notice":"",
               "keys":[{"control":"key1","row":0,"column":0,"bound":true,"active":true,"label":"Set Zone In","state":"","icon":"brackets-contain-start"},
                       {"control":"key2","row":0,"column":1,"bound":false,"active":false,"label":"","state":""}],
               "knobs":[{"control":"knob1","row":0,"column":5,
                         "ccw":{"bound":true,"active":false,"label":"Lift","state":"r"},
                         "press":{"bound":true,"active":true,"label":"Lift axis","state":"r"},
                         "cw":{"bound":true,"active":false,"label":"Lift","state":"r"}}]}"#,
        )
        .unwrap();
        assert_eq!((sheet.title.as_str(), sheet.layers.as_slice()), ("Kdenlive · Wheels", &["color-wheels".to_string()][..]));
        let zone_in = SheetEntry { bound: true, active: true, label: "Set Zone In".into(), state: String::new(), icon: "brackets-contain-start".into() };
        assert_eq!(sheet.keys[0].entries[0], zone_in);
        assert_eq!(sheet.knobs[0].entries[0].icon, "", "no icon from an older keypad app");
        assert!(!sheet.keys[1].entries[0].bound);
        let knob = &sheet.knobs[0];
        assert_eq!((knob.column, knob.entries.len(), knob.entries[0].state.as_str()), (5, 3, "r"));
        assert!(parse_sheet(r#"{"ok":false}"#).is_none());
    }

    #[test]
    fn sheet_preview_uses_the_unsaved_config_through_the_cli() {
        let dir = temp_dir("sheet");
        let daemon = fake_daemon(
            &dir,
            "case \"$*\" in *\"--window firefox\"*) ;; *) exit 9;; esac\n\
             grep -q UNSAVED \"$6\" || exit 8\n\
             echo '{\"ok\":true,\"title\":\"t\",\"keys\":[{\"control\":\"key1\",\"row\":0,\"column\":0,\"bound\":true,\"active\":true,\"label\":\"L\"}]}'\n",
        );
        let scratch = dir.join("cfg");
        let sheet = sheet_preview(Some(&daemon), &scratch, "{\"x\": \"UNSAVED\"}", "firefox", "").unwrap();
        assert_eq!(sheet.keys[0].entries[0].label, "L");
        assert_eq!(std::fs::read_dir(&scratch).unwrap().count(), 0, "the copy is removed");
        let _ = std::fs::remove_dir_all(&dir);
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
