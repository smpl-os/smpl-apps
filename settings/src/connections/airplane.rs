//! Airplane mode as a master switch over the Wi-Fi and Bluetooth radios.
//!
//! Turning it on remembers which radios were on and switches them all off;
//! turning it off brings those radios back. The two radio switches keep working
//! on their own, and the airplane switch always shows what the radios really are
//! (see [`Snapshot::airplane`]), so it can never go stale.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::radio::{self, Radio, Reading, Snapshot, Tools};

/// What each radio was doing before airplane mode switched it off.
///
/// A missing entry means "unknown", which is restored as on: airplane mode must
/// never come back with a radio stuck off that nobody asked to be off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Memory {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bluetooth: Option<bool>,
}

impl Memory {
    pub fn of(snapshot: &Snapshot) -> Self {
        let state = |r: &Reading| match r {
            Reading::On => Some(true),
            Reading::Off => Some(false),
            Reading::Unread | Reading::Missing | Reading::Unknown(_) => None,
        };
        Self { wifi: state(&snapshot.wifi), bluetooth: state(&snapshot.bluetooth) }
    }

    pub fn was_on(&self, radio: Radio) -> bool {
        let remembered = match radio {
            Radio::Wifi => self.wifi,
            Radio::Bluetooth => self.bluetooth,
        };
        remembered != Some(false)
    }
}

/// Where [`Memory`] is kept between runs, so that restarting Settings while
/// airplane mode is on does not lose what to restore. `None` keeps nothing.
#[derive(Debug, Clone, Default)]
pub struct MemoryStore {
    path: Option<PathBuf>,
}

impl MemoryStore {
    pub fn system() -> Self {
        match dirs::state_dir() {
            Some(dir) => Self::at(dir.join("smplos").join("settings").join("airplane-mode.json")),
            None => Self::none(),
        }
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: Some(path.into()) }
    }

    pub fn none() -> Self {
        Self { path: None }
    }

    /// An absent or unreadable file is the same as an empty memory.
    pub fn load(&self) -> Memory {
        self.path
            .as_ref()
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, memory: &Memory) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string(memory).map_err(io::Error::other)?;
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }

    pub fn clear(&self) -> io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        match fs::remove_file(path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    fn save_logged(&self, memory: &Memory) {
        if let Err(e) = self.save(memory) {
            eprintln!("[settings] airplane mode memory not saved: {e}");
        }
    }

    fn clear_logged(&self) {
        if let Err(e) = self.clear() {
            eprintln!("[settings] airplane mode memory not cleared: {e}");
        }
    }
}

/// The result of one switching action: the radios as they are afterwards, and
/// what went wrong (one entry per radio that could not be switched).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub snapshot: Snapshot,
    pub errors: Vec<String>,
}

impl Outcome {
    pub fn error_text(&self) -> String {
        self.errors.join("; ")
    }
}

/// Turn airplane mode on: remember the radios that are on, then switch every
/// radio that is on (or whose state cannot be read) off.
///
/// Both radios are always attempted; a failure on one does not undo or skip the other.
pub fn enable(tools: &dyn Tools, store: &MemoryStore, last_seen: &Snapshot) -> Outcome {
    let before = radio::snapshot(tools, last_seen);
    let targets: Vec<Radio> = Radio::ALL
        .into_iter()
        .filter(|r| !before.get(*r).is_off() && before.get(*r).is_present())
        .collect();

    // With nothing on there is nothing to remember, and what an earlier run
    // remembered must survive.
    if Radio::ALL.iter().any(|r| before.get(*r).is_on()) {
        store.save_logged(&Memory::of(&before));
    }

    let mut errors = Vec::new();
    for radio in targets {
        if let Err(e) = radio::set(tools, radio, false) {
            errors.push(e);
        }
    }
    Outcome { snapshot: radio::snapshot(tools, &before), errors }
}

/// Turn airplane mode off: bring back the radios that were on before.
///
/// If nothing was remembered as on, every radio that exists is brought back, so
/// the switch cannot stay on after it is turned off.
pub fn disable(tools: &dyn Tools, store: &MemoryStore, last_seen: &Snapshot) -> Outcome {
    let before = radio::snapshot(tools, last_seen);
    let memory = store.load();
    let present = before.present();

    let mut targets: Vec<Radio> = present.iter().copied().filter(|r| memory.was_on(*r)).collect();
    if targets.is_empty() {
        targets = present;
    }

    let mut errors = Vec::new();
    for radio in targets {
        if before.get(radio).is_on() {
            continue;
        }
        if let Err(e) = radio::set(tools, radio, true) {
            errors.push(e);
        }
    }
    if errors.is_empty() {
        store.clear_logged();
    }
    Outcome { snapshot: radio::snapshot(tools, &before), errors }
}

/// Switch a single radio on its own; airplane mode and its memory are untouched.
pub fn set_radio(tools: &dyn Tools, radio: Radio, on: bool, last_seen: &Snapshot) -> Outcome {
    let errors = radio::set(tools, radio, on).err().into_iter().collect();
    Outcome { snapshot: radio::snapshot(tools, last_seen), errors }
}
