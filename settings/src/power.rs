//! User-owned idle preferences. Editing never regenerates the configuration.

mod scheduler;
mod ui;
pub use ui::install;

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const DISABLED: &str = "# smpl-settings-disabled: ";
const HELPER: &str = "smplos-hypr-dpms";
pub const PRESETS: &[u32] = &[60, 300, 600, 1800, 0, 3600, 7200, 10800, 18000, 28800];
const PROFILES: &[&str] = &["power-saver", "balanced", "performance"];
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Unsupported(String),
    Conflict,
    Command(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Unsupported(e) | Self::Command(e) => f.write_str(e),
            Self::Conflict => {
                f.write_str("Configuration changed during save; refresh and try again")
            }
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

type Result<T> = std::result::Result<T, Error>;

fn unsupported(message: impl Into<String>) -> Error {
    Error::Unsupported(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Lock = 0,
    Dpms = 1,
    Suspend = 2,
    Shutdown = 3,
}

pub fn preset_index(seconds: u32) -> i32 {
    PRESETS
        .iter()
        .position(|&v| v == seconds)
        .map_or(-1, |i| i as i32)
}

pub fn timeout_label(seconds: u32) -> String {
    if seconds == 0 {
        return "Never".into();
    }
    [
        (seconds / 3600, "h"),
        (seconds % 3600 / 60, "min"),
        (seconds % 60, "s"),
    ]
    .into_iter()
    .filter(|(value, _)| *value != 0)
    .map(|(value, unit)| format!("{value} {unit}"))
    .collect::<Vec<_>>()
    .join(" ")
}

#[derive(Debug)]
struct Block {
    start: usize,
    end: usize,
    disabled: bool,
    fields: HashMap<String, usize>,
}

#[derive(Debug)]
pub struct Config {
    lines: Vec<String>,
    general: Option<Block>,
    listeners: [Option<Block>; 4],
    pub seconds: [u32; 4],
}

fn logical(line: &str) -> &str {
    line.strip_prefix(DISABLED).unwrap_or(line)
}

// Hyprlang comments are not shell comments: reject quoted/escaped '#', rather
// than interpreting a command differently from the daemon.
fn code(line: &str) -> Result<&str> {
    let line = logical(line);
    let mut quote = None;
    let mut escaped = false;
    for (offset, c) in line.char_indices() {
        if c == '#' {
            if quote.is_some() || escaped {
                return Err(unsupported(
                    "Quoted or escaped # requires manual configuration",
                ));
            }
            return Ok(line[..offset].trim());
        }
        if !escaped && (c == '\'' || c == '"') {
            if quote == Some(c) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(c);
            }
        }
        escaped = c == '\\' && !escaped;
    }
    Ok(line.trim())
}

fn field(line: &str) -> Result<(&str, &str)> {
    let (key, value) = code(line)?
        .split_once('=')
        .ok_or_else(|| unsupported("Expected a single key = value assignment"))?;
    Ok((key.trim(), value.trim()))
}

fn dpms_command(command: &str, state: &str) -> bool {
    command == format!("{HELPER} {state}")
        || command == format!("hyprctl dispatch dpms {state}")
        || command == format!("hyprctl dispatch \"hl.dsp.dpms({{state='{state}'}})\"")
}

fn classify(command: &str) -> Result<Action> {
    let command = command
        .strip_prefix("systemd-detect-virt -q || ")
        .unwrap_or(command);
    if dpms_command(command, "off") {
        Ok(Action::Dpms)
    } else {
        match command {
            "loginctl lock-session"
            | "hyprlock"
            | "pidof hyprlock || hyprlock"
            | "lock-screen"
            | "pidof hyprlock || lock-screen" => Ok(Action::Lock),
            "systemctl suspend" => Ok(Action::Suspend),
            "systemctl poweroff" => Ok(Action::Shutdown),
            _ => Err(unsupported(format!(
                "Custom listener command is not editable in Settings: {command}"
            ))),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Self> {
        let mut config = Self {
            lines: text.split_inclusive('\n').map(str::to_owned).collect(),
            general: None,
            listeners: std::array::from_fn(|_| None),
            seconds: [0; 4],
        };
        let mut current: Option<(bool, Block)> = None;
        for (i, line) in config.lines.iter().enumerate() {
            let text = code(line)?;
            if text.is_empty() {
                continue;
            }
            let disabled = line.starts_with(DISABLED);
            if let Some((_, block)) = current.as_mut() {
                if disabled != block.disabled {
                    return Err(unsupported("Partly disabled configuration block"));
                }
                if text == "}" {
                    block.end = i;
                    let (is_general, block) = current.take().unwrap();
                    if is_general {
                        if config.general.replace(block).is_some() {
                            return Err(unsupported("Multiple general blocks are ambiguous"));
                        }
                    } else {
                        let timeout = *block
                            .fields
                            .get("timeout")
                            .ok_or_else(|| unsupported("Listener is missing timeout"))?;
                        let value = field(&config.lines[timeout])?.1;
                        let seconds: u32 = value
                            .parse()
                            .map_err(|_| unsupported(format!("Invalid timeout: {value}")))?;
                        if seconds == 0 || seconds > u32::MAX / 1000 {
                            return Err(unsupported("Listener timeout must be positive and fit milliseconds; use Never to disable"));
                        }
                        let command = *block
                            .fields
                            .get("on-timeout")
                            .ok_or_else(|| unsupported("Listener is missing on-timeout"))?;
                        let action = classify(field(&config.lines[command])?.1)? as usize;
                        if config.listeners[action].is_some() {
                            return Err(unsupported(
                                "Multiple listeners for the same action are ambiguous",
                            ));
                        }
                        config.seconds[action] = if block.disabled { 0 } else { seconds };
                        config.listeners[action] = Some(block);
                    }
                } else {
                    let (key, _) = field(line)?;
                    if key == "source" || key.starts_with('$') {
                        return Err(unsupported(
                            "Includes and variables require manual configuration",
                        ));
                    }
                    if key.contains(['{', '}', ' ']) || block.fields.insert(key.into(), i).is_some()
                    {
                        return Err(unsupported(
                            "Nested blocks or duplicate properties are not editable",
                        ));
                    }
                    // Unknown properties are preserved, then checked with the native parser.
                }
            } else {
                let is_general = match text {
                    "general {" if !disabled => true,
                    "listener {" => false,
                    _ => return Err(unsupported(
                        "Unsupported hypridle syntax (including source/variables); edit the file manually"
                    )),
                };
                current = Some((
                    is_general,
                    Block {
                        start: i,
                        end: i,
                        disabled,
                        fields: HashMap::new(),
                    },
                ));
            }
        }
        if current.is_some() {
            return Err(unsupported("Unclosed configuration block"));
        }
        Ok(config)
    }

    fn replace_value(&mut self, line: usize, value: &str) {
        let old = &self.lines[line];
        let equal = old.find('=').expect("parsed assignment");
        let after = &old[equal + 1..];
        let prefix_len = after.len() - after.trim_start_matches([' ', '\t']).len();
        let end = old[equal + 1..]
            .find('#')
            .map(|p| p + equal + 1)
            .unwrap_or(old.trim_end().len());
        let suffix_start = old[..end].trim_end().len().max(equal + 1 + prefix_len);
        self.lines[line] = format!(
            "{}{}{}",
            &old[..equal + 1 + prefix_len],
            value,
            &old[suffix_start..]
        );
    }

    fn sleep_lock(&mut self, enabled: bool) -> Result<()> {
        let Some(general) = &self.general else {
            return Err(unsupported(
                "Lock changes require a general block with manual lock_cmd",
            ));
        };
        if !general.fields.contains_key("lock_cmd") {
            return Err(unsupported(
                "Manual lock_cmd is missing; configure it before changing lock timeout",
            ));
        }
        let before = general.fields.get("before_sleep_cmd").copied();
        let after = general.fields.get("after_sleep_cmd").copied();
        let end = general.end;
        if let Some(line) = before {
            let value = field(&self.lines[line])?.1;
            if !value.is_empty() && value != "loginctl lock-session" {
                return Err(unsupported(
                    "Custom before_sleep_cmd would change lock behavior; edit it manually",
                ));
            }
        }
        if let Some(line) = after {
            let value = field(&self.lines[line])?.1;
            let wake = value
                .strip_prefix("loginctl lock-session; sleep 0.5; ")
                .unwrap_or(value);
            if !wake.is_empty() && !dpms_command(wake, "on") {
                return Err(unsupported(
                    "Custom after_sleep_cmd would change lock behavior; edit it manually",
                ));
            }
            let replacement = if enabled && !wake.is_empty() {
                format!("loginctl lock-session; sleep 0.5; {wake}")
            } else {
                wake.to_owned()
            };
            self.replace_value(line, &replacement);
        }
        if let Some(line) = before {
            self.replace_value(line, if enabled { "loginctl lock-session" } else { "" });
        } else if enabled {
            // Append inside the closing line so existing line indexes stay valid.
            self.lines[end] = format!(
                "    before_sleep_cmd = loginctl lock-session\n{}",
                self.lines[end]
            );
        }
        Ok(())
    }

    pub fn change(mut self, action: Action, seconds: u32, helper: bool) -> Result<String> {
        if seconds > u32::MAX / 1000 {
            return Err(unsupported("Timeout is too large"));
        }
        if action == Action::Lock && (seconds == 0 || self.seconds[0] == 0) {
            self.sleep_lock(seconds != 0)?;
        }
        if let Some(block) = &self.listeners[action as usize] {
            let (start, end, disabled, timeout) = (
                block.start,
                block.end,
                block.disabled,
                block.fields["timeout"],
            );
            if seconds == 0 {
                if !disabled {
                    for line in &mut self.lines[start..=end] {
                        *line = format!("{DISABLED}{line}");
                    }
                }
            } else {
                if disabled {
                    if action == Action::Dpms && !helper {
                        return Err(unsupported("Update smplOS first: smplos-hypr-dpms is required to enable screen-off"));
                    }
                    for line in &mut self.lines[start..=end] {
                        *line = logical(line).to_owned();
                    }
                }
                self.replace_value(timeout, &seconds.to_string());
            }
        } else if seconds != 0 {
            let command =
                match action {
                    Action::Lock => "loginctl lock-session",
                    Action::Dpms if !helper => return Err(unsupported(
                        "Update smplOS first: smplos-hypr-dpms is required to enable screen-off",
                    )),
                    Action::Dpms => "systemd-detect-virt -q || smplos-hypr-dpms off",
                    Action::Suspend => "systemd-detect-virt -q || systemctl suspend",
                    Action::Shutdown => "systemd-detect-virt -q || systemctl poweroff",
                };
            self.lines.push(format!(
                "\nlistener {{\n    timeout = {seconds}\n    on-timeout = {command}\n    ignore_inhibit = true\n}}\n"
            ));
        }
        // Normalize only complete known commands, including reversible disabled blocks.
        if helper {
            for line in &mut self.lines {
                for state in ["on", "off"] {
                    for old in [
                        format!("hyprctl dispatch dpms {state}"),
                        format!("hyprctl dispatch \"hl.dsp.dpms({{state='{state}'}})\""),
                    ] {
                        let Ok((key, value)) = field(line) else {
                            continue;
                        };
                        if !["on-timeout", "on-resume", "after_sleep_cmd"].contains(&key) {
                            continue;
                        }
                        if [
                            old.clone(),
                            format!("systemd-detect-virt -q || {old}"),
                            format!("loginctl lock-session; sleep 0.5; {old}"),
                        ]
                        .contains(&value.to_owned())
                        {
                            *line = line.replacen(&old, &format!("{HELPER} {state}"), 1);
                        }
                    }
                }
            }
        }
        let text = self.lines.concat();
        Self::parse(&text)?;
        Ok(text)
    }
}

fn read(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.nlink() > 1 {
        return Err(unsupported(
            "Symlinked, hardlinked or non-regular hypridle.conf requires manual editing",
        ));
    }
    Ok(fs::read_to_string(path)?)
}

use std::os::unix::fs::MetadataExt;

fn temporary(parent: &Path, name: &str) -> PathBuf {
    parent.join(format!(
        ".{name}.{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ))
}

fn atomic_save(path: &Path, expected: &str, text: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| unsupported("Configuration has no parent directory"))?;
    let temp = temporary(parent, "hypridle-settings");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        file.set_permissions(fs::Permissions::from_mode(
            fs::metadata(path)?.permissions().mode(),
        ))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if read(path)? != expected {
            return Err(Error::Conflict);
        }
        fs::rename(&temp, path)?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|error| {
                unsupported(format!(
                    "File replaced, but directory sync failed; application not attempted: {error}"
                ))
            })?;
        Ok(())
    })();
    if temp.exists() {
        if let Err(error) = fs::remove_file(&temp) {
            eprintln!("[settings] temporary file cleanup failed: {error}");
        }
    }
    result
}

pub struct Output {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

pub trait Runtime {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;
    fn helper_available(&self) -> bool;
    fn validate(&self, text: &str, no_rules: bool) -> Result<()>;
    fn daemon_uses_config(&self, pid: u32, path: &Path) -> Result<()>;
    fn settle(&self);
}

pub struct System;

fn output(command: &mut Command) -> Result<Output> {
    let out = command.stdin(Stdio::null()).output()?;
    Ok(Output {
        success: out.status.success(),
        code: out.status.code(),
        stdout: String::from_utf8_lossy(&out.stdout).trim().into(),
        stderr: String::from_utf8_lossy(&out.stderr).trim().into(),
    })
}

impl Runtime for System {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        output(
            Command::new("timeout")
                .args(["--signal=KILL", "8s", program])
                .args(args),
        )
    }

    fn helper_available(&self) -> bool {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| {
                fs::metadata(dir.join(HELPER))
                    .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            })
        })
    }

    fn validate(&self, text: &str, no_rules: bool) -> Result<()> {
        let dir = temporary(&std::env::temp_dir(), "smpl-power-check");
        fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let result = (|| {
            fs::create_dir(dir.join("hypr"))?;
            fs::write(dir.join("hypr/hypridle.conf"), text)?;
            let mut command = Command::new("timeout");
            command
                .args(["--signal=KILL", "8s", "hypridle"])
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", &dir)
                .env("XDG_CONFIG_HOME", &dir)
                .env("XDG_RUNTIME_DIR", &dir)
                .env("WAYLAND_DISPLAY", "no-compositor")
                .env(
                    "DBUS_SESSION_BUS_ADDRESS",
                    "unix:path=/nonexistent/smpl-power-check",
                );
            use std::os::unix::process::CommandExt;
            // Only the isolated validation child has core dumps disabled.
            unsafe {
                command.pre_exec(|| {
                    let limit = libc::rlimit {
                        rlim_cur: 0,
                        rlim_max: 0,
                    };
                    if libc::setrlimit(libc::RLIMIT_CORE, &limit) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let out = output(&mut command)?;
            if out.code != Some(1) {
                return Err(unsupported(format!(
                    "Hypridle validation did not exit at the isolated connection check: {} {}",
                    out.stdout, out.stderr
                )));
            }
            validate_diagnostics(&format!("{}\n{}", out.stdout, out.stderr), no_rules)
        })();
        if let Err(error) = fs::remove_dir_all(&dir) {
            eprintln!("[settings] validation directory cleanup failed: {error}");
        }
        result
    }

    fn daemon_uses_config(&self, pid: u32, path: &Path) -> Result<()> {
        let args = fs::read(format!("/proc/{pid}/cmdline"))?;
        let args: Vec<_> = args.split(|&b| b == 0).filter(|v| !v.is_empty()).collect();
        let expected = path.as_os_str().as_encoded_bytes();
        if args.is_empty()
            || Path::new(std::ffi::OsStr::from_bytes(args[0])).file_name()
                != Some(std::ffi::OsStr::new("hypridle"))
        {
            return Err(unsupported("Service MainPID is not hypridle"));
        }
        if args.len() == 1
            || (args.len() == 3
                && [b"-c".as_slice(), b"--config"].contains(&args[1])
                && args[2] == expected)
        {
            let env = fs::read(format!("/proc/{pid}/environ"))?;
            for key in [
                "HOME",
                "XDG_CONFIG_HOME",
                "HYPRLAND_INSTANCE_SIGNATURE",
                "WAYLAND_DISPLAY",
            ] {
                let prefix = format!("{key}=");
                let actual = env
                    .split(|&b| b == 0)
                    .find_map(|v| v.strip_prefix(prefix.as_bytes()));
                let ours = std::env::var_os(key);
                if actual != ours.as_ref().map(|v| v.as_encoded_bytes()) {
                    return Err(unsupported(format!(
                        "hypridle has a different {key}; session/configuration is unverified"
                    )));
                }
            }
            Ok(())
        } else {
            Err(unsupported(
                "hypridle uses a different or unsupported command/configuration",
            ))
        }
    }

    fn settle(&self) {
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

use std::os::unix::{ffi::OsStrExt, fs::DirBuilderExt};

fn validate_diagnostics(diagnostics: &str, no_rules: bool) -> Result<()> {
    let mut errors = Vec::new();
    let mut config_errors = false;
    for line in diagnostics.lines() {
        if line.contains("Config has errors") {
            config_errors = true;
            continue;
        }
        if line.contains("Proceeding ignoring faulty entries") {
            config_errors = false;
            continue;
        }
        if line.trim() == "No rules configured" && no_rules {
            continue;
        }
        if (config_errors && !line.trim().is_empty())
            || line.contains("[ERR]")
            || line.contains("[ERROR]")
            || line.contains("Config error")
            || line.contains("No rules configured")
            || line.contains("missing a timeout")
            || line.contains("Error")
            || line.contains("error")
        {
            errors.push(line);
        }
    }
    if !errors.is_empty() || !diagnostics.contains("Couldn't connect to a wayland compositor") {
        return Err(unsupported(format!(
            "Hypridle validation failed: {diagnostics}"
        )));
    }
    Ok(())
}

fn checked(runtime: &impl Runtime, program: &str, args: &[&str]) -> Result<String> {
    let out = runtime.run(program, args)?;
    if !out.success {
        return Err(Error::Command(format!(
            "{program} {} failed: {} {}",
            args.join(" "),
            out.stdout,
            out.stderr
        )));
    }
    Ok(out.stdout)
}

fn profile(runtime: &impl Runtime) -> Result<i32> {
    let actual = checked(runtime, "powerprofilesctl", &["get"])?;
    PROFILES
        .iter()
        .position(|&p| p == actual)
        .map(|i| i as i32)
        .ok_or_else(|| unsupported(format!("Unknown power profile: {actual}")))
}

fn set_profile(runtime: &impl Runtime, index: i32) -> Result<()> {
    let name = PROFILES
        .get(index as usize)
        .ok_or_else(|| unsupported("Invalid power profile"))?;
    checked(runtime, "powerprofilesctl", &["set", name])?;
    if profile(runtime)? != index {
        return Err(Error::Command(
            "Power profile readback did not match the requested profile".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct Service {
    active: bool,
    pid: u32,
}

fn service(runtime: &impl Runtime) -> Result<Service> {
    let text = checked(
        runtime,
        "systemctl",
        &[
            "--user",
            "show",
            "hypridle.service",
            "--property=LoadState,ActiveState,SubState,MainPID",
        ],
    )?;
    let fields: HashMap<_, _> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    if fields.get("LoadState") != Some(&"loaded") {
        return Err(unsupported(
            "hypridle.service is not installed; saved settings are not applied",
        ));
    }
    Ok(Service {
        active: fields.get("ActiveState") == Some(&"active")
            && fields.get("SubState") == Some(&"running"),
        pid: fields
            .get("MainPID")
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| unsupported("Cannot read hypridle service PID"))?,
    })
}

fn restart(runtime: &impl Runtime, path: &Path) -> Result<()> {
    let old = service(runtime)?;
    let processes = runtime.run(
        "pgrep",
        &[
            "-u",
            &unsafe { libc::geteuid() }.to_string(),
            "-x",
            "hypridle",
        ],
    )?;
    if !processes.success && processes.code != Some(1) {
        return Err(Error::Command(format!(
            "Cannot inspect hypridle processes: {}",
            processes.stderr
        )));
    }
    for pid in processes.stdout.split_whitespace() {
        if pid.parse::<u32>().ok() != Some(old.pid) {
            return Err(unsupported(
                "An unmanaged hypridle is running; restart it manually (Settings will not kill it)",
            ));
        }
    }
    if old.pid != 0 {
        runtime.daemon_uses_config(old.pid, path)?;
    }
    checked(
        runtime,
        "systemctl",
        &["--user", "restart", "hypridle.service"],
    )?;
    runtime.settle();
    let new = service(runtime)?;
    if !new.active || new.pid == 0 || new.pid == old.pid {
        return Err(Error::Command(
            "hypridle did not remain active with a new PID; configuration is saved but not applied"
                .into(),
        ));
    }
    runtime.daemon_uses_config(new.pid, path)?;
    runtime.settle();
    let stable = service(runtime)?;
    if !stable.active || stable.pid != new.pid {
        return Err(Error::Command(
            "hypridle exited or restarted again; configuration is saved but not applied".into(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
enum SaveOutcome {
    Restarted,
    ApplicationUnconfirmed(Error),
}

fn save_timer(
    path: &Path,
    runtime: &impl Runtime,
    action: Action,
    seconds: u32,
) -> Result<SaveOutcome> {
    let latest = read(path)?;
    let text = Config::parse(&latest)?.change(action, seconds, runtime.helper_available())?;
    let config = Config::parse(&text)?;
    runtime.validate(&text, config.seconds == [0; 4])?;
    atomic_save(path, &latest, &text)?;
    let apply = (|| {
        if text.lines().any(|line| {
            !line.starts_with(DISABLED)
                && field(line).is_ok_and(|(_, command)| command.contains(HELPER))
        }) {
            checked(runtime, HELPER, &["--check"])?;
        }
        if read(path)? != text {
            return Err(Error::Conflict);
        }
        restart(runtime, path)?;
        if read(path)? != text {
            return Err(Error::Conflict);
        }
        Ok(())
    })();
    Ok(match apply {
        Ok(()) => SaveOutcome::Restarted,
        Err(error) => SaveOutcome::ApplicationUnconfirmed(error),
    })
}

#[cfg(test)]
mod tests;
