//! Taskbar preferences. The OS controller alone owns workspace reconciliation.

mod scheduler;
#[cfg(test)]
mod tests;
pub mod ui;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Count,
    Position,
    Spacing,
    Style,
    ClockFormat,
    Clock24h,
    ClockDate,
}

const SETTINGS: [Setting; 7] = [
    Setting::Count,
    Setting::Position,
    Setting::Spacing,
    Setting::Style,
    Setting::ClockFormat,
    Setting::Clock24h,
    Setting::ClockDate,
];

impl Setting {
    fn key(self) -> &'static str {
        match self {
            Self::Count => "ws_count",
            Self::Position => "ws_position",
            Self::Spacing => "ws_spacing",
            Self::Style => "ws_style",
            Self::ClockFormat => "clock_format",
            Self::Clock24h => "clock_24h",
            Self::ClockDate => "clock_date_fmt",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Count => "Workspace count",
            Self::Position => "Workspace position",
            Self::Spacing => "Workspace spacing",
            Self::Style => "Workspace style",
            Self::ClockFormat => "Clock format",
            Self::Clock24h => "Time format",
            Self::ClockDate => "Date format",
        }
    }

    fn choices(self) -> &'static [&'static str] {
        match self {
            Self::Position => &["center", "left"],
            Self::Style => &["numbers", "squares"],
            Self::ClockFormat => &["time", "dow", "date"],
            Self::Clock24h => &["false", "true"],
            Self::ClockDate => &["M/D", "D/M", "ISO", "Mon D"],
            Self::Count | Self::Spacing => &[],
        }
    }

    fn encode(self, value: i32) -> Result<String> {
        if matches!(self, Self::Count | Self::Spacing) {
            if (1..=10).contains(&value) {
                return Ok(value.to_string());
            }
        } else if let Some(choice) = self.choices().get(value as usize) {
            return Ok((*choice).into());
        }
        Err(Error::Invalid(self.key().into()))
    }

    fn decode(self, text: &str) -> Result<i32> {
        if text == "auto" && matches!(self, Self::Clock24h | Self::ClockDate) {
            return Ok(Snapshot::default().values[self as usize]);
        }
        let value = if matches!(self, Self::Count | Self::Spacing) {
            text.parse().ok()
        } else {
            self.choices()
                .iter()
                .position(|v| *v == text)
                .map(|i| i as i32)
        };
        match value {
            Some(value) if self.encode(value).is_ok_and(|encoded| encoded == text) => Ok(value),
            _ => Err(Error::Invalid(self.key().into())),
        }
    }

    fn workspace(self) -> bool {
        matches!(
            self,
            Self::Count | Self::Position | Self::Spacing | Self::Style
        )
    }
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Invalid(String),
    Conflict,
    Command(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(detail) => write!(f, "Unsupported taskbar preference: {detail}"),
            Self::Conflict => write!(f, "Taskbar preferences changed during saving"),
            Self::Command(detail) => write!(f, "{detail}"),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub struct Snapshot {
    values: [i32; 7],
}

impl Default for Snapshot {
    fn default() -> Self {
        let locale = ["LC_TIME", "LC_ALL", "LANG"]
            .into_iter()
            .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
            .unwrap_or_else(|| "en_US.UTF-8".into());
        let international = i32::from(!locale.starts_with("en_US"));
        Self {
            values: [4, 0, 1, 0, 0, international, international],
        }
    }
}

impl Snapshot {
    fn parse(text: &str) -> Result<Self> {
        let mut snapshot = Self::default();
        let mut seen = [false; 7];
        for line in text.lines() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if let Some(setting) = SETTINGS.iter().find(|s| s.key() == key.trim()) {
                let index = *setting as usize;
                if seen[index] {
                    return Err(Error::Invalid(format!("duplicate {}", setting.key())));
                }
                seen[index] = true;
                snapshot.values[index] = setting.decode(value.trim())?;
            }
        }
        Ok(snapshot)
    }
}

