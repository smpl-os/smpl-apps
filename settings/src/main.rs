mod dictation;
mod display;
mod keybindings;
mod layouts;
mod startup;
mod taskbar;
mod theme;
mod wifi;
mod xkb_labels;

use display::backend::DisplayBackend;
use display::monitor::{canvas_scale_factor, Monitor, MonitorConfig};
use slint::Model;
use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

macro_rules! debug_log {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            eprintln!($($arg)*);
        }
    };
}
pub(crate) use debug_log;

// ── Highlight blink animation ────────────────────────────────────────────────

/// Blink the highlight border 5 times with smooth fade-in/fade-out.
/// Total duration: 5 blinks × 1s = 5 seconds, then clears.
fn start_highlight_blink(ui: &MainWindow) {
    let ui_weak = ui.as_weak();
    let blink_count = Rc::new(RefCell::new(0u32));
    let timer = slint::Timer::default();
    // Start with blink on
    ui.set_highlight_blink_on(true);
    let blink_count2 = blink_count.clone();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(500),
        move || {
            let Some(ui) = ui_weak.upgrade() else { return; };
            let mut count = blink_count2.borrow_mut();
            *count += 1;
            if *count >= 10 {
                // 5 full on/off cycles done — clear
                ui.set_highlight_blink_on(false);
                ui.set_highlight_setting(slint::SharedString::default());
                return;
            }
            // Toggle blink
            ui.set_highlight_blink_on((*count).is_multiple_of(2));
        },
    );
    std::mem::forget(timer);
}

// ── Single-instance guard ────────────────────────────────────────────────────

fn acquire_single_instance(tab: i32, highlight: &str) {
    use std::os::unix::io::AsRawFd;
    let run_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    let lock_path = format!("{}/settings.lock", run_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap_or_else(|e| {
            eprintln!("[settings] cannot open lock file: {}", e);
            std::process::exit(1);
        });
    let fd = file.as_raw_fd();
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        eprintln!("[settings] another instance is already running — bringing it to focus");
        // Write deep-link file so the running instance can navigate
        if !highlight.is_empty() {
            let deeplink_path = format!("{}/settings-deeplink", run_dir);
            let _ = std::fs::write(&deeplink_path, format!("{}\n{}", tab, highlight));
        }
        // Bring the existing window to the foreground
        let _ = std::process::Command::new("hyprctl")
            .args(["dispatch", "focuswindow", "class:settings"])
            .status();
        std::process::exit(0);
    }
    std::mem::forget(file);
}

// ── Settings search index export ─────────────────────────────────────────────

fn settings_search_index() -> Vec<(&'static str, &'static str, i32)> {
    vec![
        // About
        ("About smplOS", "about", 0),
        ("Version", "about", 0),
        ("Hostname", "about", 0),
        ("Kernel", "about", 0),
        ("Uptime", "about", 0),
        ("Compositor", "about", 0),
        // Keyboard
        ("Keyboard Layout", "keyboard", 1),
        ("Add Layout", "keyboard", 1),
        ("Remove Layout", "keyboard", 1),
        ("XKB Layout", "keyboard", 1),
        ("Input Language", "keyboard", 1),
        ("Keyboard Preview", "keyboard", 1),
        // Dictation
        ("Dictation", "dictation", 2),
        ("Speech to Text", "dictation", 2),
        ("Whisper", "dictation", 2),
        ("Voice Input", "dictation", 2),
        ("Language Model", "dictation", 2),
        ("Microphone", "dictation", 2),
        // Display
        ("Display", "display", 3),
        ("Monitor", "display", 3),
        ("Resolution", "display", 3),
        ("Scale", "display", 3),
        ("Primary Monitor", "display", 3),
        ("Screen Layout", "display", 3),
        ("Refresh Rate", "display", 3),
        // Power
        ("Power Profile", "power", 4),
        ("Power Saver", "power", 4),
        ("Balanced", "power", 4),
        ("Performance", "power", 4),
        ("Lock Screen Timeout", "power", 4),
        ("Screen Off Timeout", "power", 4),
        ("Suspend Timeout", "power", 4),
        ("Shutdown After", "power", 4),
        ("Sleep", "power", 4),
        // Keybindings
        ("Keybindings", "keybindings", 5),
        ("Keyboard Shortcuts", "keybindings", 5),
        ("Hotkeys", "keybindings", 5),
        ("Shortcut", "keybindings", 5),
        // Taskbar
        ("Workspace Count", "taskbar", 6),
        ("Workspace Position", "taskbar", 6),
        ("Workspace Spacing", "taskbar", 6),
        ("Workspace Style", "taskbar", 6),
        ("Clock Format", "taskbar", 6),
        ("Time Format", "taskbar", 6),
        ("Date Format", "taskbar", 6),
        ("24-hour", "taskbar", 6),
        ("AM PM", "taskbar", 6),
        // Wi-Fi
        ("Wi-Fi", "wifi", 7),
        ("WiFi", "wifi", 7),
        ("Wireless", "wifi", 7),
        ("Connect Wi-Fi", "wifi", 7),
        ("Wi-Fi Password", "wifi", 7),
        ("Network", "wifi", 7),
        ("QR Code Wi-Fi", "wifi", 7),
        ("Share Wi-Fi", "wifi", 7),
        ("Scan QR", "wifi", 7),
        // Startup
        ("Startup Apps", "startup", 8),
        ("Autostart", "startup", 8),
        ("Startup", "startup", 8),
        ("Boot Apps", "startup", 8),
        ("Login Apps", "startup", 8),
        ("Daemons", "startup", 8),
        ("Services", "startup", 8),
        ("Background Services", "startup", 8),
    ]
}

fn export_settings_index() {
    let home = std::env::var("HOME").unwrap_or_default();
    let cache_dir = format!("{}/.cache/smplos", home);
    let _ = std::fs::create_dir_all(&cache_dir);
    let path = format!("{}/settings_index", cache_dir);

    let lines: Vec<String> = settings_search_index()
        .iter()
        .map(|(label, tab, _idx)| {
            format!(
                "{};settings --tab {} --highlight \"{}\";settings;",
                label, tab, label
            )
        })
        .collect();

    let _ = std::fs::write(&path, lines.join("\n") + "\n");
}

// ── Theme application ────────────────────────────────────────────────────────

fn apply_theme(ui: &MainWindow) {
    let palette = theme::load_theme_from_eww_scss(&format!(
        "{}/.config/eww/theme-colors.scss",
        std::env::var("HOME").unwrap_or_default()
    ));

    let theme = Theme::get(ui);
    theme.set_bg(palette.bg.darker(0.05));
    theme.set_fg(palette.fg);
    theme.set_fg_dim(palette.fg_dim);
    theme.set_accent(palette.accent);
    theme.set_bg_light(palette.bg_light);
    theme.set_bg_lighter(palette.bg_lighter);
    theme.set_danger(palette.danger);
    theme.set_success(palette.success);
    theme.set_warning(palette.warning);
    theme.set_info(palette.info);
    theme.set_opacity(palette.opacity);
}

// ── Fuzzy search ─────────────────────────────────────────────────────────────

/// Simple fuzzy match: every character in the query must appear in order in the
/// target string (case-insensitive). e.g. "ppr" matches "Power Profile".
fn fuzzy_match(target: &str, query: &str) -> bool {
    let lower = target.to_lowercase();
    let mut target_chars = lower.chars();
    for qc in query.chars() {
        loop {
            match target_chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

// ── Keyboard helpers ─────────────────────────────────────────────────────────

fn to_key_model(keys: &[xkb_labels::KeyInfo]) -> slint::ModelRc<KeyData> {
    let entries: Vec<KeyData> = keys
        .iter()
        .map(|k| KeyData {
            base: k.base.clone().into(),
            english: k.english.clone().into(),
            w: k.width,
            is_modifier: k.is_modifier,
        })
        .collect();
    slint::ModelRc::from(Rc::new(slint::VecModel::from(entries)))
}

fn set_keyboard_preview(ui: &MainWindow, layout: &str, variant: &str) {
    let (name, r0, r1, r2, r3, r4) = xkb_labels::resolve(layout, variant);
    ui.set_layout_name(name.into());
    ui.set_row0(to_key_model(&r0));
    ui.set_row1(to_key_model(&r1));
    ui.set_row2(to_key_model(&r2));
    ui.set_row3(to_key_model(&r3));
    ui.set_row4(to_key_model(&r4));
}

fn push_active_to_ui(ui: &MainWindow, active: &[layouts::ActiveLayout]) {
    let entries: Vec<LayoutEntry> = active
        .iter()
        .map(|a| LayoutEntry {
            code: a.code.clone().into(),
            variant: a.variant.clone().into(),
            description: a.description.clone().into(),
        })
        .collect();
    ui.set_active_layouts(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))));
    layouts::sync_to_compositor(active);
}

// ── Display helpers ──────────────────────────────────────────────────────────

const CANVAS_W: f64 = 580.0;
const CANVAS_H: f64 = 200.0;

struct DisplayState {
    backend: Box<dyn DisplayBackend>,
    monitors: Vec<Monitor>,
    original: Vec<Monitor>,
    offset_x: f64,
    offset_y: f64,
    scale: f64,
    primary: String,
}

impl DisplayState {
    fn new(backend: Box<dyn DisplayBackend>) -> Self {
        Self {
            backend,
            monitors: Vec::new(),
            original: Vec::new(),
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 0.1,
            primary: String::new(),
        }
    }

    fn load_monitors(&mut self) -> Result<(), String> {
        self.monitors = self.backend.query_monitors()?;
        self.original = self.monitors.clone();
        self.primary = self
            .monitors
            .iter()
            .find(|m| m.focused)
            .or(self.monitors.first())
            .map(|m| m.name.clone())
            .unwrap_or_default();
        self.recalc_canvas();
        Ok(())
    }

    fn recalc_canvas(&mut self) {
        self.scale = canvas_scale_factor(&self.monitors, CANVAS_W, CANVAS_H);
        // Positions are logical pixels; use f64 min to match canvas_scale_factor.
        let min_x = self.monitors.iter().map(|m| m.x as f64).fold(f64::MAX, f64::min);
        let min_y = self.monitors.iter().map(|m| m.y as f64).fold(f64::MAX, f64::min);
        self.offset_x = if min_x == f64::MAX { 0.0 } else { -min_x };
        self.offset_y = if min_y == f64::MAX { 0.0 } else { -min_y };
    }

