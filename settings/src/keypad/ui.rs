//! Slint wiring for the Keypad tab. The tab state lives on the UI thread;
//! sysfs scans, the daemon, keypad-ctl (flashing) and file writes run on
//! worker threads and report back through `invoke_from_event_loop`, after
//! which the whole tab is re-rendered from state. Nothing here runs unless
//! Settings is open on this tab.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::config::{self, ActionKind, Binding, KeypadConfig, Layout, SheetOptions};
use super::{FirmwareImage, InputEvent, SheetPreview, Status, Validation, Variant};
use crate::{KeypadBindingRow, KeypadControl, KeypadSheetCell, KeypadVariant, MainWindow};

const KEYPAD_TAB: i32 = 11;
pub const SCOPE_HELP_URL: &str = "https://github.com/smpl-os/smplos/blob/main/KEYPAD.md#which-keypads-work";
const POLL: Duration = Duration::from_secs(2);
const WIZARD_POLL: Duration = Duration::from_secs(1);
const FLASH_HIGHLIGHT: Duration = Duration::from_millis(350);
// What a daemon without `features` can address (key1..key15, knob1..knob3).
const LEGACY_KEYS: usize = 15;
const LEGACY_KNOBS: usize = 3;

pub const WIZARD_TITLES: [&str; 7] = [
    "Before you start",
    "Choose the firmware",
    "Unplug the keypad",
    "Enter update mode",
    "Flash",
    "Plug it back in",
    "Test every input",
];

#[derive(Default)]
struct Wizard {
    step: usize,
    /// Hardware steps advance by themselves only when entered going forward,
    /// so Back stays on the previous step.
    backed: bool,
    ack: bool,
    images: Vec<FirmwareImage>,
    image: usize,
    flashing: bool,
    flash_ok: bool,
    log: Vec<String>,
    seen: HashSet<String>,
    hint: String,
}

struct State {
    loaded: bool,
    status: Status,
    status_loaded: bool,
    variants: Vec<Variant>,
    /// "Override" was clicked for a keypad that describes its own layout.
    override_open: bool,
    daemon: Option<PathBuf>,
    config: KeypadConfig,
    /// The file exists but could not be read; editing stays off until fixed.
    config_error: Option<String>,
    exists: bool,
    /// The file's text when it was read: a save refuses to overwrite a newer one.
    loaded_text: Option<String>,
    dirty: bool,
    profile: usize,
    control: String,
    knob_event: usize,
    editor: Binding,
    mouse: bool,
    /// The installed keypad app supports the cheatsheet.
    sheet: bool,
    kdenlive_actions: Vec<(String, String)>,
    /// Kdenlive context picked for the preview (index into KDENLIVE_CONTEXTS).
    sheet_context: usize,
    /// The inputs of the last preview request, so unchanged state isn't refetched.
    sheet_key: String,
    sheet_generation: u64,
    sheet_preview: Result<SheetPreview, String>,
    identify: bool,
    /// Identify mode as last sent to the daemon.
    identify_sent: bool,
    identify_holder: Option<super::Identify>,
    max_keys: usize,
    max_knobs: usize,
    live_started: bool,
    active: Option<String>,
    adding: bool,
    app_classes: Vec<String>,
    wizard: Option<Wizard>,
    message: String,
    message_is_error: bool,
    saving: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>())))
}

// ── Derived values ───────────────────────────────────────────────────────────

impl State {
    fn variant(&self, id: &str) -> Option<&Variant> {
        self.variants.iter().find(|v| v.id == id)
    }

    /// The layout the keypad describes itself (our firmware), if any.
    fn detected(&self) -> Option<(String, usize, usize, usize)> {
        let pad = self.status.pads.first()?;
        if let Some(v) = self.variant(&pad.board) {
            return Some((v.name.clone(), v.keys, v.knobs, v.columns));
        }
        if let Some((keys, knobs)) = pad.described {
            return Some((format!("{keys} keys, {knobs} knobs"), keys, knobs, super::default_columns(keys)));
        }
        let l = self.status.layout.as_ref().filter(|l| l.source == "firmware")?;
        Some((l.name.clone(), l.keys, l.knobs, l.columns))
    }

    /// What "Automatic" resolves to: the keypad's own description, the
    /// running app's layout, or the default board.
    fn automatic(&self) -> (String, usize, usize, usize) {
        if let Some(d) = self.detected() {
            return d;
        }
        if let Some(l) = self.status.layout.as_ref().filter(|l| l.source != "config" && l.keys + l.knobs > 0) {
            return (l.name.clone(), l.keys, l.knobs, l.columns);
        }
        let v = self.variants.first().cloned().unwrap_or_else(|| super::builtin_variants().remove(0));
        (v.name, v.keys, v.knobs, v.columns)
    }

    /// Keys, knobs and key columns shown for mapping.
    fn layout(&self) -> (usize, usize, usize) {
        match self.config.layout() {
            Layout::Board(id) => match self.variant(&id) {
                Some(v) => (v.keys, v.knobs, v.columns),
                None => {
                    let (_, k, n, c) = self.automatic();
                    (k, n, c)
                }
            },
            Layout::Custom { keys, knobs, columns } => (keys, knobs, columns.max(1)),
            Layout::Auto => {
                let (_, k, n, c) = self.automatic();
                (k, n, c)
            }
        }
    }

    /// Variant menu: Automatic, the board profiles, an unknown configured id, Custom.
    fn variant_menu(&self) -> (Vec<KeypadVariant>, i32) {
        let (auto_name, k, n, c) = self.automatic();
        let auto_label = if self.detected().is_some() {
            format!("Use the detected layout ({auto_name})")
        } else {
            format!("Automatic ({auto_name})")
        };
        let entry = |name: String, keys: usize, knobs: usize, cols: usize| KeypadVariant {
            name: s(name),
            keys: keys as i32,
            knobs: knobs as i32,
            cols: cols as i32,
        };
        let mut menu = vec![entry(auto_label, k, n, c)];
        menu.extend(self.variants.iter().map(|v| entry(v.name.clone(), v.keys, v.knobs, v.columns)));
        let layout = self.config.layout();
        let mut index = 0;
        if let Layout::Board(id) = &layout {
            index = match self.variants.iter().position(|v| v.id == *id) {
                Some(i) => i + 1,
                None => {
                    let (_, k, n, c) = self.automatic();
                    menu.push(entry(format!("{id} (unknown to this version)"), k, n, c));
                    menu.len() - 1
                }
            };
        }
        let (ck, cn, cc) = match layout {
            Layout::Custom { keys, knobs, columns } => (keys, knobs, columns),
            _ => self.layout(),
        };
        menu.push(entry("Custom…".into(), ck, cn, cc));
        if matches!(self.config.layout(), Layout::Custom { .. }) {
            index = menu.len() - 1;
        }
        (menu, index as i32)
    }

    fn profiles(&self) -> Vec<config::ProfileInfo> {
        self.config.profiles()
    }

    fn slot(&self) -> String {
        if self.control.starts_with("knob") {
            format!("{}.{}", self.control, config::KNOB_EVENTS[self.knob_event.min(2)].0)
        } else {
            self.control.clone()
        }
    }

