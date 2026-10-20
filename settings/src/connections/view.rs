//! What the Connections page shows for a given state of the radios.

use super::radio::{Radio, Reading, Snapshot};

/// Fold a fresh reading of one radio into what is shown.
///
/// A failed read (`Unknown`) never replaces a state that was actually seen, so a
/// hiccup in `nmcli` or `bluetoothctl` does not flip a switch. Everything else,
/// including `Missing`, is what the radio now is.
pub fn with_reading(shown: &Snapshot, radio: Radio, fresh: Reading) -> Snapshot {
    let mut next = shown.clone();
    let keep_shown = matches!(fresh, Reading::Unknown(_)) && shown.get(radio).is_known();
    if !keep_shown {
        next.set(radio, fresh);
    }
    next
}

/// How one section (Wi-Fi or Bluetooth) is shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Section {
    /// Not read yet: leave the section as it is.
    Pending,
    /// The radio exists; `on` is its switch.
    Ready { on: bool },
    /// Missing or unreadable: show the "not available" card with this reason.
    Unavailable { note: String },
}

impl Section {
    fn of(radio: Radio, reading: &Reading) -> Self {
        match reading {
            Reading::Unread => Section::Pending,
            Reading::On => Section::Ready { on: true },
            Reading::Off => Section::Ready { on: false },
            Reading::Missing => Section::Unavailable {
                note: match radio {
                    Radio::Wifi => "No Wi-Fi adapter was found, or NetworkManager is not running.",
                    Radio::Bluetooth => {
                        "No Bluetooth adapter was found, or the Bluetooth service is not running."
                    }
                }
                .to_string(),
            },
            Reading::Unknown(why) => Section::Unavailable {
                note: format!("The {} state could not be read: {why}", radio.name()),
            },
        }
    }
}

/// Everything the page derives from the radios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub wifi: Section,
    pub bluetooth: Section,
    pub airplane_on: bool,
    /// False when there is no radio for the switch to act on.
    pub airplane_available: bool,
    pub airplane_detail: String,
}

impl View {
    /// `airplane_before` is shown again when the radios do not decide the switch
    /// (a radio that is not known to be on could not be read).
    pub fn of(snapshot: &Snapshot, airplane_before: bool) -> Self {
        // Radios the switch acts on: the ones that exist or are still being read.
        let controlled: Vec<Radio> = Radio::ALL
            .into_iter()
            .filter(|r| {
                matches!(snapshot.get(*r), Reading::Unread | Reading::On | Reading::Off)
            })
            .collect();
        let airplane_on = snapshot.airplane().unwrap_or(airplane_before);

        Self {
            wifi: Section::of(Radio::Wifi, snapshot.get(Radio::Wifi)),
            bluetooth: Section::of(Radio::Bluetooth, snapshot.get(Radio::Bluetooth)),
            airplane_on,
            airplane_available: !controlled.is_empty(),
            airplane_detail: airplane_detail(&controlled, airplane_on),
        }
    }
}

fn airplane_detail(controlled: &[Radio], airplane_on: bool) -> String {
    let names = controlled
        .iter()
        .map(|r| r.name())
        .collect::<Vec<_>>()
        .join(" and ");
    match (controlled.len(), airplane_on) {
        (0, _) => "No Wi-Fi or Bluetooth radio was found.".to_string(),
        (1, true) => format!("{names} is off. Turn off airplane mode to bring it back."),
        (_, true) => format!("{names} are off. Turn off airplane mode to bring them back."),
        (1, false) => format!("Turns off {names}."),
        (_, false) => format!("Turns off {names} at once."),
    }
}