    fn to_slint_model(&self) -> Vec<MonitorInfo> {
        let margin = 20.0;
        self.monitors
            .iter()
            .map(|m| {
                let modes: Vec<slint::SharedString> = m
                    .available_modes
                    .iter()
                    .map(|mode| slint::SharedString::from(mode.label()))
                    .collect();

                let cur_mode_idx = m
                    .available_modes
                    .iter()
                    .position(|mode| {
                        mode.width == m.width
                            && mode.height == m.height
                            && (mode.refresh_rate - m.refresh_rate).abs() < 1.0
                    })
                    .unwrap_or(0) as i32;

                MonitorInfo {
                    id: m.id,
                    name: slint::SharedString::from(&m.name),
                    description: slint::SharedString::from(&m.description),
                    width: m.width,
                    height: m.height,
                    refresh_rate: m.refresh_rate as f32,
                    pos_x: m.x,
                    pos_y: m.y,
                    scale: m.scale as f32,
                    enabled: m.enabled,
                    is_primary: m.name == self.primary,
                    canvas_x: ((m.x as f64 + self.offset_x) * self.scale + margin) as f32,
                    canvas_y: ((m.y as f64 + self.offset_y) * self.scale + margin) as f32,
                    // Positions are logical; width/height are physical — use logical size.
                    canvas_w: (m.width as f64 / m.scale * self.scale) as f32,
                    canvas_h: (m.height as f64 / m.scale * self.scale) as f32,
                    available_modes: slint::ModelRc::new(slint::VecModel::from(modes)),
                    current_mode_index: cur_mode_idx,
                }
            })
            .collect()
    }

    fn configs_from_current(&self) -> Vec<MonitorConfig> {
        self.monitors
            .iter()
            .map(|m| MonitorConfig {
                name: m.name.clone(),
                width: m.width,
                height: m.height,
                refresh_rate: m.refresh_rate,
                x: m.x,
                y: m.y,
                scale: m.scale,
                enabled: m.enabled,
            })
            .collect()
    }

    fn has_changes(&self) -> bool {
        if self.monitors.len() != self.original.len() {
            return true;
        }
        for (m, o) in self.monitors.iter().zip(self.original.iter()) {
            if m.x != o.x
                || m.y != o.y
                || m.width != o.width
                || m.height != o.height
                || (m.refresh_rate - o.refresh_rate).abs() > 0.1
                || (m.scale - o.scale).abs() > 0.01
                || m.enabled != o.enabled
            {
                return true;
            }
        }
        false
    }
}

fn push_display_state_to_ui(ui: &MainWindow, state: &DisplayState) {
    let model = state.to_slint_model();
    let model_rc = slint::ModelRc::new(slint::VecModel::from(model));
    ui.set_disp_monitors(model_rc);
    ui.set_disp_has_changes(state.has_changes());

    let idx = ui.get_disp_selected_index();
    if idx >= 0 && (idx as usize) < state.monitors.len() {
        let m = &state.monitors[idx as usize];
        let modes: Vec<slint::SharedString> = m
            .available_modes
            .iter()
            .map(|mode| slint::SharedString::from(mode.label()))
            .collect();
        ui.set_disp_selected_modes(slint::ModelRc::new(slint::VecModel::from(modes)));

        let mode_idx = m
            .available_modes
            .iter()
            .position(|mode| {
                mode.width == m.width
                    && mode.height == m.height
                    && (mode.refresh_rate - m.refresh_rate).abs() < 1.0
            })
            .unwrap_or(0);
        ui.set_disp_selected_mode_index(mode_idx as i32);
        ui.set_disp_selected_scale(m.scale as f32);
    }
}

// ── Window guard helpers ─────────────────────────────────────────────────────

fn display_conf_path() -> std::path::PathBuf {
    let mut p = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    p.push(".config/smplos/display.conf");
    p
}

fn read_display_conf() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Ok(data) = std::fs::read_to_string(display_conf_path()) {
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

fn write_display_conf(map: &std::collections::HashMap<String, String>) {
    let path = display_conf_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = std::fs::File::create(&path) {
        use std::io::Write;
        let mut pairs: Vec<_> = map.iter().collect();
        pairs.sort_by_key(|(k, _)| k.to_owned());
        for (k, v) in pairs {
            let _ = writeln!(f, "{}={}", k, v);
        }
    }
}

/// Read window-guard enabled setting (default: true).
fn window_guard_enabled() -> bool {
    let map = read_display_conf();
    map.get("window_guard")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(true) // on by default
}

/// Persist and apply window-guard toggle.
fn set_window_guard(enabled: bool) {
    let mut map = read_display_conf();
    map.insert("window_guard".to_string(), if enabled { "true" } else { "false" }.to_string());
    write_display_conf(&map);

    if enabled {
        // Start window-guard if not already running
        let already_running = std::process::Command::new("pgrep")
            .args(["-f", "window-guard"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !already_running {
            let _ = std::process::Command::new("bash")
                .arg("-c")
                .arg("window-guard &")
                .spawn();
        }
    } else {
        // Stop window-guard
        let _ = std::process::Command::new("pkill")
            .args(["-f", "window-guard"])
            .output();
    }
}

// ── Power + About helpers ────────────────────────────────────────────────────

fn is_power_profiles_available() -> bool {
    std::process::Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn get_power_profile() -> String {
    std::process::Command::new("powerprofilesctl")
        .arg("get")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "balanced".to_string())
}

fn set_power_profile(profile: &str) {
    let _ = std::process::Command::new("powerprofilesctl")
        .args(["set", profile])
        .output();
}

// ── Hypridle (idle timeouts) ─────────────────────────────────────────────────

// Preset arrays: index → seconds (0 = disabled/never)
// New unified presets: 1min, 5min, 10min, 30min, Never, 1h, 2h, 3h, 5h, 8h
const IDLE_PRESETS: &[u32] = &[60, 300, 600, 1800, 0, 3600, 7200, 10800, 18000, 28800];

fn hypridle_config_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    std::path::PathBuf::from(home).join(".config/hypr/hypridle.conf")
}

/// Parsed idle timeouts from hypridle.conf: (lock_secs, dpms_secs, suspend_secs)
fn read_hypridle_timeouts() -> (u32, u32, u32) {
    let path = hypridle_config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return (300, 330, 600), // defaults
    };

    let mut lock = 0u32;
    let mut dpms = 0u32;
    let mut suspend = 0u32;

    // Simple parser: find listener blocks and identify by on-timeout command
    let mut in_listener = false;
    let mut cur_timeout = 0u32;
    let mut cur_cmd = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("listener") && trimmed.contains('{') {
            in_listener = true;
            cur_timeout = 0;
            cur_cmd.clear();
        } else if in_listener && trimmed == "}" {
            // Classify this listener by its command
            if cur_cmd.contains("lock-session") || cur_cmd.contains("hyprlock") {
                lock = cur_timeout;
            } else if cur_cmd.contains("dpms off") || cur_cmd.contains("dpms 0") {
                dpms = cur_timeout;
            } else if cur_cmd.contains("suspend") || cur_cmd.contains("hibernate") {
                suspend = cur_timeout;
            }
            in_listener = false;
        } else if in_listener {
            if let Some(val) = trimmed.strip_prefix("timeout") {
                let val = val.trim().trim_start_matches('=').trim();
                cur_timeout = val.parse().unwrap_or(0);
            } else if let Some(val) = trimmed.strip_prefix("on-timeout") {
                cur_cmd = val.trim().trim_start_matches('=').trim().to_string();
            }
        }
    }

    (lock, dpms, suspend)
}

/// Find closest preset index for a given timeout value
fn timeout_to_index(secs: u32, presets: &[u32]) -> i32 {
    if secs == 0 {
        // "Never" is index 4
        return 4;
    }
    presets
        .iter()
        .enumerate()
        .filter(|(_, &v)| v > 0)
        .min_by_key(|(_, &v)| (v as i64 - secs as i64).unsigned_abs())
        .map(|(i, _)| i as i32)
        .unwrap_or(2) // default to middle
}

/// Write a new hypridle.conf with updated timeouts and restart hypridle
fn write_hypridle_config(lock_secs: u32, dpms_secs: u32, suspend_secs: u32) {
    let lock_cmd = if lock_secs > 0 {
        format!(
            "# {:.0} min -- lock screen\nlistener {{\n    timeout = {}\n    on-timeout = loginctl lock-session\n}}\n",
            lock_secs as f64 / 60.0, lock_secs
        )
    } else {
        String::new()
    };

    let dpms_cmd = if dpms_secs > 0 {
        format!(
            "# {:.0} min -- screen off\nlistener {{\n    timeout = {}\n    on-timeout = systemd-detect-virt -q || hyprctl dispatch dpms off\n    on-resume = hyprctl dispatch dpms on\n}}\n",
            dpms_secs as f64 / 60.0, dpms_secs
        )
    } else {
        String::new()
    };

    let suspend_cmd = if suspend_secs > 0 {
        format!(
            "# {:.0} min -- suspend\nlistener {{\n    timeout = {}\n    on-timeout = systemd-detect-virt -q || systemctl suspend\n}}\n",
            suspend_secs as f64 / 60.0, suspend_secs
        )
    } else {
        String::new()
    };

    let config = format!(
        "# smplOS Hypridle Configuration\n\
         # Managed by Settings app -- manual edits will be overwritten\n\
         \n\
         general {{\n    \
             lock_cmd = pidof hyprlock || hyprlock\n    \
             before_sleep_cmd = loginctl lock-session\n    \
             after_sleep_cmd = hyprctl dispatch dpms on\n\
         }}\n\n\
         {lock_cmd}\n\
         {dpms_cmd}\n\
         {suspend_cmd}"
    );

    let path = hypridle_config_path();
    if let Err(e) = std::fs::write(&path, config) {
        eprintln!("[settings] failed to write hypridle.conf: {}", e);
        return;
    }
    debug_log!("[settings] wrote hypridle.conf: lock={}s dpms={}s suspend={}s",
        lock_secs, dpms_secs, suspend_secs);

    // Restart hypridle to pick up changes
    let _ = std::process::Command::new("pkill").arg("hypridle").output();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let _ = std::process::Command::new("hypridle")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Schedule a system shutdown after `secs` seconds (0 = cancel any pending shutdown).
fn schedule_shutdown(secs: u32) {
    // Cancel any existing scheduled shutdown first (non-blocking, null stdin to avoid sudo hang)
    let _ = std::process::Command::new("shutdown")
        .args(["-c"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    if secs == 0 {
        debug_log!("[settings] shutdown timer cancelled");
        return;
    }

    let mins = secs.div_ceil(60); // round up to minutes
    let _ = std::process::Command::new("shutdown")
        .args(["-h", &format!("+{}", mins)])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    debug_log!("[settings] shutdown scheduled in {} minutes", mins);
}

fn get_about_info() -> (String, String, String, String, String, String, String) {
    let version = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|c| {
            c.lines()
                .find(|l| l.starts_with("VERSION_ID="))
                .map(|l| l.trim_start_matches("VERSION_ID=").trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "dev".to_string());

    let kernel = std::process::Command::new("uname")
        .arg("-r")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    let uptime = std::process::Command::new("uptime")
        .arg("-p")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .strip_prefix("up ")
                .unwrap_or("unknown")
                .to_string()
        })
        .unwrap_or_default();

    let hostname = std::fs::read_to_string("/etc/hostname")
        .unwrap_or_else(|_| "smplOS".to_string())
        .trim()
        .to_string();

    // CPU: parse /proc/cpuinfo for model name + core count
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| {
            let model = c.lines()
                .find(|l| l.starts_with("model name"))
                .and_then(|l| l.split(':').nth(1))
                .map(|s| s.trim().to_string())?;
            let cores = c.lines()
                .filter(|l| l.starts_with("processor"))
                .count();
            Some(format!("{} ({} cores)", model, cores))
        })
        .unwrap_or_default();

    // RAM: parse /proc/meminfo for MemTotal
    let ram = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|c| {
            let kb: u64 = c.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())?;
            let gb = kb as f64 / 1_048_576.0;
            Some(format!("{:.1} GB", gb))
        })
        .unwrap_or_default();

    // GPU: try nvidia-smi first (includes VRAM), then lspci + sysfs VRAM
    let gpu = (|| -> Option<String> {
        // NVIDIA: nvidia-smi gives name + VRAM in one shot
        if let Ok(o) = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
            .output()
        {
            let out = String::from_utf8_lossy(&o.stdout);
            let line = out.trim();
            if !line.is_empty() && o.status.success() {
                // Format: "NVIDIA GeForce RTX 4090, 24576"
                let parts: Vec<&str> = line.splitn(2, ", ").collect();
                if parts.len() == 2 {
                    if let Ok(mib) = parts[1].trim().parse::<u64>() {
                        let gb = mib as f64 / 1024.0;
                        return Some(format!("{} ({:.0} GB)", parts[0].trim(), gb));
                    }
                }
                return Some(line.to_string());
            }
        }

        // Fallback: lspci for GPU name
        let lspci_out = std::process::Command::new("lspci").output().ok()?;
        let lspci = String::from_utf8_lossy(&lspci_out.stdout);
        let gpu_name = lspci.lines()
            .find(|l| l.contains("VGA") || l.contains("3D controller"))
            .and_then(|l| l.split(':').next_back())
            .map(|s| s.trim().to_string())?;

        // AMD: try sysfs for VRAM
        if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
            for entry in entries.flatten() {
                let vram_path = entry.path().join("device/mem_info_vram_total");
                if let Ok(val) = std::fs::read_to_string(&vram_path) {
                    if let Ok(bytes) = val.trim().parse::<u64>() {
                        let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                        return Some(format!("{} ({:.0} GB)", gpu_name, gb));
                    }
                }
            }
        }

        Some(gpu_name)
    })().unwrap_or_default();

    (version, kernel, uptime, hostname, cpu, ram, gpu)
}