    fn global(&self) -> bool {
        self.profiles().get(self.profile).is_none_or(|p| p.global)
    }

    fn kinds(&self) -> Vec<ActionKind> {
        let kdenlive = self.profiles().get(self.profile).is_some_and(|p| p.kdenlive);
        let mut kinds = vec![
            ActionKind::Inherit,
            ActionKind::Disabled,
            ActionKind::Shortcut,
            ActionKind::Media,
        ];
        if self.mouse || self.editor.kind == ActionKind::Mouse {
            kinds.push(ActionKind::Mouse);
        }
        kinds.push(ActionKind::Command);
        if kdenlive || self.editor.kind == ActionKind::Kdenlive {
            kinds.push(ActionKind::Kdenlive);
        }
        if (self.sheet && config::sheet_slot(&self.slot())) || self.editor.kind == ActionKind::Cheatsheet {
            kinds.push(ActionKind::Cheatsheet);
        }
        if self.editor.kind == ActionKind::Advanced {
            kinds.push(ActionKind::Advanced);
        }
        kinds
    }

    /// The window class and Kdenlive context the cheatsheet preview shows.
    fn sheet_target(&self) -> (String, String) {
        let profile = self.profiles().get(self.profile).cloned();
        let class = match &profile {
            Some(p) if !p.global => config::example_class(&p.class),
            _ => "smplos-cheatsheet-preview".to_string(),
        };
        let context = if profile.is_some_and(|p| p.kdenlive) {
            super::KDENLIVE_CONTEXTS[self.sheet_context.min(super::KDENLIVE_CONTEXTS.len() - 1)].1.to_string()
        } else {
            String::new()
        };
        (class, context)
    }

    fn choices(&self) -> Vec<(String, String)> {
        let table = |t: &[(&str, &str)]| t.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect::<Vec<_>>();
        let mut list = match self.editor.kind {
            ActionKind::Media => table(config::MEDIA_KEYS),
            ActionKind::Mouse => table(config::MOUSE_BUTTONS),
            ActionKind::Cheatsheet => table(config::CHEATSHEET_MODES),
            ActionKind::Kdenlive if !self.kdenlive_actions.is_empty() => self.kdenlive_actions.clone(),
            ActionKind::Kdenlive => table(config::KDENLIVE_ACTIONS),
            _ => Vec::new(),
        };
        if !self.editor.value.is_empty() && !list.is_empty() && !list.iter().any(|(k, _)| *k == self.editor.value) {
            list.push((self.editor.value.clone(), self.editor.value.clone()));
        }
        list
    }

    fn load_editor(&mut self) {
        self.editor = self.config.binding(self.profile, &self.slot());
    }

    fn summary(&self, control: &str) -> String {
        if control.starts_with("knob") {
            let turn = self.config.binding(self.profile, &format!("{control}.turn"));
            if turn.kind != ActionKind::Inherit {
                return "turn".into();
            }
            let left = self.config.binding(self.profile, &format!("{control}.ccw")).summary();
            let right = self.config.binding(self.profile, &format!("{control}.cw")).summary();
            return match (left.is_empty(), right.is_empty()) {
                (true, true) => String::new(),
                _ => format!("{left} | {right}"),
            };
        }
        self.config.binding(self.profile, control).summary()
    }

    fn set_message(&mut self, text: impl Into<String>, error: bool) {
        self.message = text.into();
        self.message_is_error = error;
    }

    fn verify_total(&self) -> usize {
        let (keys, knobs, _) = self.layout();
        keys + knobs * 3
    }
}

fn knob_event_index(event: &str) -> Option<usize> {
    config::KNOB_EVENTS.iter().position(|(e, _)| *e == event)
}

// ── Rendering ────────────────────────────────────────────────────────────────

fn render(ui: &MainWindow) {
    STATE.with(|cell| {
        if let Some(st) = cell.borrow().as_ref() {
            render_state(ui, st);
        }
    });
    refresh_sheet_preview(ui);
}

/// Auto-hide choices (milliseconds, label).
const SHEET_HIDE: [(u32, &str); 6] = [
    (0, "Until hidden"),
    (3000, "After 3 s without keypad input"),
    (5000, "After 5 s without keypad input"),
    (10000, "After 10 s without keypad input"),
    (30000, "After 30 s without keypad input"),
    (60000, "After 1 min without keypad input"),
];

fn render_sheet_preview(ui: &MainWindow, st: &State) {
    let (title, notice, cells, cols, rows) = match &st.sheet_preview {
        Err(e) => (String::new(), e.clone(), Vec::new(), 1, 1),
        Ok(p) => {
            let mut cells: Vec<KeypadSheetCell> = p
                .keys
                .iter()
                .map(|k| {
                    let e = k.entries.first().cloned().unwrap_or_default();
                    let text = if !e.bound {
                        String::new()
                    } else if e.state.is_empty() {
                        e.label.clone()
                    } else {
                        format!("{} ({})", e.label, e.state)
                    };
                    KeypadSheetCell {
                        num: s(k.control.trim_start_matches("key")),
                        text: s(text),
                        knob: false,
                        col: k.column as i32,
                        row: k.row as i32,
                        bound: e.bound,
                        active: e.active,
                    }
                })
                .collect();
            let key_cols = p.keys.iter().map(|k| k.column + 1).max().unwrap_or(0);
            for k in &p.knobs {
                let line = |glyph: &str, e: &super::SheetEntry| {
                    let label = if e.bound { e.label.as_str() } else { "·" };
                    let state = if e.state.is_empty() { String::new() } else { format!(" ({})", e.state) };
                    let full = format!("{glyph} {label}{state}");
                    // One line per direction: the cell has room for three.
                    if full.chars().count() > 22 {
                        format!("{}…", full.chars().take(21).collect::<String>())
                    } else {
                        full
                    }
                };
                let lines: Vec<String> = ["<", "o", ">"].iter().zip(&k.entries).map(|(g, e)| line(g, e)).collect();
                cells.push(KeypadSheetCell {
                    num: s(format!("Knob {}", k.control.trim_start_matches("knob"))),
                    text: s(lines.join("\n")),
                    knob: true,
                    col: key_cols as i32,
                    row: k.row as i32,
                    bound: k.entries.iter().any(|e| e.bound),
                    active: k.entries.iter().any(|e| e.active),
                });
            }
            let rows = cells.iter().map(|c| c.row + 1).max().unwrap_or(1);
            let mut title = p.title.clone();
            for layer in &p.layers {
                if !title.contains(layer.as_str()) {
                    title = format!("{title} · {layer}");
                }
            }
            (title, p.notice.clone(), cells, key_cols.max(1) as i32, rows)
        }
    };
    ui.set_kp_sheet_title(s(title));
    ui.set_kp_sheet_notice(s(notice));
    ui.set_kp_sheet_cells(ModelRc::from(Rc::new(VecModel::from(cells))));
    ui.set_kp_sheet_cols(cols);
    ui.set_kp_sheet_rows(rows);
}

