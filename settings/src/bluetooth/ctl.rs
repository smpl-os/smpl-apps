//! BlueZ integration via the `bluetoothctl` command-line tool.
//!
//! Every public function spawns a `bluetoothctl` subprocess, sends commands via
//! stdin, and parses stdout. No global state is mutated.

use std::process::{Command, Stdio};
use std::io::Write;
use std::time::{Duration, Instant};

// ── Data types ────────────────────────────────────────────────────────────────

/// A discovered or paired Bluetooth device.
#[derive(Debug, Clone)]
pub struct BluetoothDevice {
    /// MAC address, e.g. "AA:BB:CC:DD:EE:FF".
    pub address: String,
    /// Human-readable name, e.g. "AirPods Pro".
    pub name: String,
    /// Whether the device is currently connected.
    pub connected: bool,
    /// Whether the device is paired (bonded).
    pub paired: bool,
    /// Whether the device is trusted.
    pub trusted: bool,
    /// Device icon/type string from BlueZ, e.g. "audio-headset", "input-keyboard".
    pub icon: String,
    /// RSSI (signal strength) if available, otherwise 0.
    pub rssi: i32,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Run a `bluetoothctl` command by piping it to stdin and collecting stdout.
/// Returns Ok(stdout) or Err(description).
/// Timeout for any single bluetoothctl invocation (seconds).
const CTL_TIMEOUT: Duration = Duration::from_secs(4);

fn run_ctl(commands: &[&str]) -> Result<String, String> {
    let mut child = Command::new("bluetoothctl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn bluetoothctl: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        for cmd in commands {
            let _ = writeln!(stdin, "{}", cmd);
        }
        let _ = writeln!(stdin, "exit");
    }

    // Poll with timeout to avoid hanging when bluetoothd is not running
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process exited — collect output
                let output = child.wait_with_output()
                    .map_err(|e| format!("bluetoothctl output failed: {}", e))?;
                return Ok(String::from_utf8_lossy(&output.stdout).to_string());
            }
            Ok(None) => {
                // Still running
                if start.elapsed() > CTL_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("bluetoothctl timed out (is bluetoothd running?)".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(format!("bluetoothctl wait error: {}", e));
            }
        }
    }
}

/// Run a single bluetoothctl command.
fn run_single(cmd: &str) -> Result<String, String> {
    run_ctl(&[cmd])
}

/// Strip ANSI escape codes from bluetoothctl output.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until we hit a letter (end of ANSI sequence)
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Query whether the Bluetooth adapter exists and is powered on.
/// Returns `Ok(true)` if powered, `Ok(false)` if powered off,
/// `Err` if no adapter or bluetoothd is not running.
pub fn is_powered() -> Result<bool, String> {
    let out = run_single("show")?;
    let clean = strip_ansi(&out);
    if clean.contains("No default controller") || clean.contains("Waiting to connect") {
        return Err("No Bluetooth adapter found".to_string());
    }
    Ok(clean.lines().any(|l| {
        let trimmed = l.trim();
        trimmed.starts_with("Powered:") && trimmed.ends_with("yes")
    }))
}

/// Get adapter info: name, address, powered, discoverable, pairable.
pub fn get_adapter_info() -> Result<(String, String, bool, bool, bool), String> {
    let out = run_single("show")?;
    let clean = strip_ansi(&out);
    if clean.contains("No default controller") || clean.contains("Waiting to connect") {
        return Err("No Bluetooth adapter found".to_string());
    }

    let mut name = String::new();
    let mut address = String::new();
    let mut powered = false;
    let mut discoverable = false;
    let mut pairable = false;

    for line in clean.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Controller ") {
            // "Controller AA:BB:CC:DD:EE:FF ..."
            if let Some(addr) = rest.split_whitespace().next() {
                address = addr.to_string();
            }
        }
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            name = rest.trim().to_string();
        }
        if trimmed.starts_with("Powered:") {
            powered = trimmed.ends_with("yes");
        }
        if trimmed.starts_with("Discoverable:") {
            discoverable = trimmed.ends_with("yes");
        }
        if trimmed.starts_with("Pairable:") {
            pairable = trimmed.ends_with("yes");
        }
    }

    Ok((name, address, powered, discoverable, pairable))
}

/// Turn Bluetooth adapter on or off.
pub fn set_powered(on: bool) -> Result<(), String> {
    let cmd = if on { "power on" } else { "power off" };
    let out = run_single(cmd)?;
    let clean = strip_ansi(&out);
    if clean.contains("Changing power on succeeded")
        || clean.contains("Changing power off succeeded")
        || clean.contains("already powered")
    {
        Ok(())
    } else if clean.contains("No default controller") {
        Err("No Bluetooth adapter found".to_string())
    } else {
        // Some versions just succeed silently
        Ok(())
    }
}

