//! Startup Apps backend — manage autostart entries for Hyprland sessions.
//!
//! Three sources of startup items:
//! 1. Hyprland `exec-once` lines in `~/.config/hypr/autostart.conf`
//! 2. Systemd user services (enabled/disabled)
//! 3. XDG autostart `.desktop` files in `~/.config/autostart/`
//!
//! User-added apps are stored as XDG autostart desktop files.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// A startup item shown in the UI.
#[derive(Clone, Debug)]
pub struct StartupItem {
    /// Display name (e.g. "Signal", "hypridle", "dunst")
    pub name: String,
    /// The command or service name
    pub command: String,
    /// Source: "hyprland", "systemd", "xdg"
    pub source: String,
    /// Whether it is currently enabled
    pub enabled: bool,
    /// Whether the user can toggle it (system-critical items are locked)
    pub toggleable: bool,
    /// Description or comment from .desktop file
    pub description: String,
}

// Items that should not be toggled off — they are critical for the smplOS session.
const SYSTEM_CRITICAL: &[&str] = &[
    "hypridle",
    "dunst",
    "hyprshell",
    "kb-sync",
    "theme-bg-init",
    "bar-ctl",
    "workspace-group",
    "gnome-keyring-daemon",
    "polkit-gnome-authentication-agent-1",
    "systemctl --user import-environment",
    "dbus-update-activation-environment",
    "smplos-first-run",
    "smplos-appimage-setup",
    "smplos-flatpak-setup",
    "generate-messenger-bindings",
    "automount",
];

fn autostart_conf_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config/hypr/autostart.conf")
}

fn xdg_autostart_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    home.join(".config/autostart")
}

fn is_system_critical(cmd: &str) -> bool {
    SYSTEM_CRITICAL.iter().any(|c| cmd.contains(c))
}

/// Clean up an exec-once command for display purposes.
fn display_name_from_cmd(cmd: &str) -> String {
    let cmd = cmd.trim();
    // Strip bash -c wrapper
    let inner = if cmd.starts_with("bash -c") || cmd.starts_with("sh -c") {
        cmd.splitn(2, '\'')
            .nth(1)
            .and_then(|s| s.rsplit_once('\''))
            .map(|(s, _)| s)
            .unwrap_or(cmd)
    } else {
        cmd
    };
    // Strip sleep prefix
    let inner = if inner.contains("sleep") && inner.contains("&&") {
        inner.split("&&").last().unwrap_or(inner).trim()
    } else {
        inner
    };
    // Get the first word (binary name)
    let bin = inner.split_whitespace().next().unwrap_or(inner);
    // Strip path
    bin.rsplit('/').next().unwrap_or(bin).to_string()
}

// ── Hyprland autostart.conf ──────────────────────────────────────────────────

/// Parse exec-once lines from autostart.conf.
fn read_hyprland_entries() -> Vec<StartupItem> {
    let path = autostart_conf_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut items = Vec::new();
    let mut comment_buf = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            // Accumulate comment as description for the next exec-once
            let cmt = trimmed.trim_start_matches('#').trim();
            if !cmt.is_empty() {
                if !comment_buf.is_empty() {
                    comment_buf.push_str(" — ");
                }
                comment_buf.push_str(cmt);
            }
            continue;
        }
        if let Some(cmd) = trimmed.strip_prefix("exec-once") {
            let cmd = cmd.trim().trim_start_matches('=').trim();
            if !cmd.is_empty() {
                let name = display_name_from_cmd(cmd);
                let critical = is_system_critical(cmd);
                items.push(StartupItem {
                    name,
                    command: cmd.to_string(),
                    source: "hyprland".into(),
                    enabled: true,
                    toggleable: !critical,
                    description: std::mem::take(&mut comment_buf),
                });
            }
            comment_buf.clear();
        } else if !trimmed.is_empty() {
            comment_buf.clear();
        }
    }
    items
}

// ── Systemd user services ────────────────────────────────────────────────────