/// Re-fetches the preview when the config, profile or context changed.
fn refresh_sheet_preview(ui: &MainWindow) {
    let request = with(|st| {
        if !st.loaded || !st.sheet || st.wizard.is_some() {
            return None;
        }
        let (class, context) = st.sheet_target();
        let text = st.config.render();
        let key = format!("{class}\n{context}\n{text}");
        if key == st.sheet_key {
            return None;
        }
        st.sheet_key = key;
        st.sheet_generation += 1;
        Some((st.sheet_generation, st.daemon.clone(), text, class, context))
    })
    .flatten();
    let Some((generation, daemon, text, class, context)) = request else { return };
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let scratch = super::config_path().parent().map(std::path::Path::to_path_buf).unwrap_or_default();
        let preview = super::sheet_preview(daemon.as_deref(), &scratch, &text, &class, &context);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let current = with(|st| {
                if st.sheet_generation != generation {
                    return false;
                }
                st.sheet_preview = preview;
                true
            })
            .unwrap_or(false);
            if current {
                STATE.with(|cell| {
                    if let Some(st) = cell.borrow().as_ref() {
                        render_sheet_preview(&ui, st);
                    }
                });
            }
        });
    });
}

fn set_sheet_option(ui: &MainWindow, change: impl FnOnce(&mut SheetOptions)) {
    with(|st| {
        if st.config_error.is_some() {
            return;
        }
        let mut o = st.config.sheet_options();
        change(&mut o);
        match st.config.set_sheet_options(&o) {
            Ok(()) => {
                st.dirty = true;
                st.set_message("Cheatsheet options changed. Not saved yet.", false);
            }
            Err(e) => st.set_message(e, true),
        }
    });
    render(ui);
}

