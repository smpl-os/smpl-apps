//! Slint wiring for the Connections page. System calls run on worker threads and
//! their results come back through `upgrade_in_event_loop`, so the window only
//! ever shows what the radios reported.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::ComponentHandle;

use super::airplane::{self, MemoryStore, Outcome};
use super::radio::{self, Radio, SystemTools};
use super::state::{Change, Reads, State};
use super::view::Section;
use super::{BLUETOOTH_SECTION, WIFI_SECTION};
use crate::MainWindow;

/// How often the open page re-reads the radios, so a switch flipped elsewhere
/// (a keyboard key, `rfkill`, the other settings window) shows up.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

type Shared = Arc<Mutex<State>>;

#[derive(Debug, Clone, Copy)]
enum Action {
    Airplane(bool),
    Radio(Radio, bool),
}

/// Read the radios on worker threads, one per radio so that a slow Bluetooth
/// service does not hold up the Wi-Fi answer.
fn read(ui: &MainWindow, shared: &Shared, reads: Reads) {
    let jobs = shared.lock().unwrap().begin_reads(reads);
    for (radio, epoch) in jobs {
        let weak = ui.as_weak();
        let shared = shared.clone();
        std::thread::spawn(move || {
            let reading = radio::read(&SystemTools, radio);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let change = shared.lock().unwrap().finish_read(radio, epoch, reading);
                if let Some(change) = change {
                    push(&ui, &change);
                }
            });
        });
    }
}

/// Switch radios on a worker thread. A click while another switch is still under
/// way is ignored; the switches are disabled during that time anyway.
fn act(ui: &MainWindow, shared: &Shared, action: Action) {
    let Some(last_seen) = shared.lock().unwrap().begin_action() else {
        return;
    };
    ui.set_conn_busy(true);
    ui.set_conn_error("".into());

    let weak = ui.as_weak();
    let shared = shared.clone();
    std::thread::spawn(move || {
        let outcome = run(action, &last_seen);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let change = shared.lock().unwrap().finish_action(&outcome);
            push(&ui, &change);
            ui.set_conn_error(outcome.error_text().into());
            ui.set_conn_busy(false);
        });
    });
}

fn run(action: Action, last_seen: &radio::Snapshot) -> Outcome {
    let tools = SystemTools;
    match action {
        Action::Airplane(true) => airplane::enable(&tools, &MemoryStore::system(), last_seen),
        Action::Airplane(false) => airplane::disable(&tools, &MemoryStore::system(), last_seen),
        Action::Radio(radio, on) => airplane::set_radio(&tools, radio, on, last_seen),
    }
}

/// Show a change in the window. Called without the state lock held, because
/// setting properties can run Slint handlers that call back into this module.
fn push(ui: &MainWindow, change: &Change) {
    let view = &change.view;

    match &view.wifi {
        Section::Pending => {}
        Section::Ready { on } => {
            ui.set_wifi_radio_on(*on);
            ui.set_wifi_no_hardware(false);
        }
        Section::Unavailable { note } => {
            ui.set_wifi_radio_on(false);
            ui.set_wifi_no_hardware(true);
            ui.set_conn_wifi_note(note.as_str().into());
        }
    }
    match &view.bluetooth {
        Section::Pending => {}
        Section::Ready { on } => {
            ui.set_bt_powered(*on);
            ui.set_bt_no_hardware(false);
        }
        Section::Unavailable { note } => {
            ui.set_bt_powered(false);
            ui.set_bt_no_hardware(true);
            ui.set_conn_bt_note(note.as_str().into());
        }
    }
    ui.set_conn_airplane_on(view.airplane_on);
    ui.set_conn_airplane_available(view.airplane_available);
    ui.set_conn_airplane_detail(view.airplane_detail.as_str().into());

    follow_radios(ui, change);
}

/// Keep the lists in step with the radios: fill them when a radio comes on and
/// drop what they showed when it goes off, as the per-section switches always did.
fn follow_radios(ui: &MainWindow, change: &Change) {
    let (was, now) = (change.previous.get(Radio::Wifi), change.shown.get(Radio::Wifi));
    if was.is_off() && now.is_on() {
        ui.invoke_wifi_list();
    } else if !was.is_off() && now.is_off() {
        ui.set_wifi_networks(slint::ModelRc::default());
        ui.set_wifi_current_ssid("".into());
        ui.set_wifi_selected_idx(-1);
        ui.set_wifi_qr_visible(false);
    }

    let (was, now) = (
        change.previous.get(Radio::Bluetooth),
        change.shown.get(Radio::Bluetooth),
    );
    if was.is_off() && now.is_on() {
        ui.invoke_bt_list();
    } else if !was.is_off() && now.is_off() {
        ui.set_bt_devices(slint::ModelRc::default());
        ui.set_bt_selected_idx(-1);
    }
}

/// Wire the Connections page. The returned timer polls the radios while the page
/// is open and must be kept alive for as long as the window is.
pub fn install(ui: &MainWindow) -> slint::Timer {
    let shared: Shared = Arc::new(Mutex::new(State::new()));

    {
        let (weak, shared) = (ui.as_weak(), shared.clone());
        ui.on_conn_refresh(move || {
            if let Some(ui) = weak.upgrade() {
                if !shared.lock().unwrap().busy() {
                    ui.set_conn_error("".into());
                }
                read(&ui, &shared, Reads::All);
            }
        });
    }
    {
        let (weak, shared) = (ui.as_weak(), shared.clone());
        ui.on_conn_set_airplane(move |on| {
            if let Some(ui) = weak.upgrade() {
                act(&ui, &shared, Action::Airplane(on));
            }
        });
    }
    {
        let (weak, shared) = (ui.as_weak(), shared.clone());
        ui.on_wifi_set_radio(move |on| {
            if let Some(ui) = weak.upgrade() {
                act(&ui, &shared, Action::Radio(Radio::Wifi, on));
            }
        });
    }
    {
        let (weak, shared) = (ui.as_weak(), shared.clone());
        ui.on_bt_set_powered(move |on| {
            if let Some(ui) = weak.upgrade() {
                act(&ui, &shared, Action::Radio(Radio::Bluetooth, on));
            }
        });
    }

    // Know the radios before the page is first opened.
    read(ui, &shared, Reads::All);

    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, POLL_INTERVAL, move || {
        let Some(ui) = weak.upgrade() else { return };
        let tab = ui.get_active_tab();
        if tab == WIFI_SECTION || tab == BLUETOOTH_SECTION {
            read(&ui, &shared, Reads::Present);
        }
    });
    timer
}
