//! The Mouse tab: pointer speed, scroll direction and what each button does.
//!
//! - `pointer`: speed, acceleration and scroll options. Saved to
//!   `~/.config/hypr/pointer.conf` and applied to the running Hyprland.
//! - `devices`: mouse models as data: their buttons, where those sit on the
//!   picture and which kinds of action each one can take.
//! - `config`: button assignments in `~/.config/smplos/mouse.json`, global
//!   and per app, kept separately for every device.
//! - `store`: the file writes both of them use.
//! - `pointer_state` and `buttons`: what the page's two cards hold and do,
//!   with no Slint in them.
//! - `ui`: copies them into the window and wires its callbacks.
//!
//! Assignments use the editor the Keypad tab uses (`crate::assign`). They
//! are saved but not applied yet: nothing reads the buttons until the
//! capture and remap backend exists.

pub mod buttons;
pub mod config;
pub mod devices;
pub mod pointer;
pub mod pointer_state;
pub mod store;
pub mod ui;

use std::path::PathBuf;

/// `~/.config`. The OS loaders read these files from there, so, like
/// `monitors.conf`, they never follow `XDG_CONFIG_HOME`.
pub fn config_home() -> PathBuf {
    dirs::home_dir().map(|h| h.join(".config")).unwrap_or_else(|| PathBuf::from(".config"))
}

/// Where the Mouse tab keeps its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// Button assignments (`smplos/mouse.json`).
    pub assignments: PathBuf,
    /// Pointer options, read by Hyprland (`hypr/pointer.conf`).
    pub pointer: PathBuf,
}

impl Paths {
    pub fn under(config_home: &std::path::Path) -> Self {
        Self { assignments: config_home.join("smplos/mouse.json"), pointer: config_home.join("hypr/pointer.conf") }
    }

    pub fn standard() -> Self {
        Self::under(&config_home())
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::path::PathBuf;

    /// An empty directory of its own, so tests never touch a real config.
    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("smplos-mouse-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_files_live_under_the_config_home() {
        let paths = Paths::under(Path::new("/home/u/.config"));
        assert_eq!(paths.assignments, Path::new("/home/u/.config/smplos/mouse.json"));
        assert_eq!(paths.pointer, Path::new("/home/u/.config/hypr/pointer.conf"));
    }
}