fn read_systemd_user_services() -> Vec<StartupItem> {
    // List enabled user services
    let output = match std::process::Command::new("systemctl")
        .args(["--user", "list-unit-files", "--type=service", "--no-legend"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output.stdout);
    let mut items = Vec::new();

    // Skip services that are sockets/timers targets or managed elsewhere
    let skip_prefixes = [
        "dbus", "pipewire", "wireplumber", "xdg-user-dirs",
        "p11-kit", "gcr-ssh", "gnome-keyring",
        "systemd-", "podman",
    ];

    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let unit = parts[0];
        let state = parts[1]; // enabled, disabled, static, masked, etc.

        // Only show enabled and disabled (user-toggleable)
        if state != "enabled" && state != "disabled" {
            continue;
        }

        let name = unit.trim_end_matches(".service");

        // Skip infrastructure services
        if skip_prefixes.iter().any(|p| name.starts_with(p)) {
            continue;
        }

        items.push(StartupItem {
            name: name.to_string(),
            command: unit.to_string(),
            source: "systemd".into(),
            enabled: state == "enabled",
            toggleable: true,
            description: format!("Systemd user service ({})", state),
        });
    }

    items
}

// ── XDG autostart ────────────────────────────────────────────────────────────

fn read_xdg_autostart() -> Vec<StartupItem> {
    let dir = xdg_autostart_dir();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut items = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }

        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut name = String::new();
        let mut exec = String::new();
        let mut comment = String::new();
        let mut hidden = false;

        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("Name=") {
                name = val.to_string();
            } else if let Some(val) = line.strip_prefix("Exec=") {
                exec = val.to_string();
            } else if let Some(val) = line.strip_prefix("Comment=") {
                comment = val.to_string();
            } else if line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false" {
                hidden = true;
            }
        }

        if name.is_empty() {
            name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
        }

        items.push(StartupItem {
            name,
            command: exec,
            source: "xdg".into(),
            enabled: !hidden,
            toggleable: true,
            description: comment,
        });
    }

    items
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Get all startup items from all sources, deduplicated.
pub fn list_items() -> Vec<StartupItem> {
    let mut items = Vec::new();
    let mut seen_names = HashSet::new();

    // Hyprland autostart entries first
    for item in read_hyprland_entries() {
        seen_names.insert(item.name.to_lowercase());
        items.push(item);
    }

    // Systemd user services
    for item in read_systemd_user_services() {
        if !seen_names.contains(&item.name.to_lowercase()) {
            seen_names.insert(item.name.to_lowercase());
            items.push(item);
        }
    }

    // XDG autostart entries
    for item in read_xdg_autostart() {
        if !seen_names.contains(&item.name.to_lowercase()) {
            seen_names.insert(item.name.to_lowercase());
            items.push(item);
        }
    }

    items
}

/// Toggle a Hyprland autostart entry on/off by commenting/uncommenting.
pub fn toggle_hyprland_entry(command: &str, enable: bool) {
    let path = autostart_conf_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut new_lines: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Match disabled entry: "# DISABLED: exec-once = <cmd>"
        if let Some(rest) = trimmed.strip_prefix("# DISABLED: exec-once") {
            let rest_cmd = rest.trim().trim_start_matches('=').trim();
            if rest_cmd == command {
                if enable {
                    new_lines.push(format!("exec-once = {}", command));
                } else {
                    new_lines.push(line.to_string());
                }
                continue;
            }
        }

        // Match enabled entry
        if let Some(cmd) = trimmed.strip_prefix("exec-once") {
            let cmd = cmd.trim().trim_start_matches('=').trim();
            if cmd == command {
                if enable {
                    new_lines.push(line.to_string());
                } else {
                    new_lines.push(format!("# DISABLED: exec-once = {}", command));
                }
                continue;
            }
        }

        new_lines.push(line.to_string());
    }

    let _ = fs::write(&path, new_lines.join("\n") + "\n");
}

/// Toggle a systemd user service on/off.
pub fn toggle_systemd_service(unit: &str, enable: bool) {
    let action = if enable { "enable" } else { "disable" };
    let _ = std::process::Command::new("systemctl")
        .args(["--user", action, unit])
        .output();
}

/// Toggle an XDG autostart desktop file on/off.
pub fn toggle_xdg_entry(name: &str, enable: bool) {
    let dir = xdg_autostart_dir();
    // Find the desktop file by name
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Check if Name= matches
            let file_name = content
                .lines()
                .find(|l| l.starts_with("Name="))
                .map(|l| l.trim_start_matches("Name=").to_string())
                .unwrap_or_default();

            let stem_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            if file_name == name || stem_name == name {
                // Rewrite the file: remove Hidden= line, then add or not
                let mut new_lines: Vec<String> = Vec::new();

                for line in content.lines() {
                    if line.starts_with("Hidden=") || line.starts_with("X-GNOME-Autostart-enabled=") {
                        continue;
                    }
                    new_lines.push(line.to_string());
                }

                if !enable {
                    // Add Hidden=true before the first empty line or at end
                    let insert_pos = new_lines
                        .iter()
                        .position(|l| l.trim().is_empty())
                        .unwrap_or(new_lines.len());
                    // Insert after [Desktop Entry] header
                    let pos = new_lines
                        .iter()
                        .position(|l| l.starts_with("[Desktop Entry]"))
                        .map(|i| i + 1)
                        .unwrap_or(insert_pos);
                    new_lines.insert(pos, "Hidden=true".to_string());
                }

                let _ = fs::write(&path, new_lines.join("\n") + "\n");
                return;
            }
        }
    }
}

