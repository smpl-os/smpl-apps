//! Taskbar settings backend — workspace count & position.
//!
//! Persists to `~/.config/smplos/bar.conf` (KEY=VALUE format).
//! Applies live via `eww update` and `bar-ctl apply`.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

/// Default workspace dot count shown in the bar.
const DEFAULT_WS_COUNT: i32 = 4;
/// Default spacing between workspace dots (pixels).
const DEFAULT_WS_SPACING: i32 = 1;

fn config_path() -> PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push(".config/smplos/bar.conf");
    p
}

fn eww_config_dir() -> String {
    let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push(".config/eww");
    p.to_string_lossy().into_owned()
}

/// Read bar.conf into a map.
fn read_conf() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(data) = fs::read_to_string(config_path()) {
        for line in data.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    map
}

/// Write the full conf back.
fn write_conf(map: &HashMap<String, String>) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = fs::File::create(&path) {
        // Sort for deterministic output
        let mut pairs: Vec<_> = map.iter().collect();
        pairs.sort_by_key(|(k, _)| k.to_owned());
        for (k, v) in pairs {
            let _ = writeln!(f, "{}={}", k, v);
        }
    }
}

/// Send a live update to a running EWW instance.
fn eww_update(var: &str, val: &str) {
    let cfg = eww_config_dir();
    let _ = Command::new("eww")
        .args(["--config", &cfg, "update", &format!("{}={}", var, val)])
        .output();
}