fn render_state(ui: &MainWindow, st: &State) {
    let (keys, knobs, cols) = st.layout();
    let pad = st.status.pads.first();

    // Device card: what is plugged in (sysfs) and whether the keypad app runs.
    ui.set_kp_connected(pad.is_some());
    ui.set_kp_bootloader(st.status.bootloaders > 0);
    let title = if !st.status_loaded {
        "Looking for keypads…".to_string()
    } else if let Some(pad) = pad {
        let more = if st.status.pads.len() > 1 {
            format!(" (+{} more; the keypad app drives one)", st.status.pads.len() - 1)
        } else {
            String::new()
        };
        let version = if pad.version.is_empty() { String::new() } else { format!(" {}", pad.version) };
        format!("CH552 keypad connected{more}: {}{version}", pad.firmware_label)
    } else if st.status.bootloaders > 0 {
        "A keypad is in firmware update mode (USB 4348:55e0)".into()
    } else {
        "No CH552 keypad connected. You can still prepare mappings below.".into()
    };
    ui.set_kp_device_title(s(title));
    ui.set_kp_device_detail(s(pad.map_or(String::new(), |p| {
        let name = format!("{} {}", p.manufacturer, p.product).trim().to_string();
        let serial = if p.serial.is_empty() { String::new() } else { format!(" · serial {}", p.serial) };
        format!("{name}{serial} · USB 1189:8890")
    })));
    ui.set_kp_firmware(s(pad.map_or("", |p| p.firmware.as_str())));
    ui.set_kp_firmware_text(s(match pad.map(|p| p.firmware.as_str()) {
        Some("stock") => "Stock firmware. The keypad app remaps its keys, but the stock firmware can't describe its layout and can't be backed up. Installing the open firmware is optional and one-way.",
        Some("control-surface") => "Open firmware (control-surface). The keypad describes its own layout and every key and knob reaches the keypad app directly.",
        Some("openmacropad") => "Open firmware (OpenMacroPad, not the smplOS build). Install the control-surface build to let the keypad describe its layout.",
        Some(_) => "Unknown firmware.",
        None => "Connect the keypad to see its firmware.",
    }));
    let (app_text, show_start) = super::app_summary(&st.status.app, pad.is_some());
    ui.set_kp_app_text(s(if st.status_loaded { app_text } else { String::new() }));
    ui.set_kp_show_start(st.status_loaded && show_start);

    // Variant: detected (read-only) for self-describing keypads, else a menu.
    let detected = st.detected();
    let layout_choice = st.config.layout();
    ui.set_kp_variant_detected(s(detected.as_ref().map_or(String::new(), |d| d.0.clone())));
    ui.set_kp_variant_picker(detected.is_none() || st.override_open || layout_choice != Layout::Auto);
    let (menu, index) = st.variant_menu();
    ui.set_kp_variants(ModelRc::from(Rc::new(VecModel::from(menu))));
    ui.set_kp_variant_index(index);
    if let Layout::Custom { keys, knobs, columns } = layout_choice {
        ui.set_kp_custom(true);
        ui.set_kp_custom_keys(keys as i32);
        ui.set_kp_custom_knobs(knobs as i32);
        ui.set_kp_custom_cols(columns as i32);
    } else {
        ui.set_kp_custom(false);
    }
    // Newer keypad apps let the config's layout win; older ones don't.
    let override_ignored = detected.is_some()
        && !st.dirty
        && layout_choice != Layout::Auto
        && st.status.layout.as_ref().is_some_and(|l| l.source == "firmware");
    ui.set_kp_layout_note(s(if override_ignored {
        "This version of the keypad app always uses the keypad's own layout, so the override has no effect until it's updated.".to_string()
    } else if keys > st.max_keys || knobs > st.max_knobs {
        format!(
            "The installed keypad app maps up to {} keys and {} knobs; the others are shown but can't be mapped yet.",
            st.max_keys, st.max_knobs
        )
    } else {
        String::new()
    }));

    // Layout canvas
    let rows = keys.div_ceil(cols).max(knobs).max(1);
    let controls: Vec<KeypadControl> = super::control_ids(keys, knobs)
        .into_iter()
        .map(|id| {
            let knob = id.starts_with("knob");
            let n: usize = id.trim_start_matches(char::is_alphabetic).parse().unwrap_or(1);
            let (col, row) = if knob { (cols, n - 1) } else { ((n - 1) % cols, (n - 1) / cols) };
            let mappable = if knob { n <= st.max_knobs } else { n <= st.max_keys };
            let seen = st.wizard.as_ref().is_some_and(|w| {
                if knob {
                    config::KNOB_EVENTS.iter().all(|(e, _)| w.seen.contains(&format!("{id}.{e}")))
                } else {
                    w.seen.contains(&id)
                }
            });
            KeypadControl {
                label: s(if knob { format!("Knob {n}") } else { n.to_string() }),
                knob,
                col: col as i32,
                row: row as i32,
                summary: s(if mappable { st.summary(&id) } else { "n/a".into() }),
                selected: id == st.control,
                active: st.active.as_deref() == Some(id.as_str()),
                seen,
                id: s(&id),
            }
        })
        .collect();
    ui.set_kp_controls(ModelRc::from(Rc::new(VecModel::from(controls))));
    ui.set_kp_cols(cols as i32);
    ui.set_kp_rows(rows as i32);
    ui.set_kp_knobs(knobs as i32);
    ui.set_kp_identify(st.identify);
    let live = st.status.app.bus || super::mock_events().is_some();
    let testing = st.wizard.as_ref().is_some_and(|w| w.step == 6);
    ui.set_kp_live_note(s(match (live, st.identify || testing) {
        (_, false) => "",
        (false, true) => "Live input needs the keypad app running with a keypad plugged in. Click a control instead.",
        (true, true) if testing => "Mapped actions are paused while you test.",
        (true, true) => "Press a key or turn a knob on the keypad to select it. Mapped actions are paused meanwhile.",
    }));

    // Profiles
    let profiles = st.profiles();
    let current = profiles.get(st.profile).cloned();
    ui.set_kp_profile_names(strings(profiles.iter().map(|p| p.title())));
    ui.set_kp_profile_index(st.profile as i32);
    ui.set_kp_profile_global(current.as_ref().is_none_or(|p| p.global));
    if !st.dirty || ui.get_kp_profile_class().is_empty() {
        ui.set_kp_profile_class(s(current.as_ref().map_or("", |p| p.class.as_str())));
    }
    ui.set_kp_profile_kdenlive(current.as_ref().is_some_and(|p| p.kdenlive));
    ui.set_kp_profile_key_fallback(current.as_ref().is_some_and(|p| p.key_fallback));
    ui.set_kp_profile_note(s(current.as_ref().map_or(String::new(), |p| {
        if p.layers > 0 {
            format!("This profile also has {} context layers (edit them in the file). A layer's binding wins over the one set here while its context is active.", p.layers)
        } else if p.global {
            "Global applies to every app without its own profile, and to controls an app profile leaves unset.".into()
        } else {
            String::new()
        }
    })));
    ui.set_kp_adding_profile(st.adding);
    ui.set_kp_app_classes(strings(st.app_classes.clone()));

    // Editor
    let n: usize = st.control.trim_start_matches(char::is_alphabetic).parse().unwrap_or(1);
    let knob = st.control.starts_with("knob");
    let mappable = if knob { n <= st.max_knobs } else { n <= st.max_keys };
    ui.set_kp_selected_title(s(if mappable {
        config::slot_label(&st.control)
    } else {
        format!("{} (not supported by the keypad app yet)", config::slot_label(&st.control))
    }));
    ui.set_kp_selected_knob(knob);
    ui.set_kp_knob_event(st.knob_event as i32);
    let kinds = st.kinds();
    let global = st.global();
    ui.set_kp_action_names(strings(kinds.iter().map(|k| match (k, global) {
        (ActionKind::Inherit, true) => "Not set".to_string(),
        (k, _) => k.label().to_string(),
    })));
    ui.set_kp_action_index(kinds.iter().position(|k| *k == st.editor.kind).map_or(-1, |i| i as i32));
    let choices = st.choices();
    ui.set_kp_choice_names(strings(choices.iter().map(|(_, l)| l.clone())));
    ui.set_kp_choice_index(choices.iter().position(|(k, _)| *k == st.editor.value).map_or(-1, |i| i as i32));
    let (mode, hint) = match st.editor.kind {
        ActionKind::Inherit if global => (0, "Unmapped: the control does nothing."),
        ActionKind::Inherit => (0, "Uses the Global profile's binding for this control."),
        ActionKind::Disabled => (0, "Does nothing in this profile (also stops the Global binding)."),
        ActionKind::Shortcut => (1, "e.g. ctrl+shift+s, F13, super+1; a sequence: ctrl+k x"),
        ActionKind::Command => (1, "e.g. notify-send hello (runs without a shell)"),
        ActionKind::Media | ActionKind::Mouse | ActionKind::Kdenlive => (2, ""),
        ActionKind::Cheatsheet => (2, "Shows what every key and knob does in the app you're using."),
        ActionKind::Advanced => (3, "Continuous controls, mode cycles and API requests are edited in the config file."),
    };
    ui.set_kp_editor_mode(if mappable && st.config_error.is_none() { mode } else { 0 });
    ui.set_kp_editor_hint(s(if !mappable {
        "The keypad app can't map this control yet."
    } else if st.config_error.is_some() {
        "Fix the config file first (Open file)."
    } else {
        hint
    }));
    ui.set_kp_advanced_text(s(if st.editor.kind == ActionKind::Advanced { st.editor.value.as_str() } else { "" }));
    ui.set_kp_label_enabled(mappable && st.config_error.is_none() && st.editor.takes_label());

    // Cheatsheet: options and the preview for the selected profile.
    ui.set_kp_sheet_supported(st.sheet);
    let o = st.config.sheet_options();
    ui.set_kp_sheet_opacity(o.opacity as f32);
    ui.set_kp_sheet_hide_index(SHEET_HIDE.iter().position(|(ms, _)| *ms == o.auto_hide_ms).map_or(-1, |i| i as i32));
    ui.set_kp_sheet_hide_names(strings(SHEET_HIDE.iter().map(|(_, l)| l.to_string())));
    ui.set_kp_sheet_position(config::SHEET_POSITIONS.iter().position(|p| *p == o.position).map_or(4, |i| i as i32));
    let kdenlive = st.profiles().get(st.profile).is_some_and(|p| p.kdenlive);
    ui.set_kp_sheet_contexts(strings(if kdenlive {
        super::KDENLIVE_CONTEXTS.iter().map(|(l, _)| l.to_string()).collect()
    } else {
        Vec::new()
    }));
    ui.set_kp_sheet_context(st.sheet_context as i32);
    render_sheet_preview(ui, st);

    let rows: Vec<KeypadBindingRow> = st
        .config
        .bound_slots(st.profile)
        .into_iter()
        .map(|(slot, b)| KeypadBindingRow {
            label: s(config::slot_label(&slot)),
            summary: s(match b.kind {
                ActionKind::Advanced => format!("advanced: {}", b.value),
                _ => b.summary(),
            }),
            advanced: b.kind == ActionKind::Advanced,
            slot: s(slot),
        })
        .collect();
    ui.set_kp_binding_rows(ModelRc::from(Rc::new(VecModel::from(rows))));

    ui.set_kp_dirty(st.dirty);
    ui.set_kp_saving(st.saving);
    ui.set_kp_config_path(s(super::config_path().display().to_string()));
    ui.set_kp_config_exists(st.exists);
    ui.set_kp_status(s(match &st.config_error {
        Some(e) if st.message.is_empty() => format!("The config file can't be read: {e}"),
        _ => st.message.clone(),
    }));
    ui.set_kp_status_is_error(st.message_is_error || (st.config_error.is_some() && st.message.is_empty()));

    // Wizard
    ui.set_kp_wizard_open(st.wizard.is_some());
    ui.set_kp_dry_run(!super::real_flash_enabled());
    if let Some(w) = &st.wizard {
        ui.set_kp_wizard_step(w.step as i32);
        ui.set_kp_wizard_title(s(format!("Step {} of {}: {}", w.step + 1, WIZARD_TITLES.len(), WIZARD_TITLES[w.step])));
        ui.set_kp_wizard_ack(w.ack);
        ui.set_kp_fw_names(strings(w.images.iter().map(FirmwareImage::title)));
        ui.set_kp_fw_index(if w.images.is_empty() { -1 } else { w.image as i32 });
        ui.set_kp_fw_detail(s(w.images.get(w.image).map_or(
            "No firmware images are installed yet. They ship with the control-surface package (/usr/share/control-surface/firmware) or can be placed in ~/.local/share/control-surface/firmware.".to_string(),
            |i| {
                let license = if i.license.is_empty() { String::new() } else { format!(" · {}", i.license) };
                let board = if i.board.is_empty() { String::new() } else { format!(" · board {}", i.board) };
                format!("{} · {} bytes · sha256 {}{board}{license}", i.path, i.size, &i.sha256[..i.sha256.len().min(16)])
            },
        )));
        ui.set_kp_wizard_hint(s(&w.hint));
        ui.set_kp_wizard_can_next(wizard_can_next(st, w));
        ui.set_kp_flash_log(strings(w.log.clone()));
        ui.set_kp_flash_running(w.flashing);
        ui.set_kp_verify_seen(w.seen.len() as i32);
        ui.set_kp_verify_total(st.verify_total() as i32);
    }
}