// ── Keybindings helpers ──────────────────────────────────────────────────────

struct KeybindingsState {
    file: Option<keybindings::BindingsFile>,
    /// Snapshot of serialized content to detect changes.
    original_serial: String,
}

impl KeybindingsState {
    fn new() -> Self {
        Self { file: None, original_serial: String::new() }
    }

    fn load(&mut self) -> Result<(), String> {
        let f = keybindings::BindingsFile::load()?;
        self.original_serial = f.serialize();
        self.file = Some(f);
        Ok(())
    }

    fn has_changes(&self) -> bool {
        self.file.as_ref()
            .map(|f| f.serialize() != self.original_serial)
            .unwrap_or(false)
    }
}

fn push_keybindings_to_ui(
    ui: &MainWindow,
    state: &KeybindingsState,
    filter: &str,
    section_idx: i32,
) {
    let file = match &state.file {
        Some(f) => f,
        None => return,
    };

    // Section filter
    let sections = keybindings::unique_sections(&file.bindings);
    let section_name = if section_idx > 0 {
        sections.get(section_idx as usize).cloned().unwrap_or_default()
    } else {
        String::new() // "All"
    };

    let filter_lower = filter.to_lowercase();

    let entries: Vec<BindingEntry> = file.bindings.iter().enumerate()
        .filter(|(_, kb)| {
            if !section_name.is_empty() && kb.section != section_name {
                return false;
            }
            if !filter_lower.is_empty() {
                let haystack = format!(
                    "{} {} {} {}",
                    kb.combo_display(),
                    kb.description,
                    kb.dispatcher,
                    kb.args
                ).to_lowercase();
                if !haystack.contains(&filter_lower) {
                    return false;
                }
            }
            true
        })
        .map(|(i, kb)| BindingEntry {
            idx: i as i32,
            combo: kb.combo_display().into(),
            description: kb.description.clone().into(),
            dispatcher: kb.dispatcher.clone().into(),
            args: kb.args.clone().into(),
            section: kb.section.clone().into(),
            submap: kb.submap.clone().into(),
            is_exec: kb.dispatcher == "exec",
        })
        .collect();

    ui.set_kb_bindings(slint::ModelRc::new(slint::VecModel::from(entries)));

    let section_strings: Vec<slint::SharedString> = sections.iter()
        .map(|s| slint::SharedString::from(s.as_str()))
        .collect();
    ui.set_kb_sections(slint::ModelRc::new(slint::VecModel::from(section_strings)));

    ui.set_kb_has_changes(state.has_changes());
    ui.set_kb_status_text(slint::SharedString::from(format!(
        "{} -- {} bindings",
        file.path_display(),
        file.bindings.len()
    )));
}

/// Map Slint key event text to Hyprland key names — delegates to smpl-common.
fn slint_key_to_hyprland(text: &str) -> String {
    let result = keybindings::slint_key_to_hyprland(text);
    if result.is_empty() {
        debug_log!(
            "[settings] unknown key capture: {:?} (bytes: {:?})",
            text,
            text.as_bytes()
        );
    }
    result
}

// ── Startup helpers ──────────────────────────────────────────────────────────

fn refresh_startup_items(ui: &MainWindow) {
    let items = startup::list_items();
    let entries: Vec<StartupEntry> = items
        .iter()
        .map(|i| StartupEntry {
            name: i.name.clone().into(),
            command: i.command.clone().into(),
            source: i.source.clone().into(),
            enabled: i.enabled,
            toggleable: i.toggleable,
            description: i.description.clone().into(),
        })
        .collect();
    ui.set_startup_items(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))));
    ui.set_startup_status_text(slint::SharedString::from(format!(
        "{} items",
        items.len()
    )));
}

