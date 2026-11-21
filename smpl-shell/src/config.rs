use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspacePosition {
    Left,
    Center,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceStyle {
    Numbers,
    Squares,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClockFormat {
    Time,
    Dow,
    Date,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum ClockDateFormat {
    #[serde(rename = "M/D")]
    MonthDay,
    #[serde(rename = "D/M")]
    DayMonth,
    #[serde(rename = "ISO")]
    Iso,
    #[serde(rename = "Mon D")]
    MonthNameDay,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Module {
    Start,
    Workspaces,
    WindowTitle,
    Usb,
    Keypad,
    Sync,
    Dictation,
    Keyboard,
    Bluetooth,
    Wifi,
    Network,
    Volume,
    Battery,
    Clock,
    Notifications,
    // TODO(smpl-bar-sni): register a StatusNotifierWatcher, host items, DBusMenu
    // activation/context menus and owner-change recovery before enabling this.
    Sni,
}

impl Module {
    pub const fn is_layout(self) -> bool {
        matches!(self, Self::Start | Self::Workspaces | Self::WindowTitle)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct BarConfigFile {
    pub version: u32,
    pub workspace_count: usize,
    pub workspace_position: WorkspacePosition,
    pub workspace_spacing: usize,
    pub workspace_style: WorkspaceStyle,
    pub height: usize,
    pub preview_offset_y: usize,
    pub clock_format: ClockFormat,
    pub clock_24h: bool,
    pub clock_date_format: ClockDateFormat,
    pub left: Vec<Module>,
    pub right: Vec<Module>,
}

impl Default for BarConfigFile {
    fn default() -> Self {
        Self {
            version: 1,
            workspace_count: 4,
            workspace_position: WorkspacePosition::Left,
            workspace_spacing: 2,
            workspace_style: WorkspaceStyle::Numbers,
            height: 32,
            preview_offset_y: 36,
            clock_format: ClockFormat::Time,
            clock_24h: !locale_is_us(),
            clock_date_format: if locale_is_us() {
                ClockDateFormat::MonthDay
            } else {
                ClockDateFormat::DayMonth
            },
            left: vec![Module::Start, Module::Workspaces, Module::WindowTitle],
            right: vec![
                Module::Usb,
                Module::Keypad,
                Module::Sync,
                Module::Dictation,
                Module::Keyboard,
                Module::Bluetooth,
                Module::Wifi,
                Module::Network,
                Module::Volume,
                Module::Battery,
                Module::Clock,
                Module::Notifications,
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Read { path: PathBuf, message: String },
    Parse { path: PathBuf, message: String },
    Validation(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, message } | Self::Parse { path, message } => {
                write!(formatter, "{}: {message}", path.display())
            }
            Self::Validation(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug)]
pub struct ConfigPaths {
    pub legacy: PathBuf,
    pub native: PathBuf,
}

impl ConfigPaths {
    pub fn from_home(home: Option<&Path>) -> Option<Self> {
        let home = home.filter(|path| path.is_absolute())?;
        Some(Self {
            legacy: home.join(".config/smplos/bar.conf"),
            native: home.join(".config/smplos/bar.json"),
        })
    }

    pub fn from_env() -> Option<Self> {
        Self::from_home(std::env::var_os("HOME").as_deref().map(Path::new))
    }
}

pub fn load(paths: &ConfigPaths) -> Result<BarConfigFile, ConfigError> {
    let mut config = BarConfigFile::default();
    if paths.legacy.exists() {
        merge_legacy(&mut config, &read_config(&paths.legacy)?, &paths.legacy)?;
    }
    if paths.native.exists() {
        let source = read_config(&paths.native)?;
        let native: BarConfigFile =
            serde_json::from_str(&source).map_err(|error| ConfigError::Parse {
                path: paths.native.clone(),
                message: error.to_string(),
            })?;
        config = native;
    }
    validate(&config).map_err(ConfigError::Validation)?;
    Ok(config)
}

fn read_config(path: &Path) -> Result<String, ConfigError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| ConfigError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })?;
    if !metadata.file_type().is_file() {
        return Err(ConfigError::Read {
            path: path.to_owned(),
            message: "configuration must be a regular file".to_owned(),
        });
    }
    std::fs::read_to_string(path).map_err(|error| ConfigError::Read {
        path: path.to_owned(),
        message: error.to_string(),
    })
}

pub fn merge_legacy(
    config: &mut BarConfigFile,
    source: &str,
    path: &Path,
) -> Result<(), ConfigError> {
    let mut seen = HashSet::new();
    for (number, line) in source.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| ConfigError::Parse {
            path: path.to_owned(),
            message: format!("line {} is missing '='", number + 1),
        })?;
        let key = key.trim();
        let value = value.trim();
        if matches!(
            key,
            "ws_count"
                | "ws_position"
                | "ws_spacing"
                | "ws_style"
                | "clock_format"
                | "clock_24h"
                | "clock_date_fmt"
        ) && !seen.insert(key)
        {
            return Err(ConfigError::Parse {
                path: path.to_owned(),
                message: format!("duplicate {key}"),
            });
        }
        match key {
            "ws_count" => config.workspace_count = integer(value, path, number, key)?,
            "ws_position" => {
                config.workspace_position = match value {
                    "left" => WorkspacePosition::Left,
                    "center" => WorkspacePosition::Center,
                    _ => return Err(invalid_legacy(path, number, key, value)),
                }
            }
            "ws_spacing" => config.workspace_spacing = integer(value, path, number, key)?,
            "ws_style" => {
                config.workspace_style = match value {
                    "numbers" => WorkspaceStyle::Numbers,
                    "squares" => WorkspaceStyle::Squares,
                    _ => return Err(invalid_legacy(path, number, key, value)),
                }
            }
            "clock_format" => {
                config.clock_format = match value {
                    "time" => ClockFormat::Time,
                    "dow" => ClockFormat::Dow,
                    "date" => ClockFormat::Date,
                    _ => return Err(invalid_legacy(path, number, key, value)),
                }
            }
            "clock_24h" => {
                config.clock_24h = match value {
                    "true" => true,
                    "false" => false,
                    "auto" => !locale_is_us(),
                    _ => return Err(invalid_legacy(path, number, key, value)),
                }
            }
            "clock_date_fmt" => {
                config.clock_date_format = match value {
                    "M/D" | "auto" if value == "M/D" || locale_is_us() => ClockDateFormat::MonthDay,
                    "D/M" | "auto" if value == "D/M" || !locale_is_us() => {
                        ClockDateFormat::DayMonth
                    }
                    "ISO" => ClockDateFormat::Iso,
                    "Mon D" => ClockDateFormat::MonthNameDay,
                    _ => return Err(invalid_legacy(path, number, key, value)),
                }
            }
            // Clock/taskbar writers and future OS keys may coexist.
            _ => {}
        }
    }
    validate(config).map_err(ConfigError::Validation)
}

fn integer(value: &str, path: &Path, line: usize, key: &str) -> Result<usize, ConfigError> {
    value
        .parse()
        .map_err(|_| invalid_legacy(path, line, key, value))
}

fn invalid_legacy(path: &Path, line: usize, key: &str, value: &str) -> ConfigError {
    ConfigError::Parse {
        path: path.to_owned(),
        message: format!("line {} has invalid {} value {:?}", line + 1, key, value),
    }
}

pub fn validate(config: &BarConfigFile) -> Result<(), String> {
    if config.version != 1 {
        return Err(format!("unsupported bar config version {}", config.version));
    }

    if !(1..=20).contains(&config.workspace_count) {
        return Err("workspace-count must be in 1..=20".to_owned());
    }
    if !(1..=10).contains(&config.workspace_spacing) {
        return Err("workspace-spacing must be in 1..=10".to_owned());
    }
    if !(24..=48).contains(&config.height) {
        return Err("height must be in 24..=48".to_owned());
    }
    if config.preview_offset_y > 200 {
        return Err("preview-offset-y must be in 0..=200".to_owned());
    }
    if config.left.is_empty() && config.right.is_empty() {
        return Err("at least one side must contain a module".to_owned());
    }
    let mut modules = HashSet::new();
    for module in config.left.iter().chain(&config.right) {
        if !modules.insert(module) {
            return Err(format!("module {module:?} appears more than once"));
        }
        if *module == Module::Sni {
            return Err(
                "module sni is acceptance-gated: generic StatusNotifierItem support is not implemented"
                    .to_owned(),
            );
        }
    }
    let left_layout = config
        .left
        .iter()
        .filter(|module| module.is_layout())
        .count();
    let right_layout = config
        .right
        .iter()
        .filter(|module| module.is_layout())
        .count();
    if left_layout + right_layout == 0 {
        return Err("a layout needs start, workspaces or window-title".to_owned());
    }
    Ok(())
}

fn locale_is_us() -> bool {
    ["LC_TIME", "LC_ALL", "LANG"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok().filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "en_US.UTF-8".to_owned())
        .starts_with("en_US")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_cover_the_existing_tray_modules() {
        let config = BarConfigFile::default();
        validate(&config).unwrap();
        assert!(config.right.contains(&Module::Volume));
        assert!(config.right.contains(&Module::Notifications));
        assert!(!config.right.contains(&Module::Sni));
    }

    #[test]
    fn legacy_reader_rejects_duplicate_and_invalid_managed_values() {
        let path = Path::new("/fixture/bar.conf");
        let mut config = BarConfigFile::default();
        assert!(merge_legacy(&mut config, "ws_count=4\nws_count=5\n", path).is_err());
        assert!(merge_legacy(&mut config, "ws_style=unknown\n", path).is_err());
    }

    #[test]
    fn legacy_clock_preferences_are_applied_without_rewriting_unknown_keys() {
        let path = Path::new("/fixture/bar.conf");
        let mut config = BarConfigFile::default();
        merge_legacy(
            &mut config,
            "clock_format=date\nclock_24h=true\nclock_date_fmt=ISO\ncustom=value\n",
            path,
        )
        .unwrap();
        assert_eq!(config.clock_format, ClockFormat::Date);
        assert!(config.clock_24h);
        assert_eq!(config.clock_date_format, ClockDateFormat::Iso);
    }

    #[test]
    fn native_layout_preserves_unknown_keys_as_errors() {
        let invalid = r#"{"version":1,"untyped":"module"}"#;
        assert!(serde_json::from_str::<BarConfigFile>(invalid).is_err());
    }

    #[test]
    fn sni_is_an_explicit_acceptance_gate() {
        let mut config = BarConfigFile::default();
        config.right.push(Module::Sni);
        assert!(validate(&config).unwrap_err().contains("acceptance-gated"));
    }
}