fn wizard_can_next(st: &State, w: &Wizard) -> bool {
    match w.step {
        0 => w.ack,
        1 => w.images.get(w.image).is_some_and(|i| !i.mismatch),
        2 => st.status_loaded && st.status.pads.is_empty() && st.status.bootloaders == 0,
        3 => st.status.bootloaders == 1,
        4 => w.flash_ok && !w.flashing,
        5 => !st.status.pads.is_empty() && st.status.bootloaders == 0,
        _ => true,
    }
}

fn wizard_hint(st: &State, w: &Wizard) -> String {
    let pad = st.status.pads.first();
    match w.step {
        2 if pad.is_some() || st.status.bootloaders > 0 => "Waiting for the keypad to be unplugged…".into(),
        2 => "Unplugged. Continue to the next step.".into(),
        3 if st.status.bootloaders > 1 => "More than one device is in update mode; connect only the keypad.".into(),
        3 if st.status.bootloaders == 1 => "Update mode detected (USB 4348:55e0).".into(),
        3 if pad.is_some() => "The keypad started normally. Unplug it, hold the top-left key and plug it in again.".into(),
        3 => "Waiting for the keypad in update mode…".into(),
        4 if st.status.bootloaders == 0 && !w.flash_ok && !w.flashing => {
            "The keypad left update mode (it times out after a few seconds). Go back and enter it again.".into()
        }
        5 if st.status.bootloaders > 0 => "Still in update mode: unplug it and plug it in without holding a key.".into(),
        5 => match pad {
            Some(p) => format!("Detected: {}.", p.firmware_label),
            None => "Waiting for the keypad…".into(),
        },
        _ => String::new(),
    }
}

// ── Actions ──────────────────────────────────────────────────────────────────

fn load_config(st: &mut State) {
    let path = super::config_path();
    st.config_error = None;
    st.exists = path.exists();
    st.loaded_text = std::fs::read_to_string(&path).ok();
    if st.exists {
        match st.loaded_text.clone().ok_or_else(|| "unreadable".to_string()).and_then(|t| KeypadConfig::parse(&t)) {
            Ok(cfg) => st.config = cfg,
            Err(e) => {
                st.config = KeypadConfig::parse(config::DEFAULT_CONFIG).expect("default config");
                st.config_error = Some(e);
            }
        }
    } else {
        let example = super::example_config(st.daemon.as_deref());
        st.config = example
            .as_deref()
            .and_then(|t| KeypadConfig::parse(t).ok())
            .unwrap_or_else(|| KeypadConfig::parse(config::DEFAULT_CONFIG).expect("default config"));
        st.set_message(
            "No keypad config yet: showing the defaults the keypad app uses. Saving creates the file.",
            false,
        );
    }
    st.dirty = false;
    st.override_open = false;
    st.profile = st.profile.min(st.config.profiles().len().saturating_sub(1));
    st.load_editor();
}

fn refresh_status(ui: &MainWindow) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let status = super::load_status();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            let changed = with(|st| {
                let changed = !st.status_loaded || st.status != status;
                st.status = status;
                st.status_loaded = true;
                advance_wizard(st);
                sync_identify(st, ui.get_active_tab() == KEYPAD_TAB);
                changed
            })
            .unwrap_or(false);
            if changed || ui.get_kp_wizard_open() {
                render(&ui);
            }
        });
    });
}

/// Moves the wizard forward when the hardware reached the awaited state.
fn advance_wizard(st: &mut State) {
    let Some(mut w) = st.wizard.take() else { return };
    let auto = matches!(w.step, 2 | 3 | 5) && !w.backed;
    if auto && wizard_can_next(st, &w) {
        w.step += 1;
        if w.step == 4 {
            w.log.clear();
            w.flash_ok = false;
        }
    }
    w.hint = wizard_hint(st, &w);
    st.wizard = Some(w);
}

/// Passive: listens for the daemon's InputEvent signal (or the mock file)
/// for as long as Settings runs. Events are ignored unless identifying or
/// testing inputs in the wizard.
fn start_live_input(ui: &MainWindow) {
    let start = with(|st| !std::mem::replace(&mut st.live_started, true)).unwrap_or(false);
    if !start {
        return;
    }
    let weak = ui.as_weak();
    super::spawn_live_input(move |event: InputEvent| {
        let weak = weak.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                on_input(&ui, event);
            }
        })
        .is_ok()
    });
}

/// Identify mode is on while the user identifies controls or tests inputs in
/// the wizard, and only while the Keypad tab is showing.
fn sync_identify(st: &mut State, tab_active: bool) {
    let testing = st.wizard.as_ref().is_some_and(|w| w.step == 6);
    if !tab_active {
        st.identify = false;
    }
    let want = tab_active && (st.identify || testing);
    if want != st.identify_sent {
        st.identify_sent = want;
        if let Some(holder) = &st.identify_holder {
            holder.set(want);
        }
    }
}

fn on_input(ui: &MainWindow, event: InputEvent) {
    let mut selected = false;
    let handled = with(|st| {
        let mut handled = false;
        if st.identify {
            selected = true;
            st.control = event.control.clone();
            if let Some(i) = knob_event_index(&event.event) {
                if st.control.starts_with("knob") {
                    st.knob_event = i;
                }
            }
            st.load_editor();
            handled = true;
        }
        if let Some(w) = st.wizard.as_mut().filter(|w| w.step == 6) {
            let input = if event.control.starts_with("knob") {
                format!("{}.{}", event.control, event.event)
            } else {
                event.control.clone()
            };
            if event.event != "release" {
                w.seen.insert(input);
            }
            handled = true;
        }
        if handled {
            st.active = Some(event.control.clone());
        }
        handled
    })
    .unwrap_or(false);
    if !handled {
        return;
    }
    if selected {
        // The text fields belong to the newly selected control now.
        sync_editor_text(ui);
    }
    render(ui);
    let weak = ui.as_weak();
    Timer::single_shot(FLASH_HIGHLIGHT, move || {
        if let Some(ui) = weak.upgrade() {
            with(|st| st.active = None);
            render(&ui);
        }
    });
}