/// Run clock-top.sh and clock-bot.sh immediately and push their output to
/// the running eww instance — so the bar updates the instant a setting
/// changes rather than waiting for the next poll tick.
fn eww_update_clock() {
    let scripts = format!("{}/scripts", eww_config_dir());
    if let Ok(out) = Command::new("sh").arg(format!("{}/clock-top.sh", scripts)).output() {
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !val.is_empty() { eww_update("clock-top", &val); }
    }
    if let Ok(out) = Command::new("sh").arg(format!("{}/clock-bot.sh", scripts)).output() {
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        eww_update("clock-bot", &val);
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Read current workspace count from config (default 4).
pub fn ws_count() -> i32 {
    let map = read_conf();
    map.get("ws_count")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(DEFAULT_WS_COUNT)
        .clamp(1, 10)
}

/// Read current workspace position: 0 = center, 1 = left.
pub fn ws_position_index() -> i32 {
    let map = read_conf();
    match map.get("ws_position").map(|s| s.as_str()) {
        Some("left") => 1,
        _ => 0,
    }
}

/// Set workspace count (1–10), persist, and apply live.
/// Moves any windows on workspaces beyond the new limit to group 1,
/// then switches the user to group 1 if they were on a removed group.
pub fn set_ws_count(count: i32) {
    let old_count = ws_count();
    let count = count.clamp(1, 10);
    let mut map = read_conf();
    map.insert("ws_count".into(), count.to_string());
    write_conf(&map);
    eww_update("ws-count", &count.to_string());

    // If reducing, migrate orphaned windows & switch away from removed groups
    if count < old_count {
        migrate_windows_above(count);
    }
}

/// Set workspace position (0=center, 1=left), persist, and apply live.
pub fn set_ws_position(index: i32) {
    let val = if index == 1 { "left" } else { "center" };
    let mut map = read_conf();
    map.insert("ws_position".into(), val.to_string());
    write_conf(&map);
    eww_update("ws-position", val);
}

/// Read current workspace spacing from config (default 1).
pub fn ws_spacing() -> i32 {
    let map = read_conf();
    map.get("ws_spacing")
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(DEFAULT_WS_SPACING)
        .clamp(1, 10)
}

/// Set workspace dot spacing (1-10 px), persist, and apply live.
pub fn set_ws_spacing(px: i32) {
    let px = px.clamp(1, 10);
    let mut map = read_conf();
    map.insert("ws_spacing".into(), px.to_string());
    write_conf(&map);
    eww_update("ws-spacing", &px.to_string());
}

/// Read workspace style: 0 = numbers (dots), 1 = squares.
pub fn ws_style_index() -> i32 {
    let map = read_conf();
    match map.get("ws_style").map(|s| s.as_str()) {
        Some("squares") => 1,
        _ => 0,
    }
}

/// Set workspace style (0=numbers, 1=squares), persist, and apply live.
pub fn set_ws_style(index: i32) {
    let val = if index == 1 { "squares" } else { "numbers" };
    let mut map = read_conf();
    map.insert("ws_style".into(), val.to_string());
    write_conf(&map);
    eww_update("ws-style", val);
}

// ── Clock settings ───────────────────────────────────────────────────────────

/// Detect whether the system locale prefers 24-hour time.
/// en_US → false (12 h AM/PM), everything else → true (24 h).
pub fn detect_clock_24h() -> bool {
    let lc = std::env::var("LC_TIME")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_else(|_| "en_US.UTF-8".into());
    !lc.starts_with("en_US")
}

/// Detect the preferred date format index from locale.
/// en_US → 0 (M/D), others → 1 (D/M).
pub fn detect_clock_date_fmt() -> i32 {
    if detect_clock_24h() { 1 } else { 0 }
}

/// Read clock display format: 0=time only, 1=time+dow, 2=time+date.
pub fn clock_format() -> i32 {
    let map = read_conf();
    match map.get("clock_format").map(|s| s.as_str()) {
        Some("dow")  => 1,
        Some("date") => 2,
        _            => 0,
    }
}

/// Set clock display format (0=time, 1=dow, 2=date), persist & apply live.
pub fn set_clock_format(index: i32) {
    let val = match index {
        1 => "dow",
        2 => "date",
        _ => "time",
    };
    let mut map = read_conf();
    map.insert("clock_format".into(), val.to_string());
    write_conf(&map);
    eww_update_clock();
}

/// Read whether 24-hour time is active.
/// If not set in conf, auto-detect from locale and persist.
pub fn clock_24h() -> bool {
    let mut map = read_conf();
    if let Some(v) = map.get("clock_24h") {
        return v == "true";
    }
    // First run: detect and persist
    let detected = detect_clock_24h();
    map.insert("clock_24h".into(), detected.to_string());
    write_conf(&map);
    detected
}

/// Set 24-hour toggle, persist & apply live.
pub fn set_clock_24h(on: bool) {
    let mut map = read_conf();
    map.insert("clock_24h".into(), on.to_string());
    write_conf(&map);
    eww_update_clock();
}

/// Read date format index: 0=M/D, 1=D/M, 2=ISO, 3=Mon D.
/// If not set, auto-detect from locale.
pub fn clock_date_fmt() -> i32 {
    let mut map = read_conf();
    if let Some(v) = map.get("clock_date_fmt") {
        return match v.as_str() {
            "D/M"   => 1,
            "ISO"   => 2,
            "Mon D" => 3,
            _       => 0,
        };
    }
    // First run: detect and persist
    let detected = detect_clock_date_fmt();
    let val = match detected {
        1 => "D/M",
        2 => "ISO",
        3 => "Mon D",
        _ => "M/D",
    };
    map.insert("clock_date_fmt".into(), val.to_string());
    write_conf(&map);
    detected
}

/// Set date format (0=M/D, 1=D/M, 2=ISO, 3=Mon D), persist.
pub fn set_clock_date_fmt(index: i32) {
    let val = match index {
        1 => "D/M",
        2 => "ISO",
        3 => "Mon D",
        _ => "M/D",
    };
    let mut map = read_conf();
    map.insert("clock_date_fmt".into(), val.to_string());
    write_conf(&map);
    eww_update_clock();
}

// ── Workspace window-migration ────────────────────────────────────────────────

/// Move all windows from workspace groups > `max_group` to group 1.
///
/// Uses the grouped-workspace model:
///   monitor 0 (primary/rightmost) → workspace N
///   monitor 1 (secondary)        → workspace N+10
///   monitor 2                    → workspace N+20  etc.
///
/// A window on workspace 17 is in group ((17-1)%10)+1 = 7 on monitor 1.
/// If max_group=6, move it to workspace 11 (group 1 on monitor 1).
///
/// After migrating, if the current group exceeds the limit, switch to group 1.
fn migrate_windows_above(max_group: i32) {
    // Get all clients
    let output = match Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return,
    };
    let clients_json = String::from_utf8_lossy(&output.stdout);
    let clients: serde_json::Value = match serde_json::from_str(&clients_json) {
        Ok(v) => v,
        Err(_) => return,
    };

    if let Some(arr) = clients.as_array() {
        for client in arr {
            let ws_id = match client["workspace"]["id"].as_i64() {
                Some(id) if id > 0 => id as i32,
                _ => continue, // skip special workspaces
            };
            // Determine which monitor slot this workspace belongs to
            let monitor_offset = (ws_id - 1) / 10; // 0, 1, 2, ...
            let group = ((ws_id - 1) % 10) + 1;    // 1–10

            if group > max_group {
                let target_ws = 1 + monitor_offset * 10; // group 1 on same monitor
                let addr = match client["address"].as_str() {
                    Some(a) => a.to_string(),
                    None => continue,
                };
                let _ = Command::new("hyprctl")
                    .args([
                        "dispatch",
                        "movetoworkspacesilent",
                        &format!("{},address:{}", target_ws, addr),
                    ])
                    .output();
            }
        }
    }

    // If the user is currently on a group beyond the limit, switch to group 1
    if let Ok(o) = Command::new("hyprctl")
        .args(["activeworkspace", "-j"])
        .output()
    {
        let json = String::from_utf8_lossy(&o.stdout);
        if let Ok(ws) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(id) = ws["id"].as_i64() {
                let group = ((id as i32 - 1) % 10) + 1;
                if group > max_group {
                    let _ = Command::new("workspace-group").arg("1").output();
                }
            }
        }
    }
}