pub fn config_path() -> Result<PathBuf> {
    // This path is shared with bar-ctl and the EWW clock scripts.
    dirs::home_dir()
        .map(|home| home.join(".config/smplos/bar.conf"))
        .ok_or_else(|| Error::Invalid("HOME is unavailable".into()))
}

fn read_text(path: &Path) -> Result<Option<String>> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() && meta.nlink() == 1 => Ok(Some(fs::read_to_string(path)?)),
        Ok(_) => Err(Error::Invalid(
            "bar.conf must be a regular, unlinked file".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub fn read(path: &Path) -> Result<Snapshot> {
    Snapshot::parse(read_text(path)?.as_deref().unwrap_or(""))
}

fn edit(text: &str, setting: Setting, value: i32) -> Result<String> {
    Snapshot::parse(text)?;
    let value = setting.encode(value)?;
    let mut found = false;
    let mut result = String::new();
    for line in text.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let replacement = body
            .split_once('=')
            .filter(|(key, _)| key.trim() == setting.key());
        if replacement.is_some() {
            found = true;
            result.push_str(&format!("{}={value}{}", setting.key(), &line[body.len()..]));
        } else {
            result.push_str(line);
        }
    }
    if !found {
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        if !result.is_empty() && !result.ends_with('\n') {
            result.push_str(newline);
        }
        result.push_str(&format!("{}={value}{newline}", setting.key()));
    }
    Snapshot::parse(&result)?;
    Ok(result)
}

struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("[settings] cannot remove taskbar temporary file: {error}");
            }
        }
    }
}

fn replace(path: &Path, original: &Option<String>, text: &str) -> Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("bar.conf path".into()))?;
    fs::create_dir_all(parent)?;
    let temporary_path = parent.join(format!(
        ".bar.conf.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary_path)?;
    let temporary = Temporary(temporary_path);
    if original.is_some() {
        file.set_permissions(fs::metadata(path)?.permissions())?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    if read_text(path)? != *original {
        return Err(Error::Conflict);
    }
    fs::rename(&temporary.0, path)?;
    Ok(())
}

pub trait Runtime {
    fn run(&self, program: &str, args: &[&str]) -> Result<String>;
}

pub struct System;
impl Runtime for System {
    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        let output = Command::new("timeout")
            .args(["--signal=TERM", "--kill-after=1s", "8s", program])
            .args(args)
            .output()?;
        if !output.status.success() {
            return Err(Error::Command(format!(
                "{program} failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(1000)
                    .collect::<String>()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().into())
    }
}

#[derive(Debug)]
pub enum SaveOutcome {
    Updated,
    SavedButNotApplied(Error),
}

pub fn change(
    path: &Path,
    runtime: &impl Runtime,
    setting: Setting,
    value: i32,
) -> Result<SaveOutcome> {
    let original = read_text(path)?;
    let text = edit(original.as_deref().unwrap_or(""), setting, value)?;
    replace(path, &original, &text)?;
    let applied = (|| {
        if setting.workspace() {
            runtime.run("bar-ctl", &["apply"])?;
        } else {
            let config = path
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| Error::Invalid("bar.conf path".into()))?
                .join("eww");
            let top = runtime.run(
                "sh",
                &[&config.join("scripts/clock-top.sh").to_string_lossy()],
            )?;
            let bottom = runtime.run(
                "sh",
                &[&config.join("scripts/clock-bot.sh").to_string_lossy()],
            )?;
            if top.is_empty() {
                return Err(Error::Command("Clock script returned no time".into()));
            }
            runtime.run(
                "eww",
                &[
                    "--config",
                    &config.to_string_lossy(),
                    "update",
                    &format!("clock-top={top}"),
                    &format!("clock-bot={bottom}"),
                ],
            )?;
        }
        if read_text(path)?.as_deref() != Some(text.as_str()) {
            return Err(Error::Conflict);
        }
        Ok(())
    })();
    Ok(match applied {
        Ok(()) => SaveOutcome::Updated,
        Err(error) => SaveOutcome::SavedButNotApplied(error),
    })
}