fn set_layout(st: &mut State, choice: Layout) {
    if st.config_error.is_some() || st.config.layout() == choice {
        return;
    }
    match st.config.set_layout(&choice) {
        Ok(()) => {
            st.dirty = true;
            if choice == Layout::Auto {
                st.override_open = false;
            }
            let (keys, knobs, _) = st.layout();
            let knobs_text = if knobs > 0 { format!(", {knobs} knobs") } else { String::new() };
            st.set_message(format!("Keypad variant: {keys} keys{knobs_text}. Not saved yet."), false);
        }
        Err(e) => st.set_message(e, true),
    }
}

fn select_slot(st: &mut State, slot: &str) {
    let (control, event) = slot.split_once('.').unwrap_or((slot, ""));
    st.control = control.to_string();
    if let Some(i) = knob_event_index(event) {
        st.knob_event = i;
    }
    st.load_editor();
}

fn apply_binding(ui: &MainWindow) {
    let text = ui.get_kp_action_text().to_string();
    let label = ui.get_kp_label_text().to_string();
    with(|st| {
        if st.config_error.is_some() {
            return;
        }
        let mut binding = st.editor.clone();
        if matches!(binding.kind, ActionKind::Shortcut | ActionKind::Command) {
            binding.value = text.clone();
        }
        binding.label = if binding.takes_label() { label.trim().to_string() } else { String::new() };
        let slot = st.slot();
        match st.config.set_binding(st.profile, &slot, &binding) {
            Ok(note) => {
                st.dirty = true;
                st.load_editor();
                let what = match st.editor.summary() {
                    v if v.is_empty() => "not set".to_string(),
                    v => v,
                };
                let note = note.map(|n| format!(" {n}.")).unwrap_or_default();
                st.set_message(format!("{} → {what}. Not saved yet.{note}", config::slot_label(&slot)), false);
            }
            Err(e) => st.set_message(e, true),
        }
    });
    sync_editor_text(ui);
    render(ui);
}

/// Puts the editor's value into the text field (only when the selection changes).
fn sync_editor_text(ui: &MainWindow) {
    if let Some((text, label)) = with(|st| {
        let text = match st.editor.kind {
            ActionKind::Shortcut | ActionKind::Command => st.editor.value.clone(),
            _ => String::new(),
        };
        (text, st.editor.label.clone())
    }) {
        ui.set_kp_action_text(s(text));
        ui.set_kp_label_text(s(label));
    }
}

fn save(ui: &MainWindow) {
    let Some((text, daemon, expected)) = with(|st| {
        if st.saving || st.config_error.is_some() {
            return None;
        }
        st.saving = true;
        Some((st.config.render(), st.daemon.clone(), st.loaded_text.clone()))
    })
    .flatten() else {
        return;
    };
    render(ui);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = super::save_config(&super::config_path(), &text, daemon.as_deref(), expected.as_deref());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            with(|st| {
                st.saving = false;
                match result {
                    Ok(saved) => {
                        st.dirty = false;
                        st.exists = true;
                        st.loaded_text = Some(text);
                        let backup = saved
                            .backup
                            .and_then(|b| b.file_name().map(|n| format!(" Previous version: backups/{}.", n.to_string_lossy())))
                            .unwrap_or_default();
                        let applies = if st.status.app.running() {
                            "the keypad app applies it right away"
                        } else {
                            "it takes effect when the keypad app runs"
                        };
                        let check = match saved.validation {
                            Validation::Ok { warnings } if warnings.is_empty() => {
                                format!("Saved and checked; {applies}.")
                            }
                            Validation::Ok { warnings } => format!(
                                "Saved with {} warning(s): {}",
                                warnings.len(),
                                warnings.first().cloned().unwrap_or_default()
                            ),
                            Validation::Unchecked => {
                                "Saved. Not checked: the keypad app isn't installed.".to_string()
                            }
                            Validation::Invalid(_) => unreachable!("invalid configs are never written"),
                        };
                        st.set_message(format!("{check}{backup}"), false);
                    }
                    Err(e) => st.set_message(e, true),
                }
            });
            render(&ui);
        });
    });
}

fn running_app_classes() -> Vec<String> {
    let Ok(out) = Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(clients)) = serde_json::from_slice(&out.stdout) else {
        return Vec::new();
    };
    let mut classes: Vec<String> = clients
        .iter()
        .filter_map(|c| c.get("class").and_then(|v| v.as_str()))
        .filter(|c| !c.is_empty() && *c != "settings")
        .map(String::from)
        .collect();
    classes.sort();
    classes.dedup();
    classes
}

fn open_config_file() -> Result<(), String> {
    let path = super::config_path();
    if !path.exists() {
        return Err("Save once to create the file first.".into());
    }
    let has = |tool: &str| {
        Command::new("which")
            .arg(tool)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| if has("micro") { "micro".into() } else { "nano".into() });
    Command::new("terminal")
        .args(["-e", &editor])
        .arg(&path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not open a terminal: {e}"))
}

fn start_flash(ui: &MainWindow) {
    let Some(image) = with(|st| {
        let w = st.wizard.as_mut()?;
        if w.flashing || w.step != 4 {
            return None;
        }
        let image = w.images.get(w.image)?.path.clone();
        w.flashing = true;
        w.flash_ok = false;
        w.log.clear();
        Some(image)
    })
    .flatten() else {
        return;
    };
    render(ui);
    let weak = ui.as_weak();
    let execute = super::real_flash_enabled();
    std::thread::spawn(move || {
        let push = |line: String| {
            let weak = weak.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    with(|st| {
                        if let Some(w) = st.wizard.as_mut() {
                            w.log.push(line);
                        }
                    });
                    render(&ui);
                }
            });
        };
        let ok = super::flash(&image, execute, |step| {
            let mark = match step.status.as_str() {
                "ok" => "\u{2713}",
                "failed" => "\u{2717}",
                "simulated" | "skipped" => "\u{25cb}",
                _ => "\u{2026}",
            };
            push(format!("{mark} {}: {}", step.step, step.detail));
        });
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                with(|st| {
                    if let Some(w) = st.wizard.as_mut() {
                        w.flashing = false;
                        w.flash_ok = ok;
                    }
                });
                render(&ui);
            }
        });
    });
}

// ── Installation ─────────────────────────────────────────────────────────────

pub struct KeypadTab {
    _poll: Timer,
}