/// Set discoverable on or off.
pub fn set_discoverable(on: bool) -> Result<(), String> {
    let cmd = if on {
        "discoverable on"
    } else {
        "discoverable off"
    };
    let _ = run_single(cmd)?;
    Ok(())
}

/// Set pairable on or off.
#[allow(dead_code)]
pub fn set_pairable(on: bool) -> Result<(), String> {
    let cmd = if on { "pairable on" } else { "pairable off" };
    let _ = run_single(cmd)?;
    Ok(())
}

/// Trigger a Bluetooth scan for `duration_secs` and return discovered devices.
/// This is a blocking call that takes several seconds.
pub fn scan_devices(duration_secs: u64) -> Result<Vec<BluetoothDevice>, String> {
    // Start scan, wait, stop scan, then list devices
    let _cmds: Vec<String> = vec![
        "scan on".to_string(),
        format!("# sleep {}", duration_secs), // bluetoothctl doesn't have sleep, we use a hack
    ];

    // We need a different approach: start scan, sleep externally, then list
    let _ = run_single("scan on");

    // Sleep to let devices be discovered
    std::thread::sleep(std::time::Duration::from_secs(duration_secs));

    // Stop scanning
    let _ = run_single("scan off");

    // Now list all known devices
    list_devices()
}

/// List all known Bluetooth devices (discovered + paired).
pub fn list_devices() -> Result<Vec<BluetoothDevice>, String> {
    let out = run_single("devices")?;
    let clean = strip_ansi(&out);

    let mut devices: Vec<BluetoothDevice> = Vec::new();

    for line in clean.lines() {
        let trimmed = line.trim();
        // "Device AA:BB:CC:DD:EE:FF DeviceName"
        if let Some(rest) = trimmed.strip_prefix("Device ") {
            let parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if parts.len() >= 2 {
                let addr = parts[0].to_string();
                let name = parts[1].to_string();

                // Get detailed info for this device
                let info = get_device_info(&addr);

                devices.push(BluetoothDevice {
                    address: addr,
                    name: if name.is_empty() || name == "(null)" {
                        info.0.clone()
                    } else {
                        name
                    },
                    connected: info.1,
                    paired: info.2,
                    trusted: info.3,
                    icon: info.4.clone(),
                    rssi: info.5,
                });
            }
        }
    }

    // Sort: connected first, then paired, then by name
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(b.paired.cmp(&a.paired))
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(devices)
}

/// Get detailed info for a single device.
/// Returns (name, connected, paired, trusted, icon, rssi).
fn get_device_info(address: &str) -> (String, bool, bool, bool, String, i32) {
    let cmd = format!("info {}", address);
    let out = match run_single(&cmd) {
        Ok(o) => o,
        Err(_) => return (String::new(), false, false, false, String::new(), 0),
    };
    let clean = strip_ansi(&out);

    let mut name = String::new();
    let mut connected = false;
    let mut paired = false;
    let mut trusted = false;
    let mut icon = String::new();
    let mut rssi: i32 = 0;

    for line in clean.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Name:") {
            name = rest.trim().to_string();
        }
        if trimmed.starts_with("Connected:") {
            connected = trimmed.ends_with("yes");
        }
        if trimmed.starts_with("Paired:") {
            paired = trimmed.ends_with("yes");
        }
        if trimmed.starts_with("Trusted:") {
            trusted = trimmed.ends_with("yes");
        }
        if let Some(rest) = trimmed.strip_prefix("Icon:") {
            icon = rest.trim().to_string();
        }
        if let Some(rest) = trimmed.strip_prefix("RSSI:") {
            // "RSSI: -42" → parse the number (negative dBm, map to 0-100)
            if let Ok(dbm) = rest.trim().parse::<i32>() {
                // Map RSSI from dBm to 0-100 scale
                // Typical range: -100 (weakest) to -30 (strongest)
                rssi = ((dbm + 100) * 100 / 70).clamp(0, 100);
            }
        }
    }

    (name, connected, paired, trusted, icon, rssi)
}

/// Connect to a device by MAC address.
pub fn connect(address: &str) -> Result<(), String> {
    let cmd = format!("connect {}", address);
    let out = run_single(&cmd)?;
    let clean = strip_ansi(&out);

    if clean.contains("Connection successful") || clean.contains("already connected") {
        Ok(())
    } else if clean.contains("Failed to connect") {
        Err(format!(
            "Failed to connect to {}",
            address
        ))
    } else if clean.contains("not available") || clean.contains("Device {} not available") {
        Err(format!("Device {} is not available", address))
    } else {
        // Assume success if no explicit error
        Ok(())
    }
}

