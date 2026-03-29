//! NetworkManager integration via the `nmcli` command-line tool.
//!
//! Every public function is self-contained: it spawns an `nmcli` child process,
//! waits for it to exit, then returns structured data or an error string.  No
//! global state is mutated.  Passwords travel only through argv of child processes
//! (the kernel passes them as a byte sequence without writing them to any file).
//!
//! # Memory safety for credentials
//!
//! * Stdout buffers that contain password data are zeroed with `write_volatile`
//!   (via [`zeroize::Zeroize`]) immediately after the password is extracted.
//! * The returned password is wrapped in [`SecretString`] which zeroes its heap
//!   allocation when dropped.

use std::process::Command;
use zeroize::Zeroize;

use super::secure::SecretString;

// ── Data types ────────────────────────────────────────────────────────────────

/// A discovered Wi-Fi access point.
#[derive(Debug, Clone)]
pub struct WifiNetwork {
    /// Human-readable network name.
    pub ssid: String,
    /// MAC address of the access point (may be empty if hidden).
    pub bssid: String,
    /// Signal strength 0–100 (as reported by nmcli).
    pub signal: i32,
    /// Security description: "WPA2", "WPA3", "WPA2 WPA3", "WEP", or "Open".
    pub security: String,
    /// Whether this is the currently active connection.
    pub connected: bool,
    /// Whether a saved/known profile exists in NetworkManager.
    pub saved: bool,
}

impl WifiNetwork {
    /// True when the network requires a passphrase.
    #[allow(dead_code)]
    pub fn is_secured(&self) -> bool {
        self.security != "Open" && self.security != "--" && !self.security.is_empty()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Split an nmcli `--terse` output line on unescaped `:` characters.
/// nmcli escapes literal colons in field values as `\:`.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            // Escaped character — consume the next char literally.
            if let Some(nc) = chars.next() {
                current.push(nc);
            }
        } else if c == ':' {
            fields.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

/// Parse nmcli security string into a canonical form.
fn parse_security(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() || s == "--" {
        return "Open".to_string();
    }
    // nmcli may report "WPA1 WPA2", "WPA2", "WPA3", "WPA2 WPA3", "WEP", etc.
    s.to_string()
}

/// Collect the names of saved 802-11-wireless NetworkManager profiles.
fn saved_ssids() -> Vec<String> {
    let out = Command::new("nmcli")
        .args(["--terse", "--fields", "NAME,TYPE", "connection", "show"])
        .output()
        .ok();
    let stdout = out.map(|o| o.stdout).unwrap_or_default();
    String::from_utf8_lossy(&stdout)
        .lines()
        .filter_map(|line| {
            let parts = split_terse(line);
            if parts.len() >= 2 && parts[1].trim() == "802-11-wireless" {
                Some(parts[0].clone())
            } else {
                None
            }
        })
        .collect()
}

/// Return the WiFi interface name (e.g. `wlan0`), or `None` if not found.
fn wifi_interface() -> Option<String> {
    let out = Command::new("nmcli")
        .args(["--terse", "--fields", "DEVICE,TYPE", "device", "status"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let parts = split_terse(line);
            if parts.len() >= 2 && parts[1].trim() == "wifi" {
                Some(parts[0].clone())
            } else {
                None
            }
        })
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Scan and return available Wi-Fi networks.
///
/// `rescan` controls whether nmcli triggers an active probe:
/// - `true`  → `--rescan yes`  (slower, up-to-date results)
/// - `false` → `--rescan no`   (fast, uses cached results)
pub fn list_networks(rescan: bool) -> Vec<WifiNetwork> {
    let rescan_arg = if rescan { "yes" } else { "no" };

    let out = Command::new("nmcli")
        .args([
            "--terse",
            "--fields",
            "IN-USE,SSID,BSSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "--rescan",
            rescan_arg,
        ])
        .output()
        .ok();
    let stdout = out.map(|o| o.stdout).unwrap_or_default();

    let saved = saved_ssids();

    let mut networks: Vec<WifiNetwork> = String::from_utf8_lossy(&stdout)
        .lines()
        .filter_map(|line| {
            let parts = split_terse(line);
            if parts.len() < 5 {
                return None;
            }
            let in_use = parts[0].trim() == "*";
            let ssid = parts[1].clone();
            // Skip hidden networks with blank SSID
            if ssid.trim().is_empty() {
                return None;
            }
            let bssid = parts[2].clone();
            let signal = parts[3].trim().parse::<i32>().unwrap_or(0);
            let security = parse_security(&parts[4]);
            let is_saved = saved.iter().any(|s| s == &ssid);

            Some(WifiNetwork {
                ssid,
                bssid,
                signal,
                security,
                connected: in_use,
                saved: is_saved,
            })
        })
        .collect();

    // Deduplicate by SSID (keep the entry with the strongest signal).
    networks.sort_by(|a, b| b.signal.cmp(&a.signal));
    networks.dedup_by(|a, b| {
        if a.ssid == b.ssid {
            // `b` is the higher-signal entry (comes first after sort).
            // Merge: mark `b` as connected/saved if either entry is.
            b.connected |= a.connected;
            b.saved |= a.saved;
            true
        } else {
            false
        }
    });

    // Put connected network at top, then sort by signal strength.
    networks.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.signal.cmp(&a.signal))
    });