fn refresh_available_apps(ui: &MainWindow, filter: &str) {
    let all_apps = startup::list_available_apps();
    let filtered: Vec<startup::AvailableApp> = if filter.is_empty() {
        all_apps
    } else {
        let q = filter.to_lowercase();
        all_apps
            .into_iter()
            .filter(|a| {
                a.name.to_lowercase().contains(&q) || a.description.to_lowercase().contains(&q)
            })
            .collect()
    };
    let entries: Vec<AvailableAppEntry> = filtered
        .iter()
        .map(|a| AvailableAppEntry {
            name: a.name.clone().into(),
            exec: a.exec.clone().into(),
            desktop_file: a.desktop_file.clone().into(),
            description: a.description.clone().into(),
        })
        .collect();
    ui.set_startup_available_apps(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))));
}

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<(), slint::PlatformError> {
    // Parse CLI args early (before single-instance guard needs tab/highlight)
    let mut initial_tab = 0;
    let mut initial_layout = "us".to_string();
    let mut initial_variant = String::new();
    let mut initial_highlight = String::new();
    let mut use_demo = false;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--version" => {
                println!("settings v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--export-index" => {
                export_settings_index();
                return Ok(());
            }
            "--tab" => {
                if i + 1 < args.len() {
                    initial_tab = match args[i + 1].as_str() {
                        "about" => 0,
                        "keyboard" => 1,
                        "dictation" => 2,
                        "display" => 3,
                        "power" => 4,
                        "keybindings" => 5,
                        "taskbar" => 6,
                        "wifi" => 7,
                        "startup" => 8,
                        _ => 0,
                    };
                    i += 1;
                }
            }
            "--highlight" => {
                if i + 1 < args.len() {
                    initial_highlight = args[i + 1].clone();
                    i += 1;
                }
            }
            "--demo" => {
                use_demo = true;
            }
            other => {
                // Positional: layout [variant]
                if initial_layout == "us" && !other.starts_with('-') {
                    initial_layout = other.to_string();
                    if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                        initial_variant = args[i + 1].clone();
                        i += 1;
                    }
                }
            }
        }
        i += 1;
    }

    acquire_single_instance(initial_tab, &initial_highlight);
    dictation::cleanup_stale_progress();

    // Export settings search index for start menu
    export_settings_index();

    smpl_common::init("settings", 900.0, 560.0)?;

    let ui = MainWindow::new()?;
    apply_theme(&ui);
    ui.set_active_tab(initial_tab);
    if !initial_highlight.is_empty() {
        ui.set_highlight_setting(slint::SharedString::from(&initial_highlight));
        start_highlight_blink(&ui);
    }

    // ── Keyboard tab init ────────────────────────────────────────────────────

    // Dictation data for UI
    {
        let lang_names: Vec<slint::SharedString> = dictation::LANGUAGES
            .iter()
            .map(|l| slint::SharedString::from(l.name))
            .collect();
        ui.set_dictation_lang_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(lang_names))));

        let model_entries: Vec<ModelEntry> = dictation::MODELS
            .iter()
            .map(|m| ModelEntry {
                label: m.label.into(),
                size: m.size.into(),
                note: m.note.into(),
                english_only: m.english_only,
            })
            .collect();
        ui.set_dictation_model_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(model_entries))));

        ui.set_dictation_selected_lang_name("English (recommended)".into());
        ui.set_dictation_selected_model_idx(0);
        ui.set_dictation_also_english(false);
        ui.set_dictation_show_also_english(false);

        let installed = dictation::is_installed();
        ui.set_dictation_installed(installed);
        if installed {
            if let Some(cfg) = dictation::read_config() {
                ui.set_dictation_language(dictation::language_display(&cfg));
                ui.set_dictation_model(dictation::model_display(&cfg.model));
                if let Some(idx) = dictation::find_language_idx(&cfg.primary_code) {
                    ui.set_dictation_selected_lang_name(
                        dictation::LANGUAGES[idx].name.into(),
                    );
                }
                if let Some(idx) = dictation::find_model_idx(&cfg.model) {
                    ui.set_dictation_selected_model_idx(idx as i32);
                }
                ui.set_dictation_also_english(cfg.also_english);
                let is_en = cfg.primary_code == "en" || cfg.primary_code == "auto";
                ui.set_dictation_show_also_english(!is_en);
                ui.set_dictation_config_missing(false);
            } else {
                ui.set_dictation_config_missing(true);
                ui.set_dictation_language("(no config)".into());
                ui.set_dictation_model("(no config)".into());
            }
            ui.set_dictation_service_running(dictation::is_service_running());
        }
    }

    layouts::cleanup_legacy_config();
    set_keyboard_preview(&ui, &initial_layout, &initial_variant);

    let available: Rc<RefCell<Vec<layouts::AvailableLayout>>> =
        Rc::new(RefCell::new(Vec::new()));
    let filtered_indices: Rc<RefCell<Vec<usize>>> = Rc::new(RefCell::new(Vec::new()));
    let active_layouts: Rc<RefCell<Vec<layouts::ActiveLayout>>> = Rc::new(RefCell::new(vec![
        layouts::ActiveLayout {
            code: initial_layout.to_string(),
            variant: initial_variant.to_string(),
            description: String::new(),
        },
    ]));

    // Deferred layout loading
    {
        let ui_weak = ui.as_weak();
        let avail = available.clone();
        let fi = filtered_indices.clone();
        let al = active_layouts.clone();
        let init_layout = initial_layout.to_string();
        let init_variant = initial_variant.to_string();
        slint::Timer::single_shot(std::time::Duration::from_millis(0), move || {
            debug_log!("[settings] deferred load starting...");
            let loaded = layouts::list_available_layouts();
            debug_log!("[settings] loaded {} available layouts", loaded.len());
            let display_strings: Vec<slint::SharedString> = loaded
                .iter()
                .map(|a| slint::SharedString::from(a.display()))
                .collect();

            let all_indices: Vec<usize> = (0..loaded.len()).collect();
            *fi.borrow_mut() = all_indices;

            {
                let mut active = al.borrow_mut();
                if let Some(saved) = layouts::load_from_os_config(&loaded) {
                    debug_log!("[settings] source: input.conf ({} layouts)", saved.len());
                    *active = saved;
                } else if let Some(from_compositor) = layouts::load_from_compositor(&loaded) {
                    debug_log!("[settings] source: compositor ({} layouts)", from_compositor.len());
                    *active = from_compositor;
                } else {
                    debug_log!("[settings] source: default (us)");
                    if let Some(first) = active.first_mut() {
                        first.description =
                            layouts::describe(&loaded, &init_layout, &init_variant);
                    }
                }
            }

            *avail.borrow_mut() = loaded;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_available_layouts(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                    display_strings,
                ))));
                let active = al.borrow();
                push_active_to_ui(&ui, &active);

                if init_layout != "us" || !init_variant.is_empty() {
                    set_keyboard_preview(&ui, &init_layout, &init_variant);
                } else if active.len() > 1 {
                    set_keyboard_preview(&ui, &active[1].code, &active[1].variant);
                } else if let Some(first) = active.first() {
                    set_keyboard_preview(&ui, &first.code, &first.variant);
                }

                ui.set_loading(false);
            }
        });
    }

    // Show placeholder active layouts
    {
        let active = active_layouts.borrow();
        let entries: Vec<LayoutEntry> = active
            .iter()
            .map(|a| LayoutEntry {
                code: a.code.clone().into(),
                variant: a.variant.clone().into(),
                description: a.description.clone().into(),
            })
            .collect();
        ui.set_active_layouts(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))));
    }

    // ── Display tab init ─────────────────────────────────────────────────────

    let display_backend: Box<dyn DisplayBackend> = if use_demo {
        eprintln!("Running display tab in demo mode with mock monitors");
        Box::new(display::demo::DemoBackend::new())
    } else {
        match display::backend::detect_backend() {
            Ok(b) => b,
            Err(e) => {
                debug_log!("[settings] display backend: {e} -- using demo");
                Box::new(display::demo::DemoBackend::new())
            }
        }
    };

    let disp_state = Rc::new(RefCell::new(DisplayState::new(display_backend)));
    if let Err(e) = disp_state.borrow_mut().load_monitors() {
        debug_log!("[settings] failed to query monitors: {e}");
    }
    push_display_state_to_ui(&ui, &disp_state.borrow());
    // Auto-select first monitor so controls are visible immediately
    if !disp_state.borrow().monitors.is_empty() {
        ui.set_disp_selected_index(0);
        push_display_state_to_ui(&ui, &disp_state.borrow());
    }
    ui.set_disp_status_text(slint::SharedString::from(format!(
        "Backend: {} | {} monitor(s)",
        disp_state.borrow().backend.name(),
        disp_state.borrow().monitors.len()
    )));

    // Load window-guard toggle state
    ui.set_disp_window_guard(window_guard_enabled());

    // ── Power tab init ───────────────────────────────────────────────────────

    {
        // Power profiles (may not be available)
        let ppd_available = is_power_profiles_available();
        ui.set_power_profiles_available(ppd_available);
        if ppd_available {
            let profile = get_power_profile();
            let idx: i32 = match profile.as_str() {
                "power-saver" => 0,
                "balanced" => 1,
                "performance" => 2,
                _ => 1,
            };
            ui.set_power_profile_index(idx);
        }

        // Idle timeouts from hypridle.conf
        let (lock_s, dpms_s, suspend_s) = read_hypridle_timeouts();
        ui.set_idle_lock_index(timeout_to_index(lock_s, IDLE_PRESETS));
        ui.set_idle_dpms_index(timeout_to_index(dpms_s, IDLE_PRESETS));
        ui.set_idle_suspend_index(timeout_to_index(suspend_s, IDLE_PRESETS));
        ui.set_idle_shutdown_index(4); // Default: Never
    }

    // ── About tab init ───────────────────────────────────────────────────────

    ui.set_app_version(slint::SharedString::from(format!("v{}", env!("CARGO_PKG_VERSION"))));
    {
        let (version, kernel, uptime, hostname, cpu, ram, gpu) = get_about_info();
        ui.set_about_version(slint::SharedString::from(format!("v{}", version)));
        ui.set_about_kernel(slint::SharedString::from(kernel));
        ui.set_about_uptime(slint::SharedString::from(uptime));
        ui.set_about_hostname(slint::SharedString::from(hostname));
        ui.set_about_cpu(slint::SharedString::from(cpu));
        ui.set_about_ram(slint::SharedString::from(ram));
        ui.set_about_gpu(slint::SharedString::from(gpu));
    }

    // ── Keybindings tab init (deferred if opened via --tab) ──────────────────

    if initial_tab == 5 {
        let ui_weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(50), move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.invoke_kb_load();
            }
        });
    }

    // ── Taskbar tab init ──────────────────────────────────────────────────────

    ui.set_tb_ws_count(taskbar::ws_count());
    ui.set_tb_ws_position_index(taskbar::ws_position_index());
    ui.set_tb_ws_spacing(taskbar::ws_spacing());
    ui.set_tb_ws_style_index(taskbar::ws_style_index());
    ui.set_tb_clock_format_index(taskbar::clock_format());
    ui.set_tb_clock_24h(taskbar::clock_24h());
    ui.set_tb_clock_date_fmt_index(taskbar::clock_date_fmt());

    // ── Wi-Fi tab init ────────────────────────────────────────────────────────
    {
        let networks = wifi::list_networks(false);
        let entries: Vec<WifiEntry> = networks
            .iter()
            .map(|n| WifiEntry {
                ssid: n.ssid.clone().into(),
                signal: n.signal,
                security: n.security.clone().into(),
                connected: n.connected,
                saved: n.saved,
            })
            .collect();
        ui.set_wifi_networks(slint::ModelRc::from(Rc::new(slint::VecModel::from(entries))));
        let current = wifi::get_current_ssid().unwrap_or_default();
        ui.set_wifi_current_ssid(current.into());
    }

    // ── Startup tab init ──────────────────────────────────────────────────────

    refresh_startup_items(&ui);

    // ══════════════════════════════════════════════════════════════════════════
    // CALLBACKS
    // ══════════════════════════════════════════════════════════════════════════

    ui.on_close(|| std::process::exit(0));

    // Window drag
    {
        let ui_weak = ui.as_weak();
        ui.on_move_window(move |dx, dy| {
            if let Some(ui) = ui_weak.upgrade() {
                let scale = ui.window().scale_factor();
                let pos = ui.window().position();
                ui.window().set_position(slint::WindowPosition::Physical(
                    slint::PhysicalPosition::new(
                        pos.x + (dx * scale) as i32,
                        pos.y + (dy * scale) as i32,
                    ),
                ));
            }
        });
    }

    // ── Keyboard callbacks ───────────────────────────────────────────────────

    // Add layout
    {
        let ui_weak = ui.as_weak();
        let avail = available.clone();
        let fi = filtered_indices.clone();
        let al = active_layouts.clone();
        ui.on_add_layout(move |idx| {
            let original_idx = {
                let fi = fi.borrow();
                match fi.get(idx as usize) {
                    Some(&i) => i,
                    None => return,
                }
            };

            let avail = avail.borrow();
            let entry = match avail.get(original_idx) {
                Some(e) => e,
                None => return,
            };

            {
                let current = al.borrow();
                if current.len() >= 2 {
                    return;
                }
                if current.iter().any(|a| a.code == entry.code && a.variant == entry.variant) {
                    return;
                }
            }

            al.borrow_mut().push(layouts::ActiveLayout {
                code: entry.code.clone(),
                variant: entry.variant.clone(),
                description: entry.description.clone(),
            });

            if let Some(ui) = ui_weak.upgrade() {
                let active = al.borrow();
                push_active_to_ui(&ui, &active);
                set_keyboard_preview(&ui, &entry.code, &entry.variant);
            }
        });
    }

    // Remove layout
    {
        let ui_weak = ui.as_weak();
        let al = active_layouts.clone();
        ui.on_remove_layout(move |idx| {
            let idx = idx as usize;
            {
                let current = al.borrow();
                if idx >= current.len() || current.len() <= 1 {
                    return;
                }
                if current[idx].code == "us" {
                    return;
                }
            }

            al.borrow_mut().remove(idx);

            if let Some(ui) = ui_weak.upgrade() {
                let active = al.borrow();
                push_active_to_ui(&ui, &active);
                if let Some(first) = active.first() {
                    set_keyboard_preview(&ui, &first.code, &first.variant);
                }
            }
        });
    }

    // Preview layout
    {
        let ui_weak = ui.as_weak();
        ui.on_preview_layout(move |idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let model = ui.get_active_layouts();
                if let Some(entry) = model.row_data(idx as usize) {
                    set_keyboard_preview(&ui, entry.code.as_str(), entry.variant.as_str());
                }
            }
        });
    }

    // Preview from dropdown
    {
        let ui_weak = ui.as_weak();
        let avail = available.clone();
        let fi = filtered_indices.clone();
        ui.on_preview_dropdown(move |idx| {
            let original_idx = {
                let fi = fi.borrow();
                match fi.get(idx as usize) {
                    Some(&i) => i,
                    None => return,
                }
            };
            let avail = avail.borrow();
            let entry = match avail.get(original_idx) {
                Some(e) => e,
                None => return,
            };
            if let Some(ui) = ui_weak.upgrade() {
                set_keyboard_preview(&ui, &entry.code, &entry.variant);
            }
        });
    }

    // Filter layouts
    {
        let ui_weak = ui.as_weak();
        let avail = available.clone();
        let fi = filtered_indices.clone();
        ui.on_filter_layouts(move |query| {
            let query_lower = query.to_lowercase();
            let mut new_indices = Vec::new();
            let mut filtered_strings = Vec::new();

            let avail = avail.borrow();
            for (i, a) in avail.iter().enumerate() {
                if query.is_empty() || a.display().to_lowercase().contains(&query_lower) {
                    new_indices.push(i);
                    filtered_strings.push(slint::SharedString::from(a.display()));
                }
            }

            *fi.borrow_mut() = new_indices;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_available_layouts(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                    filtered_strings,
                ))));
                ui.set_selected_dropdown_index(-1);
            }
        });
    }

    // ── Dictation callbacks ──────────────────────────────────────────────────

    let dictation_filtered_indices: Rc<RefCell<Vec<usize>>> =
        Rc::new(RefCell::new((0..dictation::LANGUAGES.len()).collect()));

    // Filter dictation languages
    {
        let ui_weak = ui.as_weak();
        let dfi = dictation_filtered_indices.clone();
        ui.on_filter_dictation_langs(move |query| {
            let query_lower = query.to_lowercase();
            let mut new_indices = Vec::new();
            let mut filtered_names = Vec::new();

            for (i, lang) in dictation::LANGUAGES.iter().enumerate() {
                if query.is_empty() || lang.name.to_lowercase().contains(&query_lower) {
                    new_indices.push(i);
                    filtered_names.push(slint::SharedString::from(lang.name));
                }
            }

            *dfi.borrow_mut() = new_indices;

            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dictation_lang_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(
                    filtered_names,
                ))));
            }
        });
    }

    // Select dictation language
    {
        let ui_weak = ui.as_weak();
        let dfi = dictation_filtered_indices.clone();
        ui.on_select_dictation_lang(move |idx| {
            let original_idx = {
                let fi = dfi.borrow();
                match fi.get(idx as usize) {
                    Some(&i) => i,
                    None => return,
                }
            };

            let lang = &dictation::LANGUAGES[original_idx];
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dictation_selected_lang_name(lang.name.into());

                let is_en_or_auto = lang.code == "en" || lang.code == "auto";
                ui.set_dictation_show_also_english(!is_en_or_auto);
                if is_en_or_auto {
                    ui.set_dictation_also_english(false);
                }

                if lang.code == "en" {
                    ui.set_dictation_selected_model_idx(0);
                } else {
                    let current = ui.get_dictation_selected_model_idx() as usize;
                    if dictation::is_model_english_only(current) {
                        ui.set_dictation_selected_model_idx(1);
                    }
                }

                let all_names: Vec<slint::SharedString> = dictation::LANGUAGES
                    .iter()
                    .map(|l| slint::SharedString::from(l.name))
                    .collect();
                ui.set_dictation_lang_list(slint::ModelRc::from(Rc::new(slint::VecModel::from(all_names))));
                let mut fi = dfi.borrow_mut();
                *fi = (0..dictation::LANGUAGES.len()).collect();
            }
        });
    }

    // Start dictation install
    {
        let ui_weak = ui.as_weak();
        ui.on_start_dictation_install(move || {
            if let Some(ui) = ui_weak.upgrade() {
                if dictation::is_install_running() {
                    return;
                }

                let lang_name = ui.get_dictation_selected_lang_name();
                let lang_code = dictation::LANGUAGES.iter()
                    .find(|l| l.name == lang_name.as_str())
                    .map(|l| l.code)
                    .unwrap_or("en");

                let mut model_idx = ui.get_dictation_selected_model_idx() as usize;

                if lang_code != "en" && dictation::is_model_english_only(model_idx) {
                    ui.set_dictation_selected_model_idx(1);
                    model_idx = 1;
                }

                let model_id = dictation::MODELS.get(model_idx)
                    .map(|m| m.id)
                    .unwrap_or("base");

                let also_english = ui.get_dictation_also_english();

                if !dictation::write_config(lang_code, model_id, also_english) {
                    ui.set_dictation_progress_text("Error: Could not write config file".into());
                    ui.set_dictation_install_error(true);
                    ui.set_dictation_installing(true);
                    return;
                }

                ui.set_dictation_progress(0.0);
                ui.set_dictation_progress_text("Starting...".into());
                ui.set_dictation_install_error(false);
                ui.set_dictation_installing(true);
                if !dictation::launch_install() {
                    ui.set_dictation_install_error(true);
                }
            }
        });
    }

    // Reconfigure dictation
    {
        let ui_weak = ui.as_weak();
        ui.on_start_dictation_reconfigure(move || {
            if let Some(ui) = ui_weak.upgrade() {
                if dictation::is_install_running() {
                    return;
                }

                let lang_name = ui.get_dictation_selected_lang_name();
                let lang_code = dictation::LANGUAGES.iter()
                    .find(|l| l.name == lang_name.as_str())
                    .map(|l| l.code)
                    .unwrap_or("en");

                let mut model_idx = ui.get_dictation_selected_model_idx() as usize;

                if lang_code != "en" && dictation::is_model_english_only(model_idx) {
                    ui.set_dictation_selected_model_idx(1);
                    model_idx = 1;
                }

                let model_id = dictation::MODELS.get(model_idx)
                    .map(|m| m.id)
                    .unwrap_or("base");

                let also_english = ui.get_dictation_also_english();

                if !dictation::write_config(lang_code, model_id, also_english) {
                    ui.set_dictation_progress_text("Error: Could not write config file".into());
                    ui.set_dictation_install_error(true);
                    ui.set_dictation_installing(true);
                    return;
                }

                ui.set_dictation_progress(0.0);
                ui.set_dictation_progress_text("Starting...".into());
                ui.set_dictation_install_error(false);
                ui.set_dictation_installing(true);
                ui.set_dictation_configuring(false);
                if !dictation::launch_model_download() {
                    ui.set_dictation_install_error(true);
                }
            }
        });
    }

    ui.on_open_dictation_config(|| {
        dictation::open_config();
    });

    {
        let ui_weak = ui.as_weak();
        ui.on_cancel_dictation_install(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_dictation_installing(false);
                ui.set_dictation_install_error(false);
                ui.set_dictation_progress(0.0);
                ui.set_dictation_progress_text("Starting...".into());
                dictation::clear_progress();
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_restart_dictation_service(move || {
            dictation::restart_service();
            let ui_weak2 = ui_weak.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
                if let Some(ui) = ui_weak2.upgrade() {
                    ui.set_dictation_service_running(dictation::is_service_running());
                }
            });
        });
    }

    // ── Display callbacks ────────────────────────────────────────────────────

    // Select monitor
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_select_monitor(move |idx| {
            let ui = ui_handle.unwrap();
            let st = state.borrow();
            if let Some(m) = st.monitors.get(idx as usize) {
                let modes: Vec<slint::SharedString> = m
                    .available_modes
                    .iter()
                    .map(|mode| slint::SharedString::from(mode.label()))
                    .collect();
                ui.set_disp_selected_modes(slint::ModelRc::new(slint::VecModel::from(modes)));

                let mode_idx = m
                    .available_modes
                    .iter()
                    .position(|mode| {
                        mode.width == m.width
                            && mode.height == m.height
                            && (mode.refresh_rate - m.refresh_rate).abs() < 1.0
                    })
                    .unwrap_or(0);
                ui.set_disp_selected_mode_index(mode_idx as i32);
                ui.set_disp_selected_scale(m.scale as f32);
            }
        });
    }

    // Drag finished
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_drag_finished(move |idx, canvas_x, canvas_y| {
            let mut st = state.borrow_mut();
            let idx = idx as usize;
            if idx >= st.monitors.len() { return; }

            let real_x = ((canvas_x as f64 - 20.0) / st.scale - st.offset_x) as i32;
            let real_y = ((canvas_y as f64 - 20.0) / st.scale - st.offset_y) as i32;
            st.monitors[idx].x = real_x;
            st.monitors[idx].y = real_y;

            // X-only glue snap: always force the dragged monitor to touch the
            // nearest adjacent edge of another monitor horizontally.
            // Y is never snapped so the user can freely align monitors vertically.
            let logical_w = (st.monitors[idx].width as f64 / st.monitors[idx].scale) as i32;
            let snapped_x = {
                let mut best_x = real_x;
                let mut best_dist = i32::MAX;
                for (i, m) in st.monitors.iter().enumerate() {
                    if i == idx { continue; }
                    let ow = (m.width as f64 / m.scale) as i32;
                    // Candidate: place dragged immediately right of this monitor
                    let right_of = m.x + ow;
                    // Candidate: place dragged immediately left of this monitor
                    let left_of = m.x - logical_w;
                    for cx in [right_of, left_of] {
                        let dist = (cx - real_x).abs();
                        if dist < best_dist {
                            best_dist = dist;
                            best_x = cx;
                        }
                    }
                }
                best_x
            };
            st.monitors[idx].x = snapped_x;
            // Y stays exactly where the user dropped it — no vertical snapping.

            st.recalc_canvas();
            let ui = ui_handle.unwrap();
            push_display_state_to_ui(&ui, &st);
        });
    }

    // Change resolution
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_change_resolution(move |mon_idx, mode_idx| {
            let mut st = state.borrow_mut();
            let mi = mon_idx as usize;
            let modi = mode_idx as usize;
            if mi < st.monitors.len() && modi < st.monitors[mi].available_modes.len() {
                let mode = st.monitors[mi].available_modes[modi].clone();
                st.monitors[mi].width = mode.width;
                st.monitors[mi].height = mode.height;
                st.monitors[mi].refresh_rate = mode.refresh_rate;
                st.recalc_canvas();
            }
            let ui = ui_handle.unwrap();
            push_display_state_to_ui(&ui, &st);
        });
    }

    // Change scale
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_change_scale(move |mon_idx, scale| {
            let mut st = state.borrow_mut();
            let mi = mon_idx as usize;
            if mi < st.monitors.len() {
                st.monitors[mi].scale = scale as f64;
                st.recalc_canvas();
            }
            let ui = ui_handle.unwrap();
            push_display_state_to_ui(&ui, &st);
        });
    }

    // Set primary
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_set_primary(move |mon_idx| {
            let mut st = state.borrow_mut();
            let mi = mon_idx as usize;
            if mi < st.monitors.len() {
                st.primary = st.monitors[mi].name.clone();
                let name = st.primary.clone();
                let _ = st.backend.set_primary(&name);
            }
            let ui = ui_handle.unwrap();
            push_display_state_to_ui(&ui, &st);
            ui.set_disp_status_text(slint::SharedString::from("Primary monitor updated"));
        });
    }

    // Apply
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_apply_changes(move || {
            let mut st = state.borrow_mut();
            let ui = ui_handle.unwrap();

            // Only apply if there are actual changes.
            if !st.has_changes() {
                ui.set_disp_status_text(slint::SharedString::from("No changes to apply"));
                return;
            }

            let configs = st.configs_from_current();

            match st.backend.apply(&configs) {
                Ok(()) => {
                    match st.backend.persist(&configs) {
                        Ok(path) => {
                            ui.set_disp_status_text(slint::SharedString::from(format!(
                                "Applied and saved to {path}"
                            )));
                        }
                        Err(e) => {
                            ui.set_disp_status_text(slint::SharedString::from(format!(
                                "Applied live but failed to save: {e}"
                            )));
                        }
                    }
                    st.original = st.monitors.clone();
                }
                Err(e) => {
                    ui.set_disp_status_text(slint::SharedString::from(format!("Apply failed: {e}")));
                }
            }
            push_display_state_to_ui(&ui, &st);
        });
    }

    // Revert
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_revert_changes(move || {
            let mut st = state.borrow_mut();
            st.monitors = st.original.clone();
            st.recalc_canvas();
            let ui = ui_handle.unwrap();
            push_display_state_to_ui(&ui, &st);
            ui.set_disp_status_text(slint::SharedString::from("Reverted to original layout"));
            ui.set_disp_selected_index(-1);
        });
    }

    // Refresh
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_refresh_monitors(move || {
            let mut st = state.borrow_mut();
            let ui = ui_handle.unwrap();
            match st.load_monitors() {
                Ok(()) => {
                    push_display_state_to_ui(&ui, &st);
                    ui.set_disp_status_text(slint::SharedString::from(format!(
                        "{} monitor(s) detected", st.monitors.len()
                    )));
                    ui.set_disp_selected_index(-1);
                }
                Err(e) => {
                    ui.set_disp_status_text(slint::SharedString::from(format!("Refresh failed: {e}")));
                }
            }
        });
    }

    // Identify
    {
        let state = disp_state.clone();
        let ui_handle = ui.as_weak();
        ui.on_disp_identify_monitors(move || {
            let st = state.borrow();
            let ui = ui_handle.unwrap();
            match st.backend.identify(&st.monitors) {
                Ok(()) => {
                    ui.set_disp_status_text(slint::SharedString::from("Identifying monitors..."));
                }
                Err(e) => {
                    ui.set_disp_status_text(slint::SharedString::from(format!("Identify failed: {e}")));
                }
            }
        });
    }

    // Window guard toggle
    ui.on_disp_set_window_guard(move |enabled| {
        set_window_guard(enabled);
    });

    // ── Power callbacks ──────────────────────────────────────────────────────

    ui.on_set_power_profile(move |idx| {
        let profile = match idx {
            0 => "power-saver",
            1 => "balanced",
            2 => "performance",
            _ => "balanced",
        };
        set_power_profile(profile);
    });

    // Helper: read current indices from UI, resolve to seconds, write config
    let write_idle = |ui: &MainWindow| {
        let lock_idx = ui.get_idle_lock_index() as usize;
        let dpms_idx = ui.get_idle_dpms_index() as usize;
        let susp_idx = ui.get_idle_suspend_index() as usize;
        let shutdown_idx = ui.get_idle_shutdown_index() as usize;
        let lock_s = IDLE_PRESETS.get(lock_idx).copied().unwrap_or(300);
        let dpms_s = IDLE_PRESETS.get(dpms_idx).copied().unwrap_or(300);
        let susp_s = IDLE_PRESETS.get(susp_idx).copied().unwrap_or(600);
        let shutdown_s = IDLE_PRESETS.get(shutdown_idx).copied().unwrap_or(0);
        write_hypridle_config(lock_s, dpms_s, susp_s);
        // Schedule shutdown timer
        schedule_shutdown(shutdown_s);
    };

    {
        let ui_handle = ui.as_weak();
        ui.on_set_idle_lock(move |_idx| {
            if let Some(ui) = ui_handle.upgrade() {
                write_idle(&ui);
            }
        });
    }
    // Bind write_idle for dpms/suspend too — need separate clones
    let write_idle2 = write_idle;
    {
        let ui_handle = ui.as_weak();
        ui.on_set_idle_dpms(move |_idx| {
            if let Some(ui) = ui_handle.upgrade() {
                write_idle2(&ui);
            }
        });
    }
    let write_idle3 = write_idle;
    {
        let ui_handle = ui.as_weak();
        ui.on_set_idle_suspend(move |_idx| {
            if let Some(ui) = ui_handle.upgrade() {
                write_idle3(&ui);
            }
        });
    }
    let write_idle4 = write_idle;
    {
        let ui_handle = ui.as_weak();
        ui.on_set_idle_shutdown(move |_idx| {
            if let Some(ui) = ui_handle.upgrade() {
                write_idle4(&ui);
            }
        });
    }

    // ── Keybindings callbacks ────────────────────────────────────────────────

    let kb_state = Rc::new(RefCell::new(KeybindingsState::new()));

    // Load
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_load(move || {
            let mut st = kb.borrow_mut();
            if st.file.is_some() {
                // Already loaded — just refresh UI
                if let Some(ui) = ui_weak.upgrade() {
                    let filter = ui.get_kb_filter_text().to_string();
                    let sec = ui.get_kb_selected_section();
                    push_keybindings_to_ui(&ui, &st, &filter, sec);
                }
                return;
            }
            match st.load() {
                Ok(()) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        push_keybindings_to_ui(&ui, &st, "", 0);
                    }
                }
                Err(e) => {
                    debug_log!("[settings] keybindings load error: {e}");
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_kb_status_text(slint::SharedString::from(format!("Error: {e}")));
                    }
                }
            }
        });
    }

    // Filter
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_filter(move |query, section_idx| {
            let st = kb.borrow();
            if let Some(ui) = ui_weak.upgrade() {
                push_keybindings_to_ui(&ui, &st, query.as_str(), section_idx);
            }
        });
    }

    // Edit combo (start capture — enters empty submap so keys pass through)
    {
        let ui_weak = ui.as_weak();
        ui.on_kb_edit_combo(move |_idx| {
            // Enter a submap with no bindings so keys pass to the app
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "kb-capture"])
                .output();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_kb_capture_combo(slint::SharedString::from(""));
            }
        });
    }

    // Save combo (after key capture) — auto-saves to file + reloads Hyprland
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_save_combo(move |idx, mods, key| {
            // Exit capture submap
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "reset"])
                .output();

            // Clean up mods: trim whitespace
            let mods_clean = mods.trim().to_string();

            // Map Slint key text to Hyprland key name
            let key_str = key.as_str();
            let hypr_key = slint_key_to_hyprland(key_str);

            if !hypr_key.is_empty() {
                let mut st = kb.borrow_mut();
                if let Some(file) = &mut st.file {
                    // Check for conflicts before applying
                    let submap = file.bindings.get(idx as usize)
                        .map(|b| b.submap.clone())
                        .unwrap_or_default();
                    if let Some(conflict) = file.find_conflict(
                        &mods_clean, &hypr_key, &submap, Some(idx as usize),
                    ) {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_kb_capturing(false);
                            ui.set_kb_status_text(slint::SharedString::from(format!(
                                "Conflict: {} is already used by \"{}\"",
                                conflict.existing.combo_display(),
                                conflict.existing.description,
                            )));
                        }
                        return;
                    }

                    file.edit_combo(idx as usize, &mods_clean, &hypr_key);

                    // Auto-save to file and reload Hyprland immediately
                    match file.save_and_reload() {
                        Ok(()) => {
                            st.original_serial = file.serialize();
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_kb_capturing(false);
                                ui.set_kb_capture_combo(slint::SharedString::from(""));
                                let filter = ui.get_kb_filter_text().to_string();
                                let sec = ui.get_kb_selected_section();
                                push_keybindings_to_ui(&ui, &st, &filter, sec);
                                ui.set_kb_status_text("Saved and reloaded".into());
                            }
                        }
                        Err(e) => {
                            debug_log!("[settings] keybindings auto-save error: {e}");
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_kb_capturing(false);
                                let filter = ui.get_kb_filter_text().to_string();
                                let sec = ui.get_kb_selected_section();
                                push_keybindings_to_ui(&ui, &st, &filter, sec);
                                ui.set_kb_status_text(slint::SharedString::from(format!("Error saving: {e}")));
                            }
                        }
                    }
                }
            } else {
                // Unknown key — cancel capture
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_kb_capturing(false);
                    ui.set_kb_status_text("Unknown key - try again".into());
                }
            }
        });
    }

    // Cancel capture
    {
        let ui_weak = ui.as_weak();
        ui.on_kb_cancel_capture(move || {
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "reset"])
                .output();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_kb_capturing(false);
            }
        });
    }

    // Edit args (command for exec bindings)
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_edit_args(move |idx, new_args| {
            let mut st = kb.borrow_mut();
            if let Some(file) = &mut st.file {
                file.edit_args(idx as usize, new_args.as_str());
            }
            if let Some(ui) = ui_weak.upgrade() {
                let filter = ui.get_kb_filter_text().to_string();
                let sec = ui.get_kb_selected_section();
                push_keybindings_to_ui(&ui, &st, &filter, sec);
            }
        });
    }

    // Remove binding
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_remove(move |idx| {
            let mut st = kb.borrow_mut();
            if let Some(file) = &mut st.file {
                file.remove(idx as usize);
            }
            if let Some(ui) = ui_weak.upgrade() {
                let filter = ui.get_kb_filter_text().to_string();
                let sec = ui.get_kb_selected_section();
                push_keybindings_to_ui(&ui, &st, &filter, sec);
            }
        });
    }

    // Add binding
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_add(move |desc, cmd| {
            let mut st = kb.borrow_mut();
            if let Some(file) = &mut st.file {
                let kb_entry = keybindings::Keybinding {
                    bind_type: "bindd".to_string(),
                    mods: "SUPER".to_string(),
                    key: String::new(),
                    description: desc.to_string(),
                    dispatcher: "exec".to_string(),
                    args: cmd.to_string(),
                    section: "Application Launchers".to_string(),
                    submap: String::new(),
                };
                file.add(kb_entry);
            }
            if let Some(ui) = ui_weak.upgrade() {
                let filter = ui.get_kb_filter_text().to_string();
                let sec = ui.get_kb_selected_section();
                push_keybindings_to_ui(&ui, &st, &filter, sec);
            }
        });
    }

    // Save
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_save(move || {
            let mut st = kb.borrow_mut();
            let result = st.file.as_ref().map(|f| f.save_and_reload());
            match result {
                Some(Ok(())) => {
                    st.original_serial = st.file.as_ref()
                        .map(|f| f.serialize())
                        .unwrap_or_default();
                    if let Some(ui) = ui_weak.upgrade() {
                        let filter = ui.get_kb_filter_text().to_string();
                        let sec = ui.get_kb_selected_section();
                        push_keybindings_to_ui(&ui, &st, &filter, sec);
                        ui.set_kb_status_text("Saved and reloaded".into());
                    }
                }
                Some(Err(e)) => {
                    debug_log!("[settings] keybindings save error: {e}");
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_kb_status_text(slint::SharedString::from(format!("Error: {e}")));
                    }
                }
                None => {}
            }
        });
    }

    // Revert
    {
        let kb = kb_state.clone();
        let ui_weak = ui.as_weak();
        ui.on_kb_revert(move || {
            let mut st = kb.borrow_mut();
            match st.load() {
                Ok(()) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        push_keybindings_to_ui(&ui, &st, "", 0);
                        ui.set_kb_filter_text("".into());
                        ui.set_kb_selected_section(0);
                        ui.set_kb_edit_idx(-1);
                        ui.set_kb_status_text("Reverted to saved".into());
                    }
                }
                Err(e) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_kb_status_text(slint::SharedString::from(format!("Error: {e}")));
                    }
                }
            }
        });
    }

    // Open in editor
    {
        let kb = kb_state.clone();
        ui.on_kb_open_editor(move || {
            let st = kb.borrow();
            if let Some(file) = &st.file {
                file.open_in_editor();
            }
        });
    }

    // ── Taskbar callbacks ────────────────────────────────────────────────────

    ui.on_tb_set_ws_count(|count| {
        taskbar::set_ws_count(count);
    });

    ui.on_tb_set_ws_position(|idx| {
        taskbar::set_ws_position(idx);
    });

    ui.on_tb_set_ws_spacing(|px| {
        taskbar::set_ws_spacing(px);
    });

    ui.on_tb_set_ws_style(|idx| {
        taskbar::set_ws_style(idx);
    });

    ui.on_tb_set_clock_format(|idx| {
        taskbar::set_clock_format(idx);
    });

    ui.on_tb_set_clock_24h(|on| {
        taskbar::set_clock_24h(on);
    });

    ui.on_tb_set_clock_date_fmt(|idx| {
        taskbar::set_clock_date_fmt(idx);
    });

    // ── Wi-Fi callbacks ──────────────────────────────────────────────────────

    /// Set `wifi-status` and derive `wifi-status-is-error` in one call.
    fn wifi_set_status(ui: &MainWindow, msg: &str) {
        let is_err = msg.starts_with("Failed")
            || msg.starts_with("Cannot")
            || msg.starts_with("Invalid")
            || msg.starts_with("Scan failed")
            || msg.starts_with("QR generation")
            || msg.starts_with("Export failed");
        ui.set_wifi_status(msg.into());
        ui.set_wifi_status_is_error(is_err);
    }

    /// Convert a backend `wifi::WifiNetwork` into the Slint-generated `WifiEntry`.
    fn to_wifi_entry(n: &wifi::WifiNetwork) -> WifiEntry {
        WifiEntry {
            ssid: n.ssid.clone().into(),
            signal: n.signal,
            security: n.security.clone().into(),
            connected: n.connected,
            saved: n.saved,
        }
    }

    // ── Wi-Fi radio init ─────────────────────────────────────────────────
    // Query the current radio state so the airplane toggle reflects reality.
    match wifi::nmcli::get_wifi_radio_state() {
        Ok(Some(enabled)) => {
            // radio exists: airplane mode is the inverse of "enabled"
            ui.set_wifi_airplane_mode(!enabled);
            ui.set_wifi_no_hardware(false);
        }
        Ok(None) => {
            // no wifi hardware or nmcli unavailable
            ui.set_wifi_no_hardware(true);
            ui.set_wifi_airplane_mode(false);
        }
        Err(e) => {
            eprintln!("[settings] wifi radio state check failed: {}", e);
            ui.set_wifi_no_hardware(false);
        }
    }

    // List networks (fast, no active re-probe)
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_list(move || {
            if let Some(ui) = ui_weak.upgrade() {
                let networks = wifi::list_networks(false);
                let entries: Vec<WifiEntry> = networks.iter().map(to_wifi_entry).collect();
                ui.set_wifi_networks(slint::ModelRc::from(
                    Rc::new(slint::VecModel::from(entries)),
                ));
                let current = wifi::get_current_ssid().unwrap_or_default();
                ui.set_wifi_current_ssid(current.into());
            }
        });
    }

    // Active scan (background thread — keeps the UI responsive during the probe)
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_scan(move || {
            if let Some(ui) = ui_weak.upgrade() {
                if ui.get_wifi_scanning() {
                    return;
                }
                ui.set_wifi_scanning(true);
                wifi_set_status(&ui, "Scanning…");
            }
            let ui_weak2 = ui_weak.clone();
            std::thread::spawn(move || {
                let networks = wifi::list_networks(true);
                let current = wifi::get_current_ssid().unwrap_or_default();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        let entries: Vec<WifiEntry> =
                            networks.iter().map(to_wifi_entry).collect();
                        ui.set_wifi_networks(slint::ModelRc::from(
                            Rc::new(slint::VecModel::from(entries)),
                        ));
                        ui.set_wifi_current_ssid(current.into());
                        ui.set_wifi_scanning(false);
                        wifi_set_status(&ui, "");
                    }
                });
            });
        });
    }

    // Connect to a selected network
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_connect(move |network_idx, password, _security_idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let networks_model = ui.get_wifi_networks();
                let idx = network_idx as usize;
                if network_idx < 0 || idx >= networks_model.row_count() {
                    return;
                }
                let entry = networks_model.row_data(idx).unwrap();
                let ssid = entry.ssid.as_str().to_string();
                let is_open = entry.security.as_str() == "Open";
                let password = wifi::SecretString::from(password.as_str());
                ui.set_wifi_password_input("".into()); // clear UI copy immediately

                ui.set_wifi_connecting(true);
                wifi_set_status(&ui, &format!("Connecting to {}…", ssid));

                let ui_weak2 = ui_weak.clone();
                std::thread::spawn(move || {
                    let result = if is_open {
                        wifi::connect_open(&ssid)
                    } else {
                        wifi::connect(&ssid, password.as_str())
                    };
                    let status = match &result {
                        Ok(_) => format!("Connected to {}", ssid),
                        Err(e) => format!("Failed: {}", e),
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak2.upgrade() {
                            ui.set_wifi_connecting(false);
                            wifi_set_status(&ui, &status);
                            let nets = wifi::list_networks(false);
                            let entries: Vec<WifiEntry> =
                                nets.iter().map(to_wifi_entry).collect();
                            ui.set_wifi_networks(slint::ModelRc::from(
                                Rc::new(slint::VecModel::from(entries)),
                            ));
                            let curr = wifi::get_current_ssid().unwrap_or_default();
                            ui.set_wifi_current_ssid(curr.into());
                        }
                    });
                });
            }
        });
    }

    // Airplane mode toggle
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_set_airplane(move |enabled| {
            let ui_weak2 = ui_weak.clone();
            std::thread::spawn(move || {
                let result = wifi::set_airplane_mode(enabled);
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        match result {
                            Ok(_) => {
                                let status = if enabled {
                                    "Airplane mode on — Wi-Fi radio disabled"
                                } else {
                                    "Airplane mode off — Wi-Fi radio enabled"
                                };
                                wifi_set_status(&ui, status);
                                if !enabled {
                                    // Re-scan after radio comes back on
                                    let nets = wifi::list_networks(false);
                                    let entries: Vec<WifiEntry> =
                                        nets.iter().map(to_wifi_entry).collect();
                                    ui.set_wifi_networks(slint::ModelRc::from(
                                        Rc::new(slint::VecModel::from(entries)),
                                    ));
                                    let curr = wifi::get_current_ssid().unwrap_or_default();
                                    ui.set_wifi_current_ssid(curr.into());
                                } else {
                                    ui.set_wifi_networks(slint::ModelRc::from(
                                        Rc::new(slint::VecModel::from(vec![])),
                                    ));
                                    ui.set_wifi_current_ssid("".into());
                                }
                            }
                            Err(e) => {
                                wifi_set_status(&ui, &format!("Airplane mode error: {}", e));
                                // Revert the toggle on error
                                ui.set_wifi_airplane_mode(!enabled);
                            }
                        }
                    }
                });
            });
        });
    }

    // Disconnect the Wi-Fi interface
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_disconnect(move || {
            if let Some(ui) = ui_weak.upgrade() {
                wifi_set_status(&ui, "Disconnecting…");
                let ui_weak2 = ui_weak.clone();
                std::thread::spawn(move || {
                    let result = wifi::disconnect();
                    let status = match result {
                        Ok(_) => String::new(),
                        Err(e) => format!("Failed: {}", e),
                    };
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(ui) = ui_weak2.upgrade() {
                            wifi_set_status(&ui, &status);
                            ui.set_wifi_current_ssid("".into());
                            let nets = wifi::list_networks(false);
                            let entries: Vec<WifiEntry> =
                                nets.iter().map(to_wifi_entry).collect();
                            ui.set_wifi_networks(slint::ModelRc::from(
                                Rc::new(slint::VecModel::from(entries)),
                            ));
                        }
                    });
                });
            }
        });
    }

    // Forget a saved network profile
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_forget(move |network_idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let networks_model = ui.get_wifi_networks();
                let idx = network_idx as usize;
                if network_idx < 0 || idx >= networks_model.row_count() {
                    return;
                }
                let entry = networks_model.row_data(idx).unwrap();
                let ssid = entry.ssid.as_str().to_string();
                match wifi::forget_network(&ssid) {
                    Ok(_) => {
                        wifi_set_status(&ui, &format!("Forgot {}", ssid));
                        ui.set_wifi_selected_idx(-1);
                        let nets = wifi::list_networks(false);
                        let entries: Vec<WifiEntry> =
                            nets.iter().map(to_wifi_entry).collect();
                        ui.set_wifi_networks(slint::ModelRc::from(
                            Rc::new(slint::VecModel::from(entries)),
                        ));
                    }
                    Err(e) => {
                        wifi_set_status(&ui, &format!("Failed: {}", e));
                    }
                }
            }
        });
    }

    // Generate an in-memory QR code image for the selected network
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_generate_qr(move |network_idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let networks_model = ui.get_wifi_networks();
                let idx = network_idx as usize;
                if network_idx < 0 || idx >= networks_model.row_count() {
                    return;
                }
                let entry = networks_model.row_data(idx).unwrap();
                let ssid = entry.ssid.as_str().to_string();
                let security = entry.security.as_str().to_string();

                // Prefer the user-typed password; fall back to the NM keyring.
                let typed = ui.get_wifi_password_input();
                let password_result: Result<wifi::SecretString, String> = if !typed.is_empty() {
                    Ok(wifi::SecretString::from(typed.as_str()))
                } else {
                    wifi::get_saved_password(&ssid)
                };
                drop(typed); // drop SharedString ref so the UI copy can be cleared
                let auth = wifi::WifiAuth::from_nmcli_security(&security);

                match password_result {
                    Ok(pass) => match wifi::generate_wifi_qr(&ssid, pass.as_str(), &auth) {
                        Some((width, rgb_bytes)) => {
                            let mut buf =
                                slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(width, width);
                            buf.make_mut_bytes().copy_from_slice(&rgb_bytes);
                            ui.set_wifi_qr_image(slint::Image::from_rgb8(buf));
                            ui.set_wifi_qr_visible(true);
                            ui.set_wifi_qr_error("".into());
                        }
                        None => {
                            ui.set_wifi_qr_error(
                                "QR generation failed (data too long?)".into(),
                            );
                        }
                    },
                    Err(e) => {
                        ui.set_wifi_qr_error(
                            format!("Cannot retrieve password: {}", e).into(),
                        );
                        // For open networks generate a no-password QR anyway.
                        if matches!(auth, wifi::WifiAuth::Open) {
                            if let Some((width, rgb_bytes)) =
                                wifi::generate_wifi_qr(&ssid, "", &wifi::WifiAuth::Open)
                            {
                                let mut buf =
                                    slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(
                                        width, width,
                                    );
                                buf.make_mut_bytes().copy_from_slice(&rgb_bytes);
                                ui.set_wifi_qr_image(slint::Image::from_rgb8(buf));
                                ui.set_wifi_qr_visible(true);
                            }
                        }
                    }
                }
            }
        });
    }

    // Connect from a pasted or scanned WIFI: URI string
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_apply_uri(move |uri| {
            if let Some(ui) = ui_weak.upgrade() {
                match wifi::parse_wifi_uri(uri.as_str()) {
                    Some((ssid, password, _auth)) => {
                        ui.set_wifi_connecting(true);
                        wifi_set_status(&ui, &format!("Connecting to {}…", ssid));
                        let ui_weak2 = ui_weak.clone();
                        std::thread::spawn(move || {
                            let result = if password.is_empty() {
                                wifi::connect_open(&ssid)
                            } else {
                                wifi::connect(&ssid, password.as_str())
                            };
                            let status = match &result {
                                Ok(_) => format!("Connected to {}", ssid),
                                Err(e) => format!("Failed: {}", e),
                            };
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(ui) = ui_weak2.upgrade() {
                                    ui.set_wifi_connecting(false);
                                    wifi_set_status(&ui, &status);
                                    ui.set_wifi_uri_input("".into());
                                    let nets = wifi::list_networks(false);
                                    let entries: Vec<WifiEntry> =
                                        nets.iter().map(to_wifi_entry).collect();
                                    ui.set_wifi_networks(slint::ModelRc::from(
                                        Rc::new(slint::VecModel::from(entries)),
                                    ));
                                    let curr = wifi::get_current_ssid().unwrap_or_default();
                                    ui.set_wifi_current_ssid(curr.into());
                                }
                            });
                        });
                    }
                    None => {
                        wifi_set_status(&ui, "Invalid WIFI: QR code format");
                    }
                }
            }
        });
    }

    // Scan screen for QR code with grim + zbarimg
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_scan_screenshot(move || {
            if let Some(ui) = ui_weak.upgrade() {
                wifi_set_status(&ui, "Scanning screen for QR code…");
                match wifi::nmcli::scan_screen_qr() {
                    Ok(uri) if uri.starts_with("WIFI:") => {
                        ui.set_wifi_uri_input(uri.into());
                        wifi_set_status(
                            &ui,
                            "QR code found — press Connect to proceed.",
                        );
                    }
                    Ok(_) => {
                        wifi_set_status(
                            &ui,
                            "QR code found but it is not a Wi-Fi QR code.",
                        );
                    }
                    Err(e) => {
                        wifi_set_status(&ui, &format!("Scan failed: {}", e));
                    }
                }
            }
        });
    }

    // Export QR code to ~/Downloads/ as an SVG file
    {
        let ui_weak = ui.as_weak();
        ui.on_wifi_export_qr(move |network_idx| {
            if let Some(ui) = ui_weak.upgrade() {
                let networks_model = ui.get_wifi_networks();
                let idx = network_idx as usize;
                if network_idx < 0 || idx >= networks_model.row_count() {
                    return;
                }
                let entry = networks_model.row_data(idx).unwrap();
                let ssid = entry.ssid.as_str().to_string();
                let security = entry.security.as_str().to_string();

                let typed = ui.get_wifi_password_input();
                let password_result: Result<wifi::SecretString, String> = if !typed.is_empty() {
                    Ok(wifi::SecretString::from(typed.as_str()))
                } else {
                    wifi::get_saved_password(&ssid)
                };
                drop(typed);
                let auth = wifi::WifiAuth::from_nmcli_security(&security);

                match password_result {
                    Ok(pass) => {
                        let uri = wifi::wifi_uri(&ssid, pass.as_str(), &auth);
                        let home =
                            std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                        // Sanitise SSID for use as a filename.
                        let safe: String = ssid
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() || c == '-' || c == '_' {
                                    c
                                } else {
                                    '_'
                                }
                            })
                            .collect();
                        let _ = std::fs::create_dir_all(format!("{}/Downloads", home));
                        let path = format!("{}/Downloads/wifi-{}.svg", home, safe);
                        match wifi::export_qr_svg(uri.as_str(), &path) {
                            Ok(_) => {
                                wifi_set_status(
                                    &ui,
                                    &format!(
                                        "Saved to ~/Downloads/wifi-{}.svg",
                                        safe
                                    ),
                                );
                                let _ = std::process::Command::new("xdg-open")
                                    .arg(&path)
                                    .spawn();
                            }
                            Err(e) => {
                                wifi_set_status(
                                    &ui,
                                    &format!("Export failed: {}", e),
                                );
                            }
                        }
                    }
                    Err(e) => {
                        wifi_set_status(
                            &ui,
                            &format!("Cannot retrieve password: {}", e),
                        );
                    }
                }
            }
        });
    }

    // ── Startup callbacks ────────────────────────────────────────────────────

    {
        let ui_handle = ui.as_weak();
        ui.on_startup_load(move || {
            let ui = ui_handle.unwrap();
            refresh_startup_items(&ui);
        });
    }

    {
        let ui_handle = ui.as_weak();
        ui.on_startup_toggle(move |idx, enable| {
            let ui = ui_handle.unwrap();
            let items = startup::list_items();
            if let Some(item) = items.get(idx as usize) {
                startup::toggle_item(item, enable);
                refresh_startup_items(&ui);
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        ui.on_startup_add(move |idx| {
            let ui = ui_handle.unwrap();
            let apps = startup::list_available_apps();
            if let Some(app) = apps.get(idx as usize) {
                match startup::add_app(app) {
                    Ok(()) => {
                        refresh_startup_items(&ui);
                        ui.set_startup_status_text(
                            slint::SharedString::from(format!("Added {}", app.name)),
                        );
                    }
                    Err(e) => {
                        ui.set_startup_status_text(slint::SharedString::from(e));
                    }
                }
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        ui.on_startup_remove(move |idx| {
            let ui = ui_handle.unwrap();
            let items = startup::list_items();
            if let Some(item) = items.get(idx as usize) {
                if item.source == "xdg" {
                    let _ = startup::remove_xdg_entry(&item.name);
                    refresh_startup_items(&ui);
                }
            }
        });
    }

    {
        let ui_handle = ui.as_weak();
        ui.on_startup_filter_apps(move |filter| {
            let ui = ui_handle.unwrap();
            refresh_available_apps(&ui, filter.as_str());
        });
    }

    // ── Search callback ──────────────────────────────────────────────────────

    {
        fn tab_display_name(key: &str) -> &str {
            match key {
                "about" => "About",
                "keyboard" => "Keyboard",
                "dictation" => "Dictation",
                "display" => "Display",
                "power" => "Power",
                "keybindings" => "Keybindings",
                "taskbar" => "Taskbar",
                "wifi" => "Wi-Fi",
                "startup" => "Startup",
                _ => key,
            }
        }

        let search_index = settings_search_index();

        let ui_handle = ui.as_weak();
        ui.on_filter_search(move |query| {
            let ui = ui_handle.unwrap();
            let q = query.to_lowercase();
            if q.is_empty() {
                ui.set_search_results(slint::ModelRc::default());
                return;
            }

            let results: Vec<SearchResult> = search_index
                .iter()
                .filter(|(label, _, _)| fuzzy_match(label, &q))
                .map(|(label, tab, idx)| SearchResult {
                    label: slint::SharedString::from(*label),
                    tab_name: slint::SharedString::from(tab_display_name(tab)),
                    tab_index: *idx,
                })
                .collect();

            ui.set_search_results(slint::ModelRc::from(
                Rc::new(slint::VecModel::from(results)),
            ));
        });
    }

    // ── Deep-link polling timer ──────────────────────────────────────────────

    {
        let ui_weak = ui.as_weak();
        let deeplink_timer = slint::Timer::default();
        deeplink_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(500),
            move || {
                let run_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
                let path = format!("{}/settings-deeplink", run_dir);
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let _ = std::fs::remove_file(&path);
                    let lines: Vec<&str> = content.lines().collect();
                    if lines.len() >= 2 {
                        if let (Ok(tab), highlight) = (lines[0].parse::<i32>(), lines[1].to_string()) {
                            if let Some(ui) = ui_weak.upgrade() {
                                ui.set_active_tab(tab);
                                ui.set_highlight_setting(slint::SharedString::from(&highlight));
                                start_highlight_blink(&ui);
                            }
                        }
                    }
                }
            },
        );
        std::mem::forget(deeplink_timer);
    }

    // ── Theme polling timer ──────────────────────────────────────────────────

    {
        let ui_weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(2),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    apply_theme(&ui);

                    if ui.get_dictation_installing() {
                        let (progress, text) = dictation::read_progress();
                        ui.set_dictation_progress(progress);
                        if !text.is_empty() {
                            ui.set_dictation_progress_text(text.clone().into());
                        }

                        if progress == 0.0 && text.starts_with("Error") {
                            ui.set_dictation_install_error(true);
                        }

                        if progress >= 1.0 {
                            let ui_weak2 = ui.as_weak();
                            slint::Timer::single_shot(
                                std::time::Duration::from_secs(2),
                                move || {
                                    if let Some(ui) = ui_weak2.upgrade() {
                                        ui.set_dictation_installing(false);
                                        dictation::clear_progress();

                                        let installed = dictation::is_installed();
                                        ui.set_dictation_installed(installed);
                                        if installed {
                                            if let Some(cfg) = dictation::read_config() {
                                                ui.set_dictation_language(dictation::language_display(&cfg));
                                                ui.set_dictation_model(dictation::model_display(&cfg.model));
                                                if let Some(idx) = dictation::find_language_idx(&cfg.primary_code) {
                                                    ui.set_dictation_selected_lang_name(
                                                        dictation::LANGUAGES[idx].name.into(),
                                                    );
                                                }
                                                if let Some(idx) = dictation::find_model_idx(&cfg.model) {
                                                    ui.set_dictation_selected_model_idx(idx as i32);
                                                }
                                                ui.set_dictation_also_english(cfg.also_english);
                                                let is_en = cfg.primary_code == "en" || cfg.primary_code == "auto";
                                                ui.set_dictation_show_also_english(!is_en);
                                                ui.set_dictation_config_missing(false);
                                            } else {
                                                ui.set_dictation_config_missing(true);
                                                ui.set_dictation_language("(no config)".into());
                                                ui.set_dictation_model("(no config)".into());
                                            }
                                            ui.set_dictation_service_running(dictation::is_service_running());
                                        }
                                    }
                                },
                            );
                        }
                    } else {
                        let installed = dictation::is_installed();
                        ui.set_dictation_installed(installed);
                        if installed {
                            if let Some(cfg) = dictation::read_config() {
                                ui.set_dictation_language(dictation::language_display(&cfg));
                                ui.set_dictation_model(dictation::model_display(&cfg.model));
                                ui.set_dictation_config_missing(false);
                            } else if !dictation::config_exists() {
                                ui.set_dictation_config_missing(true);
                                ui.set_dictation_language("(no config)".into());
                                ui.set_dictation_model("(no config)".into());
                            }
                            ui.set_dictation_service_running(dictation::is_service_running());
                        }
                    }

                    // Refresh about uptime
                    let uptime = std::process::Command::new("uptime")
                        .arg("-p")
                        .output()
                        .ok()
                        .map(|o| {
                            String::from_utf8_lossy(&o.stdout)
                                .trim()
                                .strip_prefix("up ")
                                .unwrap_or("unknown")
                                .to_string()
                        })
                        .unwrap_or_default();
                    ui.set_about_uptime(slint::SharedString::from(uptime));
                }
            },
        );
        std::mem::forget(timer);
    }

    ui.run()
}