/// Disconnect from a device by MAC address.
pub fn disconnect(address: &str) -> Result<(), String> {
    let cmd = format!("disconnect {}", address);
    let out = run_single(&cmd)?;
    let clean = strip_ansi(&out);

    if clean.contains("Successful disconnected") || clean.contains("not connected") {
        Ok(())
    } else if clean.contains("Failed") {
        Err(format!("Failed to disconnect from {}", address))
    } else {
        Ok(())
    }
}

/// Pair with a device. The user may need to confirm on the device.
pub fn pair_device(address: &str) -> Result<(), String> {
    let cmd = format!("pair {}", address);
    let out = run_single(&cmd)?;
    let clean = strip_ansi(&out);

    if clean.contains("Pairing successful") || clean.contains("Already Paired") {
        Ok(())
    } else if clean.contains("Failed to pair") || clean.contains("Authentication Failed") {
        Err(format!("Failed to pair with {}", address))
    } else {
        Ok(())
    }
}

/// Trust a device (so it auto-connects in the future).
pub fn trust_device(address: &str) -> Result<(), String> {
    let cmd = format!("trust {}", address);
    let out = run_single(&cmd)?;
    let clean = strip_ansi(&out);

    if clean.contains("trust succeeded") || clean.contains("already trusted") {
        Ok(())
    } else {
        Err(format!("Failed to trust {}", address))
    }
}

/// Forget (remove) a paired device.
pub fn forget_device(address: &str) -> Result<(), String> {
    let cmd = format!("remove {}", address);
    let out = run_single(&cmd)?;
    let clean = strip_ansi(&out);

    if clean.contains("Device has been removed")
        || clean.contains("not available")
        || clean.contains("removed")
    {
        Ok(())
    } else {
        Err(format!("Failed to remove device {}", address))
    }
}

/// Remove (alias for forget) a device.
#[allow(dead_code)]
pub fn remove_device(address: &str) -> Result<(), String> {
    forget_device(address)
}

/// Map a BlueZ icon string to a unicode emoji for UI display.
pub fn icon_for_device(icon: &str) -> &'static str {
    match icon {
        s if s.contains("audio-headset") || s.contains("audio-headphones") => "\u{1F3A7}", // 🎧
        s if s.contains("audio") || s.contains("speaker") => "\u{1F50A}",                  // 🔊
        s if s.contains("input-keyboard") => "\u{2328}",                                    // ⌨
        s if s.contains("input-mouse") => "\u{1F5B1}",                                      // 🖱
        s if s.contains("input-gaming") || s.contains("input-joystick") => "\u{1F3AE}",    // 🎮
        s if s.contains("input-tablet") => "\u{1F4DD}",                                     // 📝
        s if s.contains("phone") => "\u{1F4F1}",                                            // 📱
        s if s.contains("computer") || s.contains("laptop") => "\u{1F4BB}",                // 💻
        s if s.contains("printer") => "\u{1F5A8}",                                          // 🖨
        s if s.contains("camera") => "\u{1F4F7}",                                           // 📷
        s if s.contains("watch") => "\u{231A}",                                             // ⌚
        s if s.contains("network") => "\u{1F310}",                                          // 🌐
        _ => "\u{1F4E1}",                                                                   // 📡 generic
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_ansi_basic() {
        assert_eq!(strip_ansi("\x1b[0mhello\x1b[1m world"), "hello world");
    }

    #[test]
    fn strip_ansi_empty() {
        assert_eq!(strip_ansi(""), "");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn icon_mapping() {
        assert_eq!(icon_for_device("audio-headset"), "\u{1F3A7}");
        assert_eq!(icon_for_device("input-keyboard"), "\u{2328}");
        assert_eq!(icon_for_device("unknown-thing"), "\u{1F4E1}");
    }

    #[test]
    fn rssi_to_signal() {
        // -30 dBm → should be 100
        let signal = ((-30i32 + 100) * 100 / 70).clamp(0, 100);
        assert_eq!(signal, 100);
        // -100 dBm → should be 0
        let signal = ((-100i32 + 100) * 100 / 70).clamp(0, 100);
        assert_eq!(signal, 0);
        // -65 dBm → should be ~50
        let signal = ((-65i32 + 100) * 100 / 70).clamp(0, 100);
        assert_eq!(signal, 50);
    }
}
