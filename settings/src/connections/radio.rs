//! Reading and switching the Wi-Fi (NetworkManager) and Bluetooth (BlueZ) radios.
//!
//! Every system call goes through the [`Tools`] trait. Production code uses
//! [`SystemTools`]; tests use a mock, so nothing here is ever exercised against a
//! real radio by the test suite.

use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::bluetooth::ctl;

/// How long a single `nmcli` call may take before it is killed.
const NMCLI_TIMEOUT: Duration = Duration::from_secs(8);
/// After a switch command, how often and how far apart the state is read back.
pub(super) const CONFIRM_ATTEMPTS: u32 = 4;
const CONFIRM_PAUSE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radio {
    Wifi,
    Bluetooth,
}

impl Radio {
    /// Switching order: Wi-Fi first, then Bluetooth.
    pub const ALL: [Radio; 2] = [Radio::Wifi, Radio::Bluetooth];

    pub fn name(self) -> &'static str {
        match self {
            Radio::Wifi => "Wi-Fi",
            Radio::Bluetooth => "Bluetooth",
        }
    }
}

/// What a radio looked like the last time it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// Not read yet.
    Unread,
    /// No such hardware, or the service that manages it is not running.
    Missing,
    On,
    Off,
    /// The tool failed or printed something unexpected: the real state is not known.
    Unknown(String),
}

impl Reading {
    pub fn is_on(&self) -> bool {
        matches!(self, Reading::On)
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Reading::Off)
    }

    pub fn is_present(&self) -> bool {
        !matches!(self, Reading::Missing)
    }

    /// `On` or `Off`: the state is known and the radio can be switched.
    pub fn is_known(&self) -> bool {
        self.is_on() || self.is_off()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub wifi: Reading,
    pub bluetooth: Reading,
}

impl Snapshot {
    /// Nothing is known yet; every radio is queried the next time.
    pub fn unread() -> Self {
        Self { wifi: Reading::Unread, bluetooth: Reading::Unread }
    }

    pub fn get(&self, radio: Radio) -> &Reading {
        match radio {
            Radio::Wifi => &self.wifi,
            Radio::Bluetooth => &self.bluetooth,
        }
    }

    pub fn set(&mut self, radio: Radio, reading: Reading) {
        match radio {
            Radio::Wifi => self.wifi = reading,
            Radio::Bluetooth => self.bluetooth = reading,
        }
    }

    /// Radios that exist, in switching order.
    pub fn present(&self) -> Vec<Radio> {
        Radio::ALL
            .into_iter()
            .filter(|r| self.get(*r).is_present())
            .collect()
    }

    /// Airplane mode is derived from the real radios: it is on when at least one
    /// radio exists and every existing radio is off. `None` means a radio that is
    /// not known to be on has not been read (or could not be), so the answer is
    /// not known either.
    pub fn airplane(&self) -> Option<bool> {
        let present = self.present();
        if present.is_empty() {
            return Some(false);
        }
        if present.iter().any(|r| self.get(*r).is_on()) {
            return Some(false);
        }
        if present.iter().any(|r| !self.get(*r).is_off()) {
            return None;
        }
        Some(true)
    }
}

// ── System access seam ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    /// The program is not installed.
    Missing,
    /// It could not be run, timed out, or its output could not be collected.
    Failed(String),
}

/// Everything the radio code needs from the machine.
pub trait Tools {
    /// Run `nmcli <args>` in the C locale with a time limit.
    fn nmcli(&self, args: &[&str]) -> Result<ToolOutput, ToolError>;
    /// Feed `commands` (one per line) to `bluetoothctl` and return what it printed.
    fn bluetoothctl(&self, commands: &[&str]) -> Result<String, ToolError>;
    /// Wait before reading a state back.
    fn pause(&self, duration: Duration);
}

pub struct SystemTools;

impl Tools for SystemTools {
    fn nmcli(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        let mut child = Command::new("nmcli")
            .args(args)
            // nmcli translates its replies ("enabled"), so ask for the C locale.
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    ToolError::Missing
                } else {
                    ToolError::Failed(format!("Failed to launch nmcli: {e}"))
                }
            })?;

        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let out = child
                        .wait_with_output()
                        .map_err(|e| ToolError::Failed(format!("nmcli output failed: {e}")))?;
                    return Ok(ToolOutput {
                        success: status.success(),
                        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                    });
                }
                Ok(None) if started.elapsed() > NMCLI_TIMEOUT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ToolError::Failed("nmcli timed out".to_string()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) => return Err(ToolError::Failed(format!("nmcli wait error: {e}"))),
            }
        }
    }

    fn bluetoothctl(&self, commands: &[&str]) -> Result<String, ToolError> {
        if !on_path("bluetoothctl") {
            return Err(ToolError::Missing);
        }
        ctl::run_ctl(commands).map_err(ToolError::Failed)
    }

    fn pause(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
        .unwrap_or(false)
}

