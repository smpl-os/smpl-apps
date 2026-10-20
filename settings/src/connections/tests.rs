//! Tests for the Connections page. Nothing here touches a real radio: every
//! system call goes through `MockTools`, which only records what it is asked.

use std::cell::{Cell, RefCell};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use super::airplane::{self, Memory, MemoryStore, Outcome};
use super::radio::{
    self, Radio, Reading, Snapshot, ToolError, ToolOutput, Tools, CONFIRM_ATTEMPTS,
};
use super::state::{Reads, State};
use super::view::{self, Section, View};
use super::{
    route, section_for_label, BLUETOOTH_LABELS, BLUETOOTH_SECTION, WIFI_LABELS, WIFI_SECTION,
};

const UI: &str = include_str!("../../ui/main.slint");
const MAIN_RS: &str = include_str!("../main.rs");
const INSTANCE_RS: &str = include_str!("../instance.rs");

/// Labels of the page itself: Airplane Mode is above both sections.
const PAGE_LABELS: [&str; 2] = ["Connections", "Airplane Mode"];

// ── Mock system ───────────────────────────────────────────────────────────────

/// What a mocked radio's hardware is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hw {
    On,
    Off,
    /// No adapter.
    Absent,
    /// Wi-Fi: NetworkManager is not running.
    NoManager,
    /// The tool is not installed.
    NoTool,
    /// The tool fails, or (bluetoothctl) hangs until it is killed.
    Broken,
    /// The tool prints something unexpected.
    Garbage,
    /// Off, and refuses to be switched on (rfkill or a hardware switch).
    Blocked,
    /// On, and refuses to be switched off.
    StuckOn,
}

struct Dev {
    hw: Cell<Hw>,
    /// What reads reported before the last switch.
    shown: Cell<Hw>,
    /// Reads that still report `shown` after a switch.
    stale_reads: Cell<u32>,
}

impl Dev {
    fn new(hw: Hw) -> Self {
        Self { hw: Cell::new(hw), shown: Cell::new(hw), stale_reads: Cell::new(0) }
    }

    /// What a read reports right now.
    fn read(&self) -> Hw {
        let left = self.stale_reads.get();
        if left > 0 {
            self.stale_reads.set(left - 1);
            self.shown.get()
        } else {
            self.hw.get()
        }
    }

    /// Only a healthy radio can be switched; the faulty ones stay as they are.
    fn switch(&self, on: bool, lag: u32) {
        let now = self.hw.get();
        if !matches!(now, Hw::On | Hw::Off) {
            return;
        }
        self.shown.set(now);
        self.hw.set(if on { Hw::On } else { Hw::Off });
        self.stale_reads.set(lag);
    }
}

struct MockTools {
    wifi: Dev,
    bluetooth: Dev,
    /// How many reads still show the old state after a switch command.
    lag: Cell<u32>,
    pauses: Cell<u32>,
    log: RefCell<Vec<String>>,
}