    networks
}

/// Return the SSID of the currently active Wi-Fi connection, if any.
pub fn get_current_ssid() -> Option<String> {
    let out = Command::new("nmcli")
        .args(["--terse", "--fields", "ACTIVE,SSID", "device", "wifi"])
        .output()
        .ok()?;

    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let parts = split_terse(line);
            if parts.len() >= 2 && parts[0].trim() == "yes" {
                let ssid = parts[1].trim().to_string();
                if !ssid.is_empty() {
                    Some(ssid)
                } else {
                    None
                }
            } else {
                None
            }
        })
}

/// Retrieve the saved WPA passphrase for `ssid` from the NetworkManager keyring.
///
/// Requires that the user is allowed to view secrets (polkit may prompt).
/// Returns a [`SecretString`] that zeroes its heap allocation when dropped.
/// Returns `Err(message)` if the password cannot be retrieved.
pub fn get_saved_password(ssid: &str) -> Result<SecretString, String> {
    let mut out = Command::new("nmcli")
        .args([
            "--show-secrets",
            "--terse",
            "--get-values",
            "802-11-wireless-security.psk",
            "connection",
            "show",
            ssid,
        ])
        .output()
        .map_err(|e| format!("nmcli error: {}", e))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Zero the stdout buffer (may contain partial password data) before returning.
        out.stdout.zeroize();
        return Err(format!(
            "Could not retrieve password: {}",
            stderr.trim()
        ));
    }

    // Extract the password into a SecretString FIRST, then zero the raw stdout.
    let password = SecretString::from(
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    );
    // Zero the raw stdout buffer so no cleartext bytes linger in the Vec allocation.
    out.stdout.zeroize();

    if password.is_empty() {
        Err("No passphrase stored for this network".to_string())
    } else {
        Ok(password)
    }
}

/// Connect to a secured Wi-Fi network.
///
/// If a saved profile already exists for `ssid`, nmcli will use it
/// and the `password` you provide will update the stored credential.
pub fn connect(ssid: &str, password: &str) -> Result<(), String> {
    // Password is passed as a separate argv element — the kernel never writes
    // it to any file; it's visible in /proc/<pid>/cmdline only while the child
    // is alive, and only to root on Linux.
    let status = Command::new("nmcli")
        .args(["device", "wifi", "connect", ssid, "password", password])
        .status()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "nmcli exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Connect to an open (unsecured) Wi-Fi network.
pub fn connect_open(ssid: &str) -> Result<(), String> {
    let status = Command::new("nmcli")
        .args(["device", "wifi", "connect", ssid])
        .status()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "nmcli exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Disconnect the active Wi-Fi interface.
pub fn disconnect() -> Result<(), String> {
    let iface = wifi_interface().ok_or_else(|| "No Wi-Fi interface found".to_string())?;
    let status = Command::new("nmcli")
        .args(["device", "disconnect", &iface])
        .status()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "nmcli exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Enable or disable the Wi-Fi radio (airplane-mode equivalent).
///
/// `airplane = true`  → `nmcli radio wifi off` (radio off, no scanning/connecting)
/// `airplane = false` → `nmcli radio wifi on`  (radio on)
pub fn set_airplane_mode(airplane: bool) -> Result<(), String> {
    let switch = if airplane { "off" } else { "on" };
    let status = Command::new("nmcli")
        .args(["radio", "wifi", switch])
        .status()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "nmcli radio wifi {} failed (code {})",
            switch,
            status.code().unwrap_or(-1)
        ))
    }
}