/// Toggle a startup item (dispatches to the right source).
pub fn toggle_item(item: &StartupItem, enable: bool) {
    match item.source.as_str() {
        "hyprland" => toggle_hyprland_entry(&item.command, enable),
        "systemd" => toggle_systemd_service(&item.command, enable),
        "xdg" => toggle_xdg_entry(&item.name, enable),
        _ => {}
    }
}

// ── Add / remove apps ────────────────────────────────────────────────────────

/// Represents an installable app that can be added to autostart.
#[derive(Clone, Debug)]
pub struct AvailableApp {
    /// Display name from .desktop file
    pub name: String,
    /// Exec command from .desktop file
    pub exec: String,
    /// Desktop file path
    pub desktop_file: String,
    /// Comment/description
    pub description: String,
}

/// Scan /usr/share/applications and ~/.local/share/applications for apps
/// that are NOT already in autostart.
pub fn list_available_apps() -> Vec<AvailableApp> {
    let autostart_names: HashSet<String> = list_items()
        .iter()
        .map(|i| i.name.to_lowercase())
        .collect();

    let mut apps = Vec::new();
    let dirs = [
        PathBuf::from("/usr/share/applications"),
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".local/share/applications"),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".local/share/flatpak/exports/share/applications"),
    ];

    let mut seen = HashSet::new();

    for dir in &dirs {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }

            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Skip non-application entries
            if !content.contains("Type=Application") {
                continue;
            }
            // Skip NoDisplay entries
            if content.contains("NoDisplay=true") {
                continue;
            }

            let mut name = String::new();
            let mut exec = String::new();
            let mut comment = String::new();

            for line in content.lines() {
                let line = line.trim();
                // Only parse [Desktop Entry] section
                if line.starts_with('[') && line != "[Desktop Entry]" {
                    break;
                }
                if let Some(val) = line.strip_prefix("Name=") {
                    name = val.to_string();
                } else if let Some(val) = line.strip_prefix("Exec=") {
                    // Strip %u, %U, %f, %F etc.
                    exec = val
                        .replace(" %u", "")
                        .replace(" %U", "")
                        .replace(" %f", "")
                        .replace(" %F", "")
                        .replace(" --", "")
                        .trim()
                        .to_string();
                } else if let Some(val) = line.strip_prefix("Comment=") {
                    comment = val.to_string();
                }
            }

            if name.is_empty() || exec.is_empty() {
                continue;
            }

            // Skip if already in autostart
            if autostart_names.contains(&name.to_lowercase()) {
                continue;
            }

            // Deduplicate by name
            if seen.contains(&name.to_lowercase()) {
                continue;
            }
            seen.insert(name.to_lowercase());

            apps.push(AvailableApp {
                name,
                exec,
                desktop_file: path.to_string_lossy().to_string(),
                description: comment,
            });
        }
    }

    apps.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    apps
}

/// Add an app to autostart by creating an XDG autostart .desktop file.
pub fn add_app(app: &AvailableApp) -> Result<(), String> {
    let dir = xdg_autostart_dir();
    let _ = fs::create_dir_all(&dir);

    // Sanitize name for filename
    let filename: String = app
        .name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let path = dir.join(format!("{}.desktop", filename));

    let content = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Exec={}\n\
         Comment={}\n\
         X-GNOME-Autostart-enabled=true\n",
        app.name, app.exec, app.description
    );

    fs::write(&path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

/// Remove an app from XDG autostart by deleting its .desktop file.
pub fn remove_xdg_entry(name: &str) -> Result<(), String> {
    let dir = xdg_autostart_dir();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let content = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let file_name = content
                .lines()
                .find(|l| l.starts_with("Name="))
                .map(|l| l.trim_start_matches("Name=").to_string())
                .unwrap_or_default();

            let stem_name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            if file_name == name || stem_name == name {
                return fs::remove_file(&path)
                    .map_err(|e| format!("Failed to remove {}: {}", path.display(), e));
            }
        }
    }
    Err(format!("Autostart entry '{}' not found", name))
}