impl MockTools {
    fn new(wifi: Hw, bluetooth: Hw) -> Self {
        Self {
            wifi: Dev::new(wifi),
            bluetooth: Dev::new(bluetooth),
            lag: Cell::new(0),
            pauses: Cell::new(0),
            log: RefCell::new(Vec::new()),
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    /// The commands that change a radio, in order.
    fn switches(&self) -> Vec<String> {
        self.log()
            .into_iter()
            .filter(|c| {
                matches!(
                    c.as_str(),
                    "nmcli radio wifi on"
                        | "nmcli radio wifi off"
                        | "bluetoothctl power on"
                        | "bluetoothctl power off"
                )
            })
            .collect()
    }
}

fn reply(success: bool, stdout: &str, stderr: &str) -> Result<ToolOutput, ToolError> {
    Ok(ToolOutput { success, stdout: stdout.to_string(), stderr: stderr.to_string() })
}

impl Tools for MockTools {
    fn nmcli(&self, args: &[&str]) -> Result<ToolOutput, ToolError> {
        self.log.borrow_mut().push(format!("nmcli {}", args.join(" ")));
        match self.wifi.hw.get() {
            Hw::NoTool => return Err(ToolError::Missing),
            Hw::Broken => return Err(ToolError::Failed("nmcli timed out".to_string())),
            Hw::NoManager => return reply(false, "", "Error: NetworkManager is not running."),
            _ => {}
        }
        match args {
            ["-t", "-f", "TYPE", "device"] => {
                let devices = if self.wifi.hw.get() == Hw::Absent {
                    "ethernet\nloopback\n"
                } else {
                    "ethernet\nwifi\nloopback\n"
                };
                reply(true, devices, "")
            }
            ["radio", "wifi"] => match self.wifi.read() {
                Hw::On | Hw::StuckOn | Hw::Absent => reply(true, "enabled\n", ""),
                Hw::Off | Hw::Blocked => reply(true, "disabled\n", ""),
                _ => reply(true, "maybe\n", ""),
            },
            ["radio", "wifi", verb @ ("on" | "off")] => {
                self.wifi.switch(*verb == "on", self.lag.get());
                reply(true, "", "")
            }
            other => panic!("unexpected nmcli call: {other:?}"),
        }
    }

    fn bluetoothctl(&self, commands: &[&str]) -> Result<String, ToolError> {
        self.log
            .borrow_mut()
            .push(format!("bluetoothctl {}", commands.join(" ; ")));
        match self.bluetooth.hw.get() {
            Hw::NoTool => return Err(ToolError::Missing),
            Hw::Broken => return Err(ToolError::Failed("bluetoothctl timed out".to_string())),
            _ => {}
        }
        match commands {
            ["show"] => Ok(match self.bluetooth.read() {
                Hw::Absent => "No default controller available\n".to_string(),
                Hw::Garbage => "Controller 00:1A:7D:DA:71:13 (public)\n\tName: laptop\n".to_string(),
                state => {
                    let powered = if matches!(state, Hw::On | Hw::StuckOn) { "yes" } else { "no" };
                    format!(
                        "Controller 00:1A:7D:DA:71:13 (public)\n\tName: laptop\n\tPowered: {powered}\n\tDiscoverable: no\n"
                    )
                }
            }),
            // One command per line, as bluetoothctl is fed them.
            [line @ ("power on" | "power off")] => {
                let on = *line == "power on";
                let hw = self.bluetooth.hw.get();
                if hw == Hw::Absent {
                    return Ok("No default controller available\n".to_string());
                }
                if hw == Hw::Blocked && on {
                    return Ok("Failed to set power on: org.bluez.Error.Blocked\n".to_string());
                }
                self.bluetooth.switch(on, self.lag.get());
                Ok(format!("Changing power {} succeeded\n", if on { "on" } else { "off" }))
            }
            other => panic!("unexpected bluetoothctl call: {other:?}"),
        }
    }

    fn pause(&self, _: Duration) {
        self.pauses.set(self.pauses.get() + 1);
    }
}

/// A private directory for memory files, removed when the test ends.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "smpl-connections-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Its directory does not exist yet, as on a first run.
    fn memory_file(&self) -> PathBuf {
        self.0.join("state").join("airplane-mode.json")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn snap(wifi: Reading, bluetooth: Reading) -> Snapshot {
    Snapshot { wifi, bluetooth }
}

fn unknown() -> Reading {
    Reading::Unknown("boom".to_string())
}

fn remembered(wifi: bool, bluetooth: bool) -> Memory {
    Memory { wifi: Some(wifi), bluetooth: Some(bluetooth) }
}

fn outcome(snapshot: Snapshot) -> Outcome {
    Outcome { snapshot, errors: Vec::new() }
}

// ── Reading the radios ────────────────────────────────────────────────────────

#[test]
fn wifi_reading_covers_every_hardware_state() {
    let cases = [
        (Hw::On, Reading::On),
        (Hw::Off, Reading::Off),
        (Hw::Blocked, Reading::Off),
        (Hw::StuckOn, Reading::On),
        (Hw::Absent, Reading::Missing),
        (Hw::NoManager, Reading::Missing),
        (Hw::NoTool, Reading::Missing),
    ];
    for (hw, want) in cases {
        let tools = MockTools::new(hw, Hw::On);
        assert_eq!(radio::read(&tools, Radio::Wifi), want, "{hw:?}");
    }
    for hw in [Hw::Broken, Hw::Garbage] {
        let tools = MockTools::new(hw, Hw::On);
        assert!(matches!(radio::read(&tools, Radio::Wifi), Reading::Unknown(_)), "{hw:?}");
    }
}

#[test]
fn bluetooth_reading_covers_every_hardware_state() {
    let cases = [
        (Hw::On, Reading::On),
        (Hw::Off, Reading::Off),
        (Hw::Blocked, Reading::Off),
        (Hw::StuckOn, Reading::On),
        (Hw::Absent, Reading::Missing),
        (Hw::NoTool, Reading::Missing),
        // A bluetoothd that does not answer leaves bluetoothctl waiting until it is killed.
        (Hw::Broken, Reading::Missing),
    ];
    for (hw, want) in cases {
        let tools = MockTools::new(Hw::On, hw);
        assert_eq!(radio::read(&tools, Radio::Bluetooth), want, "{hw:?}");
    }
    let tools = MockTools::new(Hw::On, Hw::Garbage);
    assert!(matches!(radio::read(&tools, Radio::Bluetooth), Reading::Unknown(_)));
}

#[test]
fn reading_never_changes_a_radio() {
    let tools = MockTools::new(Hw::On, Hw::Off);
    let snapshot = radio::snapshot(&tools, &Snapshot::unread());
    assert_eq!(snapshot, snap(Reading::On, Reading::Off));
    assert!(tools.switches().is_empty(), "{:?}", tools.log());
}

#[test]
fn a_snapshot_does_not_ask_again_for_a_missing_radio() {
    let tools = MockTools::new(Hw::On, Hw::Absent);
    let first = radio::snapshot(&tools, &Snapshot::unread());
    assert_eq!(first.bluetooth, Reading::Missing);

    let asked = tools.log().len();
    let second = radio::snapshot(&tools, &first);
    assert_eq!(second, first);
    assert!(
        tools.log()[asked..].iter().all(|c| c.starts_with("nmcli")),
        "{:?}",
        tools.log()
    );
}

// ── Switching one radio ───────────────────────────────────────────────────────

#[test]
fn switching_issues_the_expected_commands_and_reads_back() {
    let tools = MockTools::new(Hw::On, Hw::On);
    radio::set(&tools, Radio::Wifi, false).unwrap();
    assert_eq!(
        tools.log(),
        ["nmcli radio wifi off", "nmcli -t -f TYPE device", "nmcli radio wifi"]
    );

    let tools = MockTools::new(Hw::On, Hw::On);
    radio::set(&tools, Radio::Bluetooth, false).unwrap();
    assert_eq!(tools.log(), ["bluetoothctl power off", "bluetoothctl show"]);
}

#[test]
fn a_blocked_radio_says_why_it_stayed_off() {
    let tools = MockTools::new(Hw::Blocked, Hw::Blocked);

    let bluetooth = radio::set(&tools, Radio::Bluetooth, true).unwrap_err();
    assert!(bluetooth.contains("Bluetooth"), "{bluetooth}");
    assert!(bluetooth.contains("rfkill"), "{bluetooth}");

    // NetworkManager accepts the command even when the radio cannot come on.
    let wifi = radio::set(&tools, Radio::Wifi, true).unwrap_err();
    assert_eq!(wifi, "Wi-Fi did not turn on");
}

#[test]
fn switching_waits_for_a_slow_radio_to_report_the_new_state() {
    let tools = MockTools::new(Hw::On, Hw::On);
    tools.lag.set(2);
    radio::set(&tools, Radio::Wifi, false).unwrap();
    assert_eq!(tools.pauses.get(), 2);

    let tools = MockTools::new(Hw::On, Hw::On);
    tools.lag.set(2);
    radio::set(&tools, Radio::Bluetooth, false).unwrap();
    assert_eq!(tools.pauses.get(), 2);
}

#[test]
fn switching_gives_up_after_a_bounded_number_of_reads() {
    let tools = MockTools::new(Hw::On, Hw::On);
    tools.lag.set(1000);
    let err = radio::set(&tools, Radio::Wifi, false).unwrap_err();
    assert_eq!(err, "Wi-Fi did not turn off");
    assert_eq!(tools.pauses.get(), CONFIRM_ATTEMPTS - 1);
}

#[test]
fn switching_explains_a_tool_that_is_missing_or_failing() {
    let tools = MockTools::new(Hw::NoTool, Hw::NoTool);
    assert_eq!(
        radio::set(&tools, Radio::Wifi, false).unwrap_err(),
        "Wi-Fi is not available: nmcli is not installed"
    );
    assert_eq!(
        radio::set(&tools, Radio::Bluetooth, false).unwrap_err(),
        "Bluetooth is not available: bluetoothctl is not installed"
    );

    let tools = MockTools::new(Hw::Broken, Hw::Broken);
    assert_eq!(
        radio::set(&tools, Radio::Wifi, false).unwrap_err(),
        "Could not turn Wi-Fi off: nmcli timed out"
    );
    assert_eq!(
        radio::set(&tools, Radio::Bluetooth, false).unwrap_err(),
        "Could not turn Bluetooth off: bluetoothctl timed out"
    );

    let tools = MockTools::new(Hw::Absent, Hw::Absent);
    assert_eq!(
        radio::set(&tools, Radio::Bluetooth, true).unwrap_err(),
        "Could not turn Bluetooth on: No Bluetooth adapter found"
    );
}

// ── Airplane mode ─────────────────────────────────────────────────────────────

#[test]
fn airplane_mode_is_derived_from_the_radios() {
    use Reading::{Missing, Off, On, Unread};

    // No radio at all: nothing to switch off.
    assert_eq!(snap(Missing, Missing).airplane(), Some(false));
    // Any radio that is on keeps it off.
    assert_eq!(snap(On, Off).airplane(), Some(false));
    assert_eq!(snap(Off, On).airplane(), Some(false));
    assert_eq!(snap(On, On).airplane(), Some(false));
    assert_eq!(snap(On, Missing).airplane(), Some(false));
    assert_eq!(snap(On, unknown()).airplane(), Some(false));
    // Every radio that exists is off.
    assert_eq!(snap(Off, Off).airplane(), Some(true));
    assert_eq!(snap(Off, Missing).airplane(), Some(true));
    assert_eq!(snap(Missing, Off).airplane(), Some(true));
    // A radio that is not known to be on could not be read: undecided.
    assert_eq!(snap(Off, unknown()).airplane(), None);
    assert_eq!(snap(Off, Unread).airplane(), None);
    assert_eq!(snap(Unread, Unread).airplane(), None);
}

#[test]
fn turning_airplane_mode_on_remembers_and_switches_both_radios_off() {
    let dir = TempDir::new("enable");
    let store = MemoryStore::at(dir.memory_file());
    let tools = MockTools::new(Hw::On, Hw::On);

    let out = airplane::enable(&tools, &store, &Snapshot::unread());

    assert!(out.errors.is_empty(), "{:?}", out.errors);
    assert_eq!(out.snapshot, snap(Reading::Off, Reading::Off));
    assert_eq!(out.snapshot.airplane(), Some(true));
    assert_eq!(tools.switches(), ["nmcli radio wifi off", "bluetoothctl power off"]);
    assert_eq!(store.load(), remembered(true, true));
}

#[test]
fn turning_airplane_mode_off_restores_both_radios() {
    let dir = TempDir::new("disable");
    let store = MemoryStore::at(dir.memory_file());
    let tools = MockTools::new(Hw::On, Hw::On);

    let on = airplane::enable(&tools, &store, &Snapshot::unread());
    let off = airplane::disable(&tools, &store, &on.snapshot);

    assert!(off.errors.is_empty(), "{:?}", off.errors);
    assert_eq!(off.snapshot, snap(Reading::On, Reading::On));
    assert_eq!(off.snapshot.airplane(), Some(false));
    assert_eq!(
        tools.switches(),
        [
            "nmcli radio wifi off",
            "bluetoothctl power off",
            "nmcli radio wifi on",
            "bluetoothctl power on",
        ]
    );
    assert!(!dir.memory_file().exists(), "the memory is spent once it is restored");
}

#[test]
fn a_radio_that_was_off_before_airplane_mode_stays_off_after_it() {
    let dir = TempDir::new("wifi-off");
    let store = MemoryStore::at(dir.memory_file());
    let tools = MockTools::new(Hw::Off, Hw::On);

    let on = airplane::enable(&tools, &store, &Snapshot::unread());
    assert_eq!(tools.switches(), ["bluetoothctl power off"]);
    assert_eq!(on.snapshot, snap(Reading::Off, Reading::Off));
    assert_eq!(store.load(), remembered(false, true));

    let off = airplane::disable(&tools, &store, &on.snapshot);
    assert_eq!(tools.switches(), ["bluetoothctl power off", "bluetoothctl power on"]);
    assert_eq!(off.snapshot, snap(Reading::Off, Reading::On));
    assert_eq!(off.snapshot.airplane(), Some(false));
}

#[test]
fn turning_airplane_mode_off_without_a_memory_brings_every_radio_back() {
    let store = MemoryStore::none();
    let tools = MockTools::new(Hw::Off, Hw::Off);

    let out = airplane::disable(&tools, &store, &Snapshot::unread());

    assert!(out.errors.is_empty());
    assert_eq!(tools.switches(), ["nmcli radio wifi on", "bluetoothctl power on"]);
    assert_eq!(out.snapshot, snap(Reading::On, Reading::On));
}

#[test]
fn a_radio_the_memory_does_not_know_is_restored_as_on() {
    let dir = TempDir::new("partial");
    let store = MemoryStore::at(dir.memory_file());
    store
        .save(&Memory { wifi: None, bluetooth: Some(false) })
        .unwrap();
    let tools = MockTools::new(Hw::Off, Hw::Off);

    let out = airplane::disable(&tools, &store, &Snapshot::unread());

    assert_eq!(tools.switches(), ["nmcli radio wifi on"]);
    assert_eq!(out.snapshot, snap(Reading::On, Reading::Off));
}

#[test]
fn airplane_mode_acts_only_on_radios_that_exist() {
    // Wi-Fi only.
    for no_bluetooth in [Hw::Absent, Hw::NoTool, Hw::Broken] {
        let tools = MockTools::new(Hw::On, no_bluetooth);
        let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
        assert_eq!(tools.switches(), ["nmcli radio wifi off"], "{no_bluetooth:?}");
        assert!(out.errors.is_empty(), "{no_bluetooth:?}: {:?}", out.errors);
        assert_eq!(out.snapshot, snap(Reading::Off, Reading::Missing));
        assert_eq!(out.snapshot.airplane(), Some(true));
    }

    // Bluetooth only.
    for no_wifi in [Hw::Absent, Hw::NoManager, Hw::NoTool] {
        let tools = MockTools::new(no_wifi, Hw::On);
        let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
        assert_eq!(tools.switches(), ["bluetoothctl power off"], "{no_wifi:?}");
        assert!(out.errors.is_empty(), "{no_wifi:?}: {:?}", out.errors);
        assert_eq!(out.snapshot, snap(Reading::Missing, Reading::Off));
    }

    // Neither: nothing to do, and nothing to complain about.
    let tools = MockTools::new(Hw::Absent, Hw::Absent);
    let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
    assert!(tools.switches().is_empty());
    assert!(out.errors.is_empty());
    assert_eq!(out.snapshot.airplane(), Some(false));
}

#[test]
fn one_radio_failing_does_not_stop_the_other() {
    // Wi-Fi refuses to turn off.
    let tools = MockTools::new(Hw::StuckOn, Hw::On);
    let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
    assert_eq!(tools.switches(), ["nmcli radio wifi off", "bluetoothctl power off"]);
    assert_eq!(out.errors, ["Wi-Fi did not turn off"]);
    assert_eq!(out.error_text(), "Wi-Fi did not turn off");
    assert_eq!(out.snapshot, snap(Reading::On, Reading::Off));
    assert_eq!(out.snapshot.airplane(), Some(false), "a radio is still on");

    // Bluetooth refuses to turn off.
    let tools = MockTools::new(Hw::On, Hw::StuckOn);
    let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
    assert_eq!(out.snapshot, snap(Reading::Off, Reading::On));
    assert_eq!(out.errors, ["Bluetooth did not turn off"]);

    // Both fail: both are reported.
    let tools = MockTools::new(Hw::StuckOn, Hw::StuckOn);
    let out = airplane::enable(&tools, &MemoryStore::none(), &Snapshot::unread());
    assert_eq!(
        out.error_text(),
        "Wi-Fi did not turn off; Bluetooth did not turn off"
    );
}

#[test]
fn turning_airplane_mode_off_reports_a_blocked_radio_and_keeps_the_memory() {
    let dir = TempDir::new("blocked");
    let store = MemoryStore::at(dir.memory_file());
    store.save(&remembered(true, true)).unwrap();
    let tools = MockTools::new(Hw::Off, Hw::Blocked);

    let out = airplane::disable(&tools, &store, &Snapshot::unread());

    assert_eq!(tools.switches(), ["nmcli radio wifi on", "bluetoothctl power on"]);
    assert_eq!(out.snapshot, snap(Reading::On, Reading::Off));
    assert_eq!(out.errors.len(), 1);
    assert!(out.errors[0].contains("rfkill"), "{:?}", out.errors);
    assert_eq!(store.load(), remembered(true, true), "kept, so a retry restores the same radios");
}

#[test]
fn a_radio_that_cannot_be_read_is_switched_off_and_reported_if_unconfirmed() {
    let dir = TempDir::new("unreadable");
    let store = MemoryStore::at(dir.memory_file());
    let tools = MockTools::new(Hw::On, Hw::Garbage);

    let out = airplane::enable(&tools, &store, &Snapshot::unread());

    assert_eq!(tools.switches(), ["nmcli radio wifi off", "bluetoothctl power off"]);
    assert_eq!(out.errors, ["Could not confirm that Bluetooth turned off"]);
    // What could not be read is remembered as unknown, which comes back as on.
    assert_eq!(store.load(), Memory { wifi: Some(true), bluetooth: None });
}

#[test]
fn turning_airplane_mode_on_when_everything_is_off_keeps_the_memory() {
    let dir = TempDir::new("already-off");
    let store = MemoryStore::at(dir.memory_file());
    store.save(&remembered(true, false)).unwrap();
    let tools = MockTools::new(Hw::Off, Hw::Off);

    let out = airplane::enable(&tools, &store, &Snapshot::unread());

    assert!(tools.switches().is_empty());
    assert!(out.errors.is_empty());
    assert_eq!(store.load(), remembered(true, false));
}

#[test]
fn a_single_radio_switch_leaves_airplane_mode_and_its_memory_alone() {
    let dir = TempDir::new("single");
    let store = MemoryStore::at(dir.memory_file());
    store.save(&remembered(true, true)).unwrap();
    let tools = MockTools::new(Hw::On, Hw::On);

    let first = airplane::set_radio(&tools, Radio::Wifi, false, &Snapshot::unread());
    assert_eq!(tools.switches(), ["nmcli radio wifi off"]);
    assert_eq!(first.snapshot, snap(Reading::Off, Reading::On));
    assert_eq!(first.snapshot.airplane(), Some(false));

    // Switching the last radio off by hand is airplane mode too: it is derived, not stored.
    let second = airplane::set_radio(&tools, Radio::Bluetooth, false, &first.snapshot);
    assert_eq!(second.snapshot.airplane(), Some(true));
    assert_eq!(store.load(), remembered(true, true));

    // And one radio coming back ends it.
    let third = airplane::set_radio(&tools, Radio::Wifi, true, &second.snapshot);
    assert_eq!(third.snapshot.airplane(), Some(false));
}

#[test]
fn a_radio_that_cannot_be_switched_reports_it_and_shows_the_real_state() {
    let tools = MockTools::new(Hw::Blocked, Hw::On);
    let out = airplane::set_radio(&tools, Radio::Wifi, true, &Snapshot::unread());
    assert_eq!(out.errors, ["Wi-Fi did not turn on"]);
    assert_eq!(out.snapshot, snap(Reading::Off, Reading::On));
}

// ── The memory file ───────────────────────────────────────────────────────────

#[test]
fn the_memory_survives_a_restart() {
    let dir = TempDir::new("restart");
    let memory = Memory { wifi: Some(true), bluetooth: Some(false) };
    MemoryStore::at(dir.memory_file()).save(&memory).unwrap();

    assert_eq!(MemoryStore::at(dir.memory_file()).load(), memory);

    let files: Vec<OsString> = fs::read_dir(dir.memory_file().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files, [OsString::from("airplane-mode.json")], "no temporary file is left");
}

#[test]
fn the_memory_file_format_is_stable() {
    let json = serde_json::to_string(&remembered(true, false)).unwrap();
    assert_eq!(json, r#"{"wifi":true,"bluetooth":false}"#);
    let json = serde_json::to_string(&Memory { wifi: Some(true), bluetooth: None }).unwrap();
    assert_eq!(json, r#"{"wifi":true}"#);
}

#[test]
fn an_unreadable_memory_means_nothing_was_remembered() {
    let dir = TempDir::new("unreadable-memory");
    let store = MemoryStore::at(dir.memory_file());
    assert_eq!(store.load(), Memory::default(), "no file");

    fs::create_dir_all(dir.memory_file().parent().unwrap()).unwrap();
    for text in ["", "not json", "[1, 2]", "{}", r#"{"wifi":"yes"}"#] {
        fs::write(dir.memory_file(), text).unwrap();
        let loaded = store.load();
        assert!(loaded == Memory::default(), "{text:?} loaded as {loaded:?}");
    }
}

#[test]
fn a_store_without_a_path_keeps_nothing() {
    let store = MemoryStore::none();
    store.save(&remembered(true, true)).unwrap();
    assert_eq!(store.load(), Memory::default());
    store.clear().unwrap();
}

#[test]
fn clearing_a_memory_that_does_not_exist_is_fine() {
    let dir = TempDir::new("clear");
    let store = MemoryStore::at(dir.memory_file());
    store.clear().unwrap();
    store.save(&remembered(true, true)).unwrap();
    store.clear().unwrap();
    assert!(!dir.memory_file().exists());
}

#[test]
fn the_memory_records_only_known_states_and_restores_the_rest_as_on() {
    let memory = Memory::of(&snap(Reading::On, unknown()));
    assert_eq!(memory, Memory { wifi: Some(true), bluetooth: None });
    assert!(memory.was_on(Radio::Wifi));
    assert!(memory.was_on(Radio::Bluetooth), "unknown comes back on");

    let memory = Memory::of(&snap(Reading::Off, Reading::Missing));
    assert_eq!(memory, Memory { wifi: Some(false), bluetooth: None });
    assert!(!memory.was_on(Radio::Wifi));

    assert_eq!(Memory::of(&Snapshot::unread()), Memory::default());
}

// ── What the page shows ───────────────────────────────────────────────────────

#[test]
fn a_failed_read_never_replaces_a_state_that_was_seen() {
    let shown = snap(Reading::On, Reading::Off);

    assert_eq!(view::with_reading(&shown, Radio::Wifi, unknown()), shown);
    assert_eq!(view::with_reading(&shown, Radio::Bluetooth, unknown()), shown);

    // Everything else is what the radio now is, including "gone".
    assert_eq!(view::with_reading(&shown, Radio::Wifi, Reading::Off).wifi, Reading::Off);
    assert_eq!(view::with_reading(&shown, Radio::Wifi, Reading::Missing).wifi, Reading::Missing);

    // With nothing better known, a failed read is shown as such.
    let first = view::with_reading(&Snapshot::unread(), Radio::Wifi, unknown());
    assert_eq!(first.wifi, unknown());
    let after_missing = view::with_reading(&snap(Reading::Missing, Reading::On), Radio::Wifi, unknown());
    assert_eq!(after_missing.wifi, unknown());
}

#[test]
fn the_view_follows_the_radios() {
    let v = View::of(&snap(Reading::On, Reading::Off), false);
    assert_eq!(v.wifi, Section::Ready { on: true });
    assert_eq!(v.bluetooth, Section::Ready { on: false });
    assert!(!v.airplane_on);
    assert!(v.airplane_available);
    assert_eq!(v.airplane_detail, "Turns off Wi-Fi and Bluetooth at once.");

    let v = View::of(&snap(Reading::Off, Reading::Off), false);
    assert!(v.airplane_on, "the radios decide, not the earlier value");
    assert_eq!(
        v.airplane_detail,
        "Wi-Fi and Bluetooth are off. Turn off airplane mode to bring them back."
    );

    let v = View::of(&snap(Reading::On, Reading::Off), true);
    assert!(!v.airplane_on, "one radio is on, so airplane mode is not");
}

#[test]
fn the_airplane_switch_keeps_its_last_value_when_the_radios_cannot_decide() {
    for before in [true, false] {
        let v = View::of(&snap(Reading::Off, unknown()), before);
        assert_eq!(v.airplane_on, before);
        let v = View::of(&snap(Reading::Off, Reading::Unread), before);
        assert_eq!(v.airplane_on, before);
    }
}

#[test]
fn sections_without_hardware_explain_why() {
    let v = View::of(&snap(Reading::Missing, Reading::Missing), true);
    match (&v.wifi, &v.bluetooth) {
        (Section::Unavailable { note: wifi }, Section::Unavailable { note: bluetooth }) => {
            assert!(wifi.contains("NetworkManager"), "{wifi}");
            assert!(bluetooth.contains("Bluetooth service"), "{bluetooth}");
        }
        other => panic!("expected both sections to be unavailable: {other:?}"),
    }
    assert!(!v.airplane_available);
    assert!(!v.airplane_on);
    assert_eq!(v.airplane_detail, "No Wi-Fi or Bluetooth radio was found.");

    let v = View::of(&snap(Reading::On, unknown()), false);
    match &v.bluetooth {
        Section::Unavailable { note } => assert!(note.contains("boom"), "{note}"),
        other => panic!("expected an unavailable section: {other:?}"),
    }
    assert!(v.airplane_available, "Wi-Fi can still be switched");
}

#[test]
fn a_section_that_was_not_read_yet_is_left_alone() {
    let v = View::of(&Snapshot::unread(), false);
    assert_eq!(v.wifi, Section::Pending);
    assert_eq!(v.bluetooth, Section::Pending);
    assert!(v.airplane_available);
}

#[test]
fn the_airplane_text_names_the_radios_it_controls() {
    let detail = |wifi, bluetooth| View::of(&snap(wifi, bluetooth), false).airplane_detail;
    assert_eq!(detail(Reading::On, Reading::Missing), "Turns off Wi-Fi.");
    assert_eq!(detail(Reading::Missing, Reading::On), "Turns off Bluetooth.");
    assert_eq!(
        detail(Reading::Off, Reading::Missing),
        "Wi-Fi is off. Turn off airplane mode to bring it back."
    );
    assert_eq!(
        detail(Reading::Missing, Reading::Off),
        "Bluetooth is off. Turn off airplane mode to bring it back."
    );
}

#[test]
fn the_window_starts_with_the_text_the_page_shows_for_two_radios() {
    let v = View::of(&snap(Reading::On, Reading::On), false);
    assert!(UI.contains(&format!("conn-airplane-detail: \"{}\";", v.airplane_detail)));
}

// ── Bookkeeping: which result is still current ────────────────────────────────

#[test]
fn a_radio_is_not_read_twice_at_once() {
    let mut state = State::new();
    assert_eq!(
        state.begin_reads(Reads::All),
        [(Radio::Wifi, 0), (Radio::Bluetooth, 0)]
    );
    assert!(state.begin_reads(Reads::All).is_empty());

    state.finish_read(Radio::Wifi, 0, Reading::On);
    assert_eq!(state.begin_reads(Reads::All), [(Radio::Wifi, 0)]);
}

#[test]
fn polling_skips_missing_radios_but_checking_again_does_not() {
    let mut state = State::new();
    for (radio, epoch) in state.begin_reads(Reads::All) {
        let reading = if radio == Radio::Wifi { Reading::On } else { Reading::Missing };
        state.finish_read(radio, epoch, reading);
    }

    assert_eq!(state.begin_reads(Reads::Present), [(Radio::Wifi, 0)]);
    state.finish_read(Radio::Wifi, 0, Reading::On);

    assert_eq!(
        state.begin_reads(Reads::All),
        [(Radio::Wifi, 0), (Radio::Bluetooth, 0)]
    );
}

#[test]
fn nothing_is_read_or_started_while_a_switch_is_under_way() {
    let mut state = State::new();
    assert!(!state.busy());
    assert_eq!(state.begin_action(), Some(Snapshot::unread()));
    assert!(state.busy());

    assert!(state.begin_reads(Reads::All).is_empty());
    assert_eq!(state.begin_action(), None, "a second click is ignored");

    state.finish_action(&outcome(snap(Reading::Off, Reading::Off)));
    assert!(!state.busy());
    assert_eq!(state.begin_reads(Reads::All).len(), 2);
}

#[test]
fn a_read_that_started_before_a_switch_is_dropped() {
    let mut state = State::new();
    let reads = state.begin_reads(Reads::All);
    let (wifi_epoch, bluetooth_epoch) = (reads[0].1, reads[1].1);

    state.begin_action().unwrap();
    // The read finishes while the switch is still running.
    assert!(state.finish_read(Radio::Wifi, wifi_epoch, Reading::On).is_none());

    state.finish_action(&outcome(snap(Reading::Off, Reading::Off)));
    // ... or after it: it saw the radios before the switch.
    assert!(state.finish_read(Radio::Bluetooth, bluetooth_epoch, Reading::On).is_none());

    // Neither is left "in flight", so the next read can go ahead and is current.
    let next = state.begin_reads(Reads::Present);
    assert_eq!(next.len(), 2);
    let change = state.finish_read(Radio::Wifi, next[0].1, Reading::Off).unwrap();
    assert_eq!(change.previous, snap(Reading::Off, Reading::Off), "the switch's state was kept");
}

#[test]
fn an_action_result_replaces_what_was_shown() {
    let mut state = State::new();
    for (radio, epoch) in state.begin_reads(Reads::All) {
        state.finish_read(radio, epoch, Reading::On);
    }
    assert_eq!(state.begin_action(), Some(snap(Reading::On, Reading::On)));

    let change = state.finish_action(&outcome(snap(Reading::Off, Reading::Missing)));

    assert_eq!(change.previous, snap(Reading::On, Reading::On));
    assert_eq!(change.shown, snap(Reading::Off, Reading::Missing), "not merged with the old state");
    assert_eq!(change.view.wifi, Section::Ready { on: false });
    assert!(matches!(change.view.bluetooth, Section::Unavailable { .. }));
    assert!(change.view.airplane_on);
}

#[test]
fn a_finished_read_reports_what_changed() {
    let mut state = State::new();
    let reads = state.begin_reads(Reads::All);

    let change = state.finish_read(Radio::Wifi, reads[0].1, Reading::Off).unwrap();
    assert_eq!(change.previous, Snapshot::unread());
    assert_eq!(change.shown, snap(Reading::Off, Reading::Unread));
    assert!(!change.view.airplane_on, "Bluetooth is not read yet: undecided, so unchanged");

    let change = state.finish_read(Radio::Bluetooth, reads[1].1, Reading::Off).unwrap();
    assert_eq!(change.previous, snap(Reading::Off, Reading::Unread));
    assert!(change.view.airplane_on);
}

#[test]
fn the_state_keeps_the_airplane_switch_while_a_radio_cannot_be_read() {
    let mut state = State::new();
    let reads = state.begin_reads(Reads::All);
    state.finish_read(Radio::Wifi, reads[0].1, Reading::Off);
    let change = state.finish_read(Radio::Bluetooth, reads[1].1, Reading::Missing).unwrap();
    assert!(change.view.airplane_on);

    let reads = state.begin_reads(Reads::All);
    assert_eq!(reads.len(), 2);
    let change = state
        .finish_read(Radio::Bluetooth, reads[1].1, unknown())
        .unwrap();
    assert_eq!(change.shown.bluetooth, unknown());
    assert!(change.view.airplane_on, "undecided, so the switch keeps its value");
}

// ── Deep links and search ─────────────────────────────────────────────────────

#[test]
fn every_label_opens_its_section() {
    for label in WIFI_LABELS {
        assert_eq!(section_for_label(label), Some(WIFI_SECTION), "{label}");
    }
    for label in BLUETOOTH_LABELS {
        assert_eq!(section_for_label(label), Some(BLUETOOTH_SECTION), "{label}");
    }
    for label in PAGE_LABELS {
        assert_eq!(section_for_label(label), None, "{label}");
    }
    assert_eq!(section_for_label(""), None);
    assert_eq!(section_for_label("Keyboard"), None);
    assert_eq!(section_for_label("headphones"), Some(BLUETOOTH_SECTION), "case does not matter");
}

#[test]
fn the_launcher_deep_links_land_on_the_right_section() {
    // `settings --tab <name> --highlight <label>`: the label decides, whichever tab name was used.
    for tab in [WIFI_SECTION, BLUETOOTH_SECTION] {
        for label in ["WiFi", "Wireless", "Connect Wi-Fi", "Wi-Fi Password", "QR Code Wi-Fi"] {
            assert_eq!(route(tab, label), WIFI_SECTION, "{label} via {tab}");
        }
        for label in [
            "Bluetooth Devices",
            "Pair Bluetooth",
            "Connect Bluetooth",
            "Headphones",
            "Discoverable",
        ] {
            assert_eq!(route(tab, label), BLUETOOTH_SECTION, "{label} via {tab}");
        }
    }
    // Airplane Mode is above both sections: it stays in the one that was asked for.
    assert_eq!(route(WIFI_SECTION, "Airplane Mode"), WIFI_SECTION);
    assert_eq!(route(BLUETOOTH_SECTION, "Airplane Mode"), BLUETOOTH_SECTION);
}

#[test]
fn a_tab_name_without_a_label_opens_its_own_section() {
    assert_eq!(route(WIFI_SECTION, ""), WIFI_SECTION);
    assert_eq!(route(BLUETOOTH_SECTION, ""), BLUETOOTH_SECTION);
    assert_eq!(route(WIFI_SECTION, "Something else"), WIFI_SECTION);
    assert_eq!(route(BLUETOOTH_SECTION, "Something else"), BLUETOOTH_SECTION);

    // `--tab connections`, and the names that were separate tabs before.
    for line in [r#""connections" => 7,"#, r#""wifi" => 7,"#, r#""bluetooth" => 8,"#] {
        assert!(MAIN_RS.contains(line), "{line}");
    }
}

#[test]
fn other_tabs_are_never_redirected() {
    for tab in [0, 1, 2, 3, 4, 5, 6, 9, 10, 11, 12] {
        for label in ["Bluetooth", "Headphones", "Wi-Fi", "Airplane Mode", ""] {
            assert_eq!(route(tab, label), tab, "{label} on tab {tab}");
        }
    }
}

#[test]
fn search_lands_every_connections_label_in_connections() {
    let index = crate::settings_search_index();

    // Wi-Fi and Bluetooth have no tabs of their own any more.
    assert!(index.iter().all(|(_, tab, _)| *tab != "wifi" && *tab != "bluetooth"));

    let rows: Vec<_> = index.iter().filter(|(_, tab, _)| *tab == "connections").collect();
    for label in WIFI_LABELS.iter().chain(&BLUETOOTH_LABELS).chain(&PAGE_LABELS) {
        let row = rows
            .iter()
            .find(|(l, _, _)| l == label)
            .unwrap_or_else(|| panic!("no search row for {label:?}"));
        let section = section_for_label(label).unwrap_or(WIFI_SECTION);
        assert_eq!(row.2, section, "{label}");
    }

    // A new row has to be given a section here too.
    for (label, _, _) in &rows {
        assert!(
            WIFI_LABELS.contains(label)
                || BLUETOOTH_LABELS.contains(label)
                || PAGE_LABELS.contains(label),
            "{label:?} is not a known Connections label"
        );
    }
}

// ── The window and main.rs ────────────────────────────────────────────────────

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text.find(start).unwrap_or_else(|| panic!("missing {start:?}"));
    let rest = &text[from..];
    let to = rest[start.len()..]
        .find(end)
        .unwrap_or_else(|| panic!("missing {end:?} after {start:?}"))
        + start.len();
    &rest[..to]
}

fn wifi_section() -> &'static str {
    between(UI, "if root.active-tab == 7: Rectangle {", "if root.active-tab == 8: Rectangle {")
}

fn bluetooth_section() -> &'static str {
    between(UI, "if root.active-tab == 8: Rectangle {", "if root.active-tab == 9: Rectangle {")
}

#[test]
fn the_sidebar_has_one_connections_entry_instead_of_wifi_and_bluetooth() {
    let sidebar = between(UI, "// ── Left sidebar", "// Sidebar separator");
    assert_eq!(sidebar.matches("label: \"Connections\";").count(), 1);
    assert!(!sidebar.contains("label: \"Wi-Fi\";"));
    assert!(!sidebar.contains("label: \"Bluetooth\";"));
    assert!(
        sidebar.contains("active: root.active-tab == 7 || root.active-tab == 8;"),
        "both sections keep the entry lit"
    );
}

#[test]
fn both_sections_open_with_the_airplane_header() {
    for (name, section, flick) in [
        ("Wi-Fi", wifi_section(), "tab7-flick"),
        ("Bluetooth", bluetooth_section(), "tab8-flick"),
    ] {
        assert_eq!(section.matches("ConnectionsHeader {").count(), 1, "{name}");
        assert!(section.contains(&format!("{flick} := Flickable")), "{name}");
        assert!(
            section.contains("set-airplane(want) => { root.conn-set-airplane(want); }"),
            "{name}"
        );
        let header = section.find("ConnectionsHeader {").unwrap();
        let title = section.find("// ── Header: title + scan button ──").unwrap();
        assert!(header < title, "{name}: the airplane header comes first");
    }
}

#[test]
fn each_section_has_its_own_radio_switch() {
    assert!(wifi_section().contains("toggled(want) => { root.wifi-set-radio(want); }"));
    assert!(bluetooth_section().contains("toggled(want) => { root.bt-set-powered(want); }"));
    assert!(!wifi_section().contains("bt-set-powered"));
    assert!(!bluetooth_section().contains("wifi-set-radio"));
}

#[test]
fn unavailable_sections_offer_a_neutral_card_and_a_way_to_check_again() {
    for (section, note) in [
        (wifi_section(), "root.conn-wifi-note"),
        (bluetooth_section(), "root.conn-bt-note"),
    ] {
        assert!(section.contains(note));
        assert!(section.contains("clicked => { root.conn-refresh(); }"));
    }
    assert!(!UI.contains("No Wi-Fi adapter detected"));
    assert!(!UI.contains("No Bluetooth adapter detected"));
}

#[test]
fn airplane_mode_is_no_longer_a_wifi_only_setting() {
    assert!(!UI.contains("wifi-airplane-mode"));
    assert!(!UI.contains("wifi-set-airplane"));
    assert!(!MAIN_RS.contains("on_wifi_set_airplane"));
    assert_eq!(UI.matches(r#"text: "\u{2708}  Airplane Mode";"#).count(), 1);
}

#[test]
fn main_installs_the_page_and_the_shared_launch_path_applies_the_routing() {
    assert!(MAIN_RS.contains("mod connections;"));
    assert!(MAIN_RS.contains("connections::ui::install(&ui)"));

    // Routing is not done in main.rs: the first launch and a launch handed to the
    // running window both go through `instance::apply`, so they cannot disagree.
    assert!(MAIN_RS.contains("instance::apply(&ui, &launch);"));
    assert!(MAIN_RS.contains("instance::watch(&ui);"));
    assert!(!MAIN_RS.contains("connections::route("));
    assert_eq!(INSTANCE_RS.matches("connections::route(").count(), 1);
    assert!(INSTANCE_RS.contains("if let Some(tab) = request.landing_tab() {"));
    assert!(!INSTANCE_RS.contains("if let Some(tab) = request.tab {"));
}

#[test]
fn every_launch_handed_to_a_running_window_gets_a_full_blink_of_its_own() {
    // The blink used to leak its timer: after the tenth tick it went on clearing
    // the highlight every half second, which cut every later blink in the same
    // window short, and each launch handed to a running window starts one. One
    // shared timer that each blink restarts and the last tick stops cannot do that.
    let start = MAIN_RS.find("fn start_highlight_blink(").expect("the blink exists");
    let rest = &MAIN_RS[start..];
    let body = &rest[..rest.find("\n}\n").expect("the blink ends")];

    assert!(body.contains("BLINK_TIMER.with(|timer| {"));
    assert!(body.contains("BLINK_TIMER.with(|timer| timer.stop());"));
    assert!(!body.contains("mem::forget"));
    assert!(!body.contains("slint::Timer::default()"));
    assert_eq!(MAIN_RS.matches("static BLINK_TIMER: slint::Timer").count(), 1);
}