/// First visit of the tab: find the daemon, read the config and start the
/// slow probes. Nothing here runs at Settings startup unless the Keypad tab is
/// the one being opened.
fn ensure_loaded(ui: &MainWindow) -> bool {
    let first = with(|st| {
        if st.loaded {
            return false;
        }
        st.loaded = true;
        st.daemon = super::daemon_binary();
        st.identify_holder = Some(super::Identify::spawn());
        load_config(st);
        // Start on the generic tier: the Global profile.
        st.profile = st.config.profiles().iter().position(|p| p.global).unwrap_or(0);
        st.load_editor();
        true
    })
    .unwrap_or(false);
    if !first {
        return false;
    }
    let weak = ui.as_weak();
    let daemon = with(|st| st.daemon.clone()).flatten();
    std::thread::spawn(move || {
        let variants = super::variants(daemon.as_deref());
        let features = super::features(daemon.as_deref());
        let mouse = match &features {
            Some(f) => f.mouse,
            None => super::probe_mouse(daemon.as_deref()),
        };
        let mut actions = super::running_kdenlive_actions(daemon.as_deref());
        if actions.is_empty() {
            actions = super::kdenlive_catalog(daemon.as_deref());
        }
        let _ = slint::invoke_from_event_loop(move || {
            with(|st| {
                st.variants = variants;
                st.mouse = mouse;
                st.kdenlive_actions = actions;
                if let Some(f) = features {
                    st.max_keys = f.max_keys;
                    st.max_knobs = f.max_knobs;
                    st.sheet = f.cheatsheet;
                }
            });
            if let Some(ui) = weak.upgrade() {
                render(&ui);
            }
        });
    });
    start_live_input(ui);
    refresh_status(ui);
    sync_editor_text(ui);
    render(ui);
    true
}