/// Query the current Wi-Fi radio state.
///
/// Returns:
/// - `Ok(Some(true))`  → radio is enabled
/// - `Ok(Some(false))` → radio is disabled (software kill / airplane mode)
/// - `Ok(None)`        → no Wi-Fi hardware detected (nmcli not found, or no wifi device)
pub fn get_wifi_radio_state() -> Result<Option<bool>, String> {
    // First check if there's any wifi device at all
    let dev_output = match Command::new("nmcli")
        .args(["-t", "-f", "TYPE", "device"])
        .output()
    {
        Err(_) => return Ok(None), // nmcli not available → no wifi
        Ok(o) if !o.status.success() => return Ok(None),
        Ok(o) => o,
    };
    let dev_stdout = String::from_utf8_lossy(&dev_output.stdout);
    let has_wifi = dev_stdout.lines().any(|l| l.trim() == "wifi");
    if !has_wifi {
        return Ok(None);
    }

    // Query radio state
    let output = Command::new("nmcli")
        .args(["radio", "wifi"])
        .output()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    let state = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_lowercase();

    Ok(Some(state == "enabled"))
}

/// Delete the saved NetworkManager profile for `ssid`.
///
/// This removes all stored credentials; the network will require a new
/// passphrase the next time it is connected to.
pub fn forget_network(ssid: &str) -> Result<(), String> {
    let status = Command::new("nmcli")
        .args(["connection", "delete", ssid])
        .status()
        .map_err(|e| format!("Failed to launch nmcli: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "nmcli exited with code {}",
            status.code().unwrap_or(-1)
        ))
    }
}

/// Attempt to read a WIFI: URI from an on-screen QR code by taking a Wayland
/// screenshot (via `grim`) and scanning it with `zbarimg`.
///
/// Returns the raw WIFI: URI string on success.
/// Returns `Err` if either tool is unavailable or no QR code is detected.
pub fn scan_screen_qr() -> Result<String, String> {
    use std::io::Write;

    // Capture screen to a temp file.
    let tmp = std::env::temp_dir().join("smpl-wifi-scan.png");
    let grim_status = Command::new("grim")
        .arg(tmp.to_str().unwrap_or("/tmp/smpl-wifi-scan.png"))
        .status()
        .map_err(|_| "grim is not installed (needed for screenshot QR scan)".to_string())?;

    if !grim_status.success() {
        return Err("grim failed to take a screenshot".to_string());
    }

    // Scan the screenshot for QR codes.
    let mut out = Command::new("zbarimg")
        .args(["--quiet", "--raw", tmp.to_str().unwrap_or("/tmp/smpl-wifi-scan.png")])
        .output()
        .map_err(|_| "zbarimg is not installed (needed to decode QR codes)".to_string())?;

    // Clean up temp file (best-effort).
    let _ = std::fs::remove_file(&tmp);

    let content = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // Zero the raw stdout buffer immediately after extracting the URI string.
    // The URI may contain a password (WIFI:T:WPA;S:…;P:secret;;).
    out.stdout.zeroize();

    if content.is_empty() {
        return Err("No QR code found on screen".to_string());
    }
    // Drop any stderr warnings.
    let _ = std::io::stderr().write_all(&out.stderr);

    Ok(content)
}
