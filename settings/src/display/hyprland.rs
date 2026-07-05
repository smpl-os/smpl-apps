use serde::Deserialize;
use std::process::Command;

use super::backend::DisplayBackend;
use super::monitor::{Monitor, MonitorConfig, MonitorMode};

pub struct HyprlandBackend;

impl HyprlandBackend {
    pub fn new() -> Self {
        Self
    }

    /// After a monitor re-arrangement, pull windows that ended up outside the
    /// combined desktop bounds back into view. Floating windows are centered;
    /// tiled windows reflow automatically when re-added to their workspace.
    fn recover_offscreen_windows(&self, configs: &[MonitorConfig]) {
        let mut max_x = 0i32;
        let mut max_y = 0i32;
        for c in configs.iter().filter(|c| c.enabled) {
            let w = (if c.transform % 2 == 1 { c.height } else { c.width } as f64 / c.scale) as i32;
            let h = (if c.transform % 2 == 1 { c.width } else { c.height } as f64 / c.scale) as i32;
            max_x = max_x.max(c.x + w);
            max_y = max_y.max(c.y + h);
        }
        if max_x == 0 || max_y == 0 {
            return;
        }
        let clients_json = match self.hyprctl(&["clients", "-j"]) {
            Ok(j) => j,
            Err(_) => return,
        };
        let clients: Vec<serde_json::Value> =
            serde_json::from_str(&clients_json).unwrap_or_default();
        for c in clients {
            let addr = c.get("address").and_then(|v| v.as_str()).unwrap_or("");
            let floating = c.get("floating").and_then(|v| v.as_bool()).unwrap_or(false);
            if addr.is_empty() {
                continue;
            }
            if let Some(at) = c.get("at").and_then(|v| v.as_array()) {
                let x = at.first().and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                let y = at.get(1).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                if x < 0 || y < 0 || x >= max_x || y >= max_y {
                    if floating {
                        let _ = self.hyprctl(&["dispatch", "centerwindow", &format!("address:{addr}")]);
                    } else {
                        let _ = self.hyprctl(&["dispatch", "movetoworkspace", &format!("+0,address:{addr}")]);
                    }
                }
            }
        }
    }