pub fn install(ui: &MainWindow) -> KeypadTab {
    let state = State {
        loaded: false,
        status: Status::default(),
        status_loaded: false,
        variants: super::builtin_variants(),
        override_open: false,
        daemon: None,
        config: KeypadConfig::parse(config::DEFAULT_CONFIG).expect("default config"),
        config_error: None,
        exists: false,
        loaded_text: None,
        dirty: false,
        profile: 0,
        control: "key1".into(),
        knob_event: 0,
        editor: Binding::new(ActionKind::Inherit, ""),
        mouse: false,
        sheet: false,
        kdenlive_actions: Vec::new(),
        sheet_context: 0,
        sheet_key: String::new(),
        sheet_generation: 0,
        sheet_preview: Err(String::new()),
        identify: false,
        identify_sent: false,
        identify_holder: None,
        max_keys: LEGACY_KEYS,
        max_knobs: LEGACY_KNOBS,
        live_started: false,
        active: None,
        adding: false,
        app_classes: Vec::new(),
        wizard: None,
        message: String::new(),
        message_is_error: false,
        saving: false,
    };
    STATE.with(|cell| *cell.borrow_mut() = Some(state));
    if ui.get_active_tab() == KEYPAD_TAB {
        ensure_loaded(ui);
    }

    let weak = ui.as_weak();
    ui.on_kp_refresh(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ensure_loaded(&ui) {
            return;
        }
        with(|st| {
            if !st.dirty {
                load_config(st);
                if st.exists {
                    st.message.clear();
                }
            }
            sync_identify(st, true);
        });
        sync_editor_text(&ui);
        refresh_status(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_start_app(move || {
        let Some(ui) = weak.upgrade() else { return };
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = super::start_app();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    if let Err(e) = result {
                        with(|st| st.set_message(format!("Could not start the keypad app: {e}"), true));
                    }
                    refresh_status(&ui);
                }
            });
        });
    });

    let weak = ui.as_weak();
    ui.on_kp_open_scope_help(move || {
        let Some(ui) = weak.upgrade() else { return };
        if Command::new("xdg-open").arg(SCOPE_HELP_URL).spawn().is_err() {
            with(|st| st.set_message(format!("Open {SCOPE_HELP_URL} in a browser."), false));
            render(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_override_variant(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.override_open = true);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_variant(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let (menu, _) = st.variant_menu();
            let i = i.max(0) as usize;
            let choice = if i == 0 {
                Layout::Auto
            } else if i + 1 == menu.len() {
                let (keys, knobs, columns) = st.layout();
                Layout::Custom {
                    keys: keys.min(config::MAX_KEYS),
                    knobs: knobs.min(config::MAX_KNOBS),
                    columns: columns.clamp(1, config::MAX_COLUMNS),
                }
            } else if let Some(v) = st.variants.get(i - 1) {
                Layout::Board(v.id.clone())
            } else {
                return; // the "unknown id" entry: already selected
            };
            set_layout(st, choice);
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_custom(move |keys, knobs, columns| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let choice = Layout::Custom {
                keys: (keys.max(0) as usize).min(config::MAX_KEYS),
                knobs: (knobs.max(0) as usize).min(config::MAX_KNOBS),
                columns: (columns.max(1) as usize).min(config::MAX_COLUMNS),
            };
            set_layout(st, choice);
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_opacity(move |v| {
        if let Some(ui) = weak.upgrade() {
            let v = (f64::from(v) * 20.0).round() / 20.0;
            set_sheet_option(&ui, |o| o.opacity = v.clamp(0.05, 1.0));
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_hide(move |i| {
        if let (Some(ui), Some((ms, _))) = (weak.upgrade(), SHEET_HIDE.get(i.max(0) as usize)) {
            set_sheet_option(&ui, |o| o.auto_hide_ms = *ms);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_position(move |i| {
        if let (Some(ui), Some(p)) = (weak.upgrade(), config::SHEET_POSITIONS.get(i.max(0) as usize)) {
            set_sheet_option(&ui, |o| o.position = p.to_string());
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_context(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.sheet_context = i.max(0) as usize);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_show_sheet(move || {
        let Some(ui) = weak.upgrade() else { return };
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = super::show_sheet();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    let push = with(|st| st.status.sheet_push).flatten();
                    let message = match (result, push) {
                        (Ok(()), Some(false)) => (
                            "The keypad app isn't sending its cheatsheet to the bar. It needs to run as control-surface.service (or with --eww-window pad-cheatsheet).".to_string(),
                            true,
                        ),
                        (Ok(()), _) => ("Cheatsheet shown on screen (it uses the saved config).".to_string(), false),
                        (Err(e), _) => (e, true),
                    };
                    with(|st| st.set_message(message.0, message.1));
                    render(&ui);
                }
            });
        });
    });

    let weak = ui.as_weak();
    ui.on_kp_select_control(move |slot| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| select_slot(st, &slot));
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_knob_event(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.knob_event = (i.max(0) as usize).min(2);
            st.load_editor();
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_profile(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.profile = i.max(0) as usize;
            st.adding = false;
            st.load_editor();
        });
        let class = with(|st| st.profiles().get(st.profile).map(|p| p.class.clone())).flatten();
        ui.set_kp_profile_class(s(class.unwrap_or_default()));
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_start_add_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.adding = true);
        ui.set_kp_new_app(SharedString::default());
        render(&ui);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let classes = running_app_classes();
            let _ = slint::invoke_from_event_loop(move || {
                with(|st| st.app_classes = classes);
                if let Some(ui) = weak.upgrade() {
                    render(&ui);
                }
            });
        });
    });

    let weak = ui.as_weak();
    ui.on_kp_pick_app(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(class) = with(|st| st.app_classes.get(i as usize).cloned()).flatten() {
            ui.set_kp_new_app(s(class));
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_confirm_add_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        let class = ui.get_kp_new_app().to_string();
        with(|st| {
            let name = class.rsplit('.').next().unwrap_or(&class).to_string();
            match st.config.add_app_profile(&name, &class) {
                Ok(i) => {
                    st.profile = i;
                    st.adding = false;
                    st.dirty = true;
                    st.load_editor();
                    st.set_message(format!("Added a profile for {class}. Map its controls, then Save."), false);
                }
                Err(e) => st.set_message(e, true),
            }
        });
        let class = with(|st| st.profiles().get(st.profile).map(|p| p.class.clone())).flatten();
        ui.set_kp_profile_class(s(class.unwrap_or_default()));
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_cancel_add_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.adding = false);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_remove_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let name = st.profiles().get(st.profile).map(|p| p.name.clone()).unwrap_or_default();
            match st.config.remove_profile(st.profile) {
                Ok(()) => {
                    st.profile = st.profile.saturating_sub(1);
                    st.dirty = true;
                    st.load_editor();
                    st.set_message(format!("Removed the {name} profile. Not saved yet."), false);
                }
                Err(e) => st.set_message(e, true),
            }
        });
        let class = with(|st| st.profiles().get(st.profile).map(|p| p.class.clone())).flatten();
        ui.set_kp_profile_class(s(class.unwrap_or_default()));
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_apply_profile_class(move || {
        let Some(ui) = weak.upgrade() else { return };
        let regex = ui.get_kp_profile_class().to_string();
        with(|st| match st.config.set_profile_class(st.profile, &regex) {
            Ok(()) => {
                st.dirty = true;
                st.set_message("Window class updated. Not saved yet.", false);
            }
            Err(e) => st.set_message(e, true),
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_kdenlive(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if st.config.set_profile_flag(st.profile, "kdenlive", on).is_ok() {
                st.dirty = true;
                st.set_message(
                    if on {
                        "Kdenlive API plugin on: this profile can trigger Kdenlive actions over D-Bus. Enable the control-surface interface in Kdenlive's settings."
                    } else {
                        "Kdenlive API plugin off for this profile."
                    },
                    false,
                );
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_key_fallback(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if st.config.set_profile_flag(st.profile, "keyFallback", on).is_ok() {
                st.dirty = true;
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_load_kdenlive_preset(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let preset = super::example_config(st.daemon.as_deref())
                .as_deref()
                .and_then(config::kdenlive_preset);
            match preset {
                Some(p) => {
                    st.profile = st.config.apply_kdenlive_preset(p);
                    st.dirty = true;
                    st.load_editor();
                    st.set_message("Loaded the recommended Kdenlive layout (with context layers). Not saved yet.", false);
                }
                None => st.set_message("The recommended layout comes with the keypad app, which isn't installed.", true),
            }
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_action(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let Some(kind) = st.kinds().get(i.max(0) as usize).copied() else { return };
            if kind == st.editor.kind {
                return;
            }
            let current = st.config.binding(st.profile, &st.slot());
            let label = st.editor.label.clone();
            st.editor = if current.kind == kind {
                current
            } else {
                Binding::new(kind, "").labelled(&label)
            };
            if let Some((first, _)) = st.choices().first().filter(|_| st.editor.value.is_empty()) {
                st.editor.value = first.clone();
            }
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_choice(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if let Some((id, _)) = st.choices().get(i.max(0) as usize) {
                st.editor.value = id.clone();
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_apply_binding(move || {
        if let Some(ui) = weak.upgrade() {
            apply_binding(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_save(move || {
        if let Some(ui) = weak.upgrade() {
            save(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_revert(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            load_config(st);
            st.set_message("Reverted to the saved config.", false);
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_open_config(move || {
        let Some(ui) = weak.upgrade() else { return };
        if let Err(e) = open_config_file() {
            with(|st| st.set_message(e, true));
            render(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_toggle_identify(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.identify = !st.identify;
            sync_identify(st, true);
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_open_wizard(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.identify = false;
            st.wizard = Some(Wizard::default());
            sync_identify(st, true);
        });
        render(&ui);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let images = super::firmware_images();
            let _ = slint::invoke_from_event_loop(move || {
                with(|st| {
                    if let Some(w) = st.wizard.as_mut() {
                        w.image = images.iter().position(|i| i.verified).unwrap_or(0);
                        w.images = images;
                    }
                });
                if let Some(ui) = weak.upgrade() {
                    render(&ui);
                }
            });
        });
    });

    let weak = ui.as_weak();
    ui.on_kp_set_wizard_ack(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if let Some(w) = st.wizard.as_mut() {
                w.ack = on;
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_firmware(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if let Some(w) = st.wizard.as_mut() {
                w.image = i.max(0) as usize;
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_wizard_next(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let Some(mut w) = st.wizard.take() else { return };
            if !wizard_can_next(st, &w) {
                st.wizard = Some(w);
                return;
            }
            if w.step + 1 >= WIZARD_TITLES.len() {
                st.set_message(
                    format!("Firmware wizard finished: {} of {} inputs confirmed.", w.seen.len(), st.verify_total()),
                    false,
                );
                return;
            }
            w.step += 1;
            w.backed = false;
            if w.step == 4 {
                w.log.clear();
                w.flash_ok = false;
            }
            w.hint = wizard_hint(st, &w);
            st.wizard = Some(w);
        });
        with(|st| sync_identify(st, true));
        refresh_status(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_wizard_back(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if let Some(mut w) = st.wizard.take() {
                if !w.flashing {
                    w.step = w.step.saturating_sub(1);
                    w.backed = true;
                    w.hint = wizard_hint(st, &w);
                }
                st.wizard = Some(w);
            }
            sync_identify(st, true);
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_close_wizard(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if st.wizard.as_ref().is_some_and(|w| !w.flashing) {
                st.wizard = None;
            }
            sync_identify(st, true);
        });
        refresh_status(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_flash(move || {
        if let Some(ui) = weak.upgrade() {
            start_flash(&ui);
        }
    });

    // Poll the keypad state while the tab is visible (faster in the wizard).
    let poll = Timer::default();
    let weak = ui.as_weak();
    let ticks = std::cell::Cell::new(0u32);
    poll.start(TimerMode::Repeated, WIZARD_POLL, move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_active_tab() != KEYPAD_TAB {
            if with(|st| st.identify_sent).unwrap_or(false) {
                with(|st| sync_identify(st, false));
                render(&ui);
            }
            return;
        }
        if ensure_loaded(&ui) {
            return;
        }
        ticks.set(ticks.get().wrapping_add(1));
        let every = (POLL.as_millis() / WIZARD_POLL.as_millis()) as u32;
        if ui.get_kp_wizard_open() || ticks.get().is_multiple_of(every) {
            refresh_status(&ui);
        }
    });
    KeypadTab { _poll: poll }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wizard_titles_match_the_ui_step_count() {
        assert_eq!(WIZARD_TITLES.len(), 7);
    }

    #[test]
    fn knob_events_map_to_editor_indices() {
        assert_eq!(knob_event_index("ccw"), Some(0));
        assert_eq!(knob_event_index("cw"), Some(1));
        assert_eq!(knob_event_index("press"), Some(2));
        assert_eq!(knob_event_index("release"), None);
    }
}