// ── Reading ───────────────────────────────────────────────────────────────────

fn read_wifi(tools: &dyn Tools) -> Reading {
    let devices = match tools.nmcli(&["-t", "-f", "TYPE", "device"]) {
        Err(ToolError::Missing) => return Reading::Missing,
        Err(ToolError::Failed(e)) => return Reading::Unknown(e),
        // NetworkManager is not running: there is nothing to switch.
        Ok(out) if !out.success => return Reading::Missing,
        Ok(out) => out,
    };
    if !devices.stdout.lines().any(|l| l.trim() == "wifi") {
        return Reading::Missing;
    }

    match tools.nmcli(&["radio", "wifi"]) {
        Err(ToolError::Missing) => Reading::Missing,
        Err(ToolError::Failed(e)) => Reading::Unknown(e),
        Ok(out) if !out.success => Reading::Unknown(error_text(&out)),
        Ok(out) => match out.stdout.trim().to_ascii_lowercase().as_str() {
            "enabled" => Reading::On,
            "disabled" => Reading::Off,
            other => Reading::Unknown(format!("Unexpected nmcli output: {other:?}")),
        },
    }
}

fn read_bluetooth(tools: &dyn Tools) -> Reading {
    match tools.bluetoothctl(&["show"]) {
        // bluetoothctl is missing, or bluetoothd does not answer (bluetoothctl then
        // waits for it until it is killed): either way there is no usable adapter.
        Err(_) => Reading::Missing,
        Ok(out) => match ctl::parse_powered(&out) {
            Ok(true) => Reading::On,
            Ok(false) => Reading::Off,
            Err(e) if e == ctl::NO_ADAPTER => Reading::Missing,
            Err(e) => Reading::Unknown(e),
        },
    }
}

pub fn read(tools: &dyn Tools, radio: Radio) -> Reading {
    match radio {
        Radio::Wifi => read_wifi(tools),
        Radio::Bluetooth => read_bluetooth(tools),
    }
}

/// Read both radios. A radio that `last_seen` already reports as missing is not
/// queried again: asking an unreachable bluetoothd can take seconds.
pub fn snapshot(tools: &dyn Tools, last_seen: &Snapshot) -> Snapshot {
    let mut snapshot = Snapshot::unread();
    for radio in Radio::ALL {
        let reading = if *last_seen.get(radio) == Reading::Missing {
            Reading::Missing
        } else {
            read(tools, radio)
        };
        snapshot.set(radio, reading);
    }
    snapshot
}

// ── Switching ─────────────────────────────────────────────────────────────────

/// Switch one radio and confirm by reading its state back.
///
/// The read-back matters: bluetoothctl reports failures such as a rfkill block
/// as text while still exiting normally, and NetworkManager accepts `radio wifi on`
/// even when a hardware switch keeps the radio off.
pub fn set(tools: &dyn Tools, radio: Radio, on: bool) -> Result<(), String> {
    let verb = if on { "on" } else { "off" };
    let name = radio.name();

    match radio {
        Radio::Wifi => match tools.nmcli(&["radio", "wifi", verb]) {
            Ok(out) if out.success => {}
            Ok(out) => return Err(format!("Could not turn {name} {verb}: {}", error_text(&out))),
            Err(ToolError::Missing) => {
                return Err(format!("{name} is not available: nmcli is not installed"))
            }
            Err(ToolError::Failed(e)) => return Err(format!("Could not turn {name} {verb}: {e}")),
        },
        Radio::Bluetooth => {
            let command = format!("power {verb}");
            match tools.bluetoothctl(&[command.as_str()]) {
                Ok(out) => ctl::power_reply(&out)
                    .map_err(|e| format!("Could not turn {name} {verb}: {e}"))?,
                Err(ToolError::Missing) => {
                    return Err(format!("{name} is not available: bluetoothctl is not installed"))
                }
                Err(ToolError::Failed(e)) => {
                    return Err(format!("Could not turn {name} {verb}: {e}"))
                }
            }
        }
    }

    confirm(tools, radio, on)
}

fn confirm(tools: &dyn Tools, radio: Radio, on: bool) -> Result<(), String> {
    let wanted = if on { Reading::On } else { Reading::Off };
    let verb = if on { "on" } else { "off" };
    let mut last = Reading::Missing;
    for attempt in 0..CONFIRM_ATTEMPTS {
        if attempt > 0 {
            tools.pause(CONFIRM_PAUSE);
        }
        last = read(tools, radio);
        if last == wanted {
            return Ok(());
        }
    }
    Err(if last.is_known() {
        format!("{} did not turn {verb}", radio.name())
    } else {
        format!("Could not confirm that {} turned {verb}", radio.name())
    })
}

/// First useful line of a failed nmcli run.
fn error_text(out: &ToolOutput) -> String {
    out.stderr
        .lines()
        .chain(out.stdout.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches("Error: ").to_string())
        .unwrap_or_else(|| "nmcli reported an error".to_string())
}