    fn hyprctl(&self, args: &[&str]) -> Result<String, String> {
        let output = Command::new("hyprctl")
            .args(args)
            .output()
            .map_err(|e| format!("Failed to run hyprctl: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("hyprctl {} failed: {stderr}", args.join(" ")));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }
}

#[derive(Debug, Deserialize)]
struct HyprMonitor {
    id: i32,
    name: String,
    description: String,
    width: i32,
    height: i32,
    #[serde(rename = "refreshRate")]
    refresh_rate: f64,
    x: i32,
    y: i32,
    scale: f64,
    transform: i32,
    focused: bool,
    #[serde(rename = "dpmsStatus")]
    dpms_status: bool,
    disabled: bool,
    #[serde(rename = "availableModes")]
    available_modes: Vec<String>,
}

fn parse_mode(s: &str) -> Option<MonitorMode> {
    let s = s.trim_end_matches("Hz");
    let (res, hz) = s.split_once('@')?;
    let (w, h) = res.split_once('x')?;
    Some(MonitorMode {
        width: w.parse().ok()?,
        height: h.parse().ok()?,
        refresh_rate: hz.parse().ok()?,
    })
}

impl DisplayBackend for HyprlandBackend {
    fn query_monitors(&self) -> Result<Vec<Monitor>, String> {
        let json = self.hyprctl(&["monitors", "-j"])?;
        let raw: Vec<HyprMonitor> =
            serde_json::from_str(&json).map_err(|e| format!("Failed to parse monitors JSON: {e}"))?;

        let monitors = raw
            .into_iter()
            .map(|m| {
                let available_modes: Vec<MonitorMode> =
                    m.available_modes.iter().filter_map(|s| parse_mode(s)).collect();

                Monitor {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    width: m.width,
                    height: m.height,
                    refresh_rate: m.refresh_rate,
                    x: m.x,
                    y: m.y,
                    scale: m.scale,
                    transform: m.transform,
                    enabled: !m.disabled,
                    dpms: m.dpms_status,
                    focused: m.focused,
                    available_modes,
                }
            })
            .collect();

        Ok(monitors)
    }

    fn apply(&self, configs: &[MonitorConfig]) -> Result<(), String> {
        // Stop Eww BEFORE applying monitor changes to avoid windows appearing
        // on wrong monitors during the transition
        let _ = Command::new("bash")
            .args(["-c", "bar-ctl stop 2>/dev/null"])
            .output();

        // Re-source the persisted monitors.conf (written by persist() *before*
        // this call). Going through `hyprctl reload` — the same mechanism the
        // keybindings/layout/idle settings use — is authoritative: a live
        // `keyword monitor` batch that bundles a rotation with a repositioning
        // can be silently dropped by Hyprland, so the transform/scale would not
        // stick. Reload re-runs the Lua config that parses monitors.conf, which
        // applies the exact layout that was just written to disk.
        self.hyprctl(&["reload"])?;

        // Wait a moment for Hyprland to stabilize the new layout
        std::thread::sleep(std::time::Duration::from_millis(300));

        // Restart Eww AFTER monitor changes are applied and stabilized so the
        // bars anchor to the final (rotated/scaled) monitor geometry.
        let _ = Command::new("bash")
            .args(["-c", "bar-ctl start 2>/dev/null"])
            .output();

        // Recover any window left off-screen by the re-arrangement: bind every
        // workspace to a live monitor and re-center floating windows whose
        // top-left now sits outside the combined desktop bounds.
        self.recover_offscreen_windows(configs);

        Ok(())
    }

    fn persist(&self, configs: &[MonitorConfig]) -> Result<String, String> {
        let config_dir =
            dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("~/.config"));
        let monitors_conf = config_dir.join("hypr").join("monitors.conf");

        let mut content = String::from(
            "# Generated by settings -- edit freely, re-running settings will overwrite.\n\n",
        );

        for c in configs {
            content.push_str(&c.to_hyprland_line());
            content.push('\n');
        }

        std::fs::write(&monitors_conf, &content)
            .map_err(|e| format!("Failed to write {}: {e}", monitors_conf.display()))?;

        Ok(monitors_conf.display().to_string())
    }

    fn set_primary(&self, monitor_name: &str) -> Result<(), String> {
        self.hyprctl(&[
            "dispatch",
            "moveworkspacetomonitor",
            &format!("1 {monitor_name}"),
        ])?;
        Ok(())
    }

    fn identify(&self, monitors: &[Monitor]) -> Result<(), String> {
        // Use eww overlay windows — each defwindow targets a specific monitor
        // by index so labels appear simultaneously on every screen for 5 s
        // without touching the cursor.
        let home = std::env::var("HOME").unwrap_or_default();
        let cfg = format!("{home}/.config/eww");

        // Update the eww label variables for each monitor (up to 4)
        for (idx, m) in monitors.iter().enumerate().take(4) {
            let arg = format!("mon-id-label-{idx}={}  {}", idx + 1, m.name);
            let _ = Command::new("eww").args(["-c", &cfg, "update", &arg]).output();
        }

        let wins: Vec<String> = (0..monitors.len().min(4))
            .map(|i| format!("mon-identify-{i}"))
            .collect();

        // Close any stale instances first
        let _ = Command::new("eww").arg("-c").arg(&cfg)
            .arg("close").args(&wins).output();

        // Open all windows simultaneously
        let _ = Command::new("eww").arg("-c").arg(&cfg)
            .arg("open-many").args(&wins).output();

        // Close after 5 s in a detached thread
        let cfg2 = cfg.clone();
        let wins2 = wins.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let _ = std::process::Command::new("eww")
                .arg("-c").arg(&cfg2)
                .arg("close").args(&wins2)
                .output();
        });

        Ok(())
    }

    fn name(&self) -> &'static str {
        "Hyprland"
    }
}
