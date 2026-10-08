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

use super::config::{self, ActionKind, Binding, KeypadConfig, Layout, SheetOptions, ICON_NONE};
use super::icons;
use super::{FirmwareImage, InputEvent, SheetPreview, Status, Validation, Variant};
use crate::{
    KeypadBindingRow, KeypadControl, KeypadIcon, KeypadSheetCell, KeypadSheetLine, KeypadTuning, KeypadVariant, MainWindow,
};

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
    /// The input mode to put back when the wizard ends (it switches the
    /// keypad app to the keymap for the update).
    restore_input: Option<String>,
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
    /// The daemon's values for cheatsheet options the config leaves out.
    sheet_defaults: config::SheetDefaults,
    /// The installed keypad app draws binding icons on the cheatsheet.
    icons_shown: bool,
    /// The keypad app's own words for each input mode (`features`).
    input_modes: Vec<(String, String)>,
    /// Options the installed keypad app sets in place (`SetOption`).
    direct_options: HashSet<String>,
    /// Option changes waiting for the debounce timer, then `SetOption`.
    pending_options: Vec<(String, String)>,
    /// The firmware reports turns while a knob is pressed (`None`: unknown).
    shift_supported: Option<bool>,
    /// The keypad app has held layers ("while holding key N").
    held_supported: bool,
    /// The held layer being edited (index in the profile's "layers"); `None`:
    /// the profile's normal bindings.
    layer: Option<usize>,
    /// "Hold a key…": the next control clicked becomes a held layer's key.
    picking_held: bool,
    /// The key chosen for converting shift bindings (index: key1 = 0).
    shift_key: usize,
    icon_picker: bool,
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
        let mut context = if profile.is_some_and(|p| p.kdenlive) {
            super::KDENLIVE_CONTEXTS[self.sheet_context.min(super::KDENLIVE_CONTEXTS.len() - 1)].1.to_string()
        } else {
            String::new()
        };
        // A held layer previews as if its key(s) were down ("$held").
        if let Some(h) = self.held_scope() {
            context = super::with_held(&context, &h.held[0].join("+"));
        }
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
        self.editor = self.config.binding_at(self.profile, self.held_index(), &self.slot());
    }

    /// The held layer being edited, if it still is one.
    fn held_scope(&self) -> Option<config::HeldLayer> {
        let i = self.layer?;
        self.config.held_layers(self.profile).into_iter().find(|h| h.index == i)
    }

    fn held_index(&self) -> Option<usize> {
        self.held_scope().map(|h| h.index)
    }

    /// The selected control is the one held for the layer being edited.
    fn editing_held_control(&self) -> bool {
        self.held_scope().is_some_and(|h| h.controls().contains(&self.control))
    }

    /// Back to the profile's normal bindings (another profile, a reload).
    fn leave_layer(&mut self) {
        self.layer = None;
        self.picking_held = false;
    }

    /// The icon the keypad app resolved for `slot` in the preview (the
    /// selected profile, as saved or with unsaved edits applied).
    fn preview_icon(&self, slot: &str) -> String {
        let Ok(p) = &self.sheet_preview else { return String::new() };
        let (control, event) = slot.split_once('.').unwrap_or((slot, ""));
        let entry = if event.is_empty() {
            p.keys.iter().find(|k| k.control == control).and_then(|k| k.entries.first())
        } else {
            let i = ["ccw", "press", "cw"].iter().position(|e| *e == event);
            p.knobs.iter().find(|k| k.control == control).and_then(|k| k.entries.get(i?))
        };
        entry.map(|e| e.icon.clone()).unwrap_or_default()
    }

    /// The icon the cheatsheet shows for this profile's own binding of `slot`:
    /// its "icon", or the keypad app's automatic one; none when unbound here.
    fn slot_icon(&self, slot: &str) -> String {
        let b = self.config.binding_at(self.profile, self.held_index(), slot);
        let turned = slot.ends_with(".ccw") || slot.ends_with(".cw");
        let by_turn = || {
            let knob = slot.split('.').next().unwrap_or(slot);
            self.config.binding_at(self.profile, self.held_index(), &format!("{knob}.turn")).kind != ActionKind::Inherit
        };
        match b.kind {
            ActionKind::Inherit if turned && by_turn() => self.preview_icon(slot),
            ActionKind::Inherit => String::new(),
            _ if b.icon == ICON_NONE => String::new(),
            _ if !b.icon.is_empty() => b.icon,
            _ => self.preview_icon(slot),
        }
    }

    /// The glyph and caption of the editor's icon row.
    fn icon_caption(&self) -> (&'static str, String) {
        let icon = self.editor.icon.as_str();
        let (glyph, text) = if icon == ICON_NONE {
            ("", "No icon: the label alone".to_string())
        } else if icon.is_empty() {
            match self.preview_icon(&self.slot()) {
                auto if auto.is_empty() => ("", "Automatic: the keypad app picks one".to_string()),
                auto => (icons::glyph(&auto), format!("Automatic: {auto}")),
            }
        } else {
            match icons::glyph(icon) {
                "" => ("", format!("{icon} (not in smplOS's icon set: label only)")),
                glyph => (glyph, icon.to_string()),
            }
        };
        let applied = self.config.binding_at(self.profile, self.held_index(), &self.slot()).icon == icon;
        (glyph, if applied { text } else { format!("{text}. Not applied yet") })
    }

    fn summary(&self, control: &str) -> String {
        if control.starts_with("knob") {
            let layer = self.held_index();
            let turn = self.config.binding_at(self.profile, layer, &format!("{control}.turn"));
            if turn.kind != ActionKind::Inherit {
                return "turn".into();
            }
            let left = self.config.binding_at(self.profile, layer, &format!("{control}.ccw")).short();
            let right = self.config.binding_at(self.profile, layer, &format!("{control}.cw")).short();
            return match (left.is_empty(), right.is_empty()) {
                (true, true) => String::new(),
                _ => format!("{left} | {right}"),
            };
        }
        self.config.binding_at(self.profile, self.held_index(), control).short()
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
/// How long Show on screen leaves a sheet whose saved setting is "until hidden".
const SHEET_SHOW_PREVIEW_MS: u32 = 8000;

const SHEET_HIDE: [(u32, &str); 7] = [
    (0, "Until hidden (click it or press the key)"),
    (3000, "After 3 s without keypad input"),
    (5000, "After 5 s without keypad input"),
    (8000, "After 8 s without keypad input"),
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
                        glyph: s(icons::glyph(&e.icon)),
                        lines: ModelRc::default(),
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
                let lines: Vec<KeypadSheetLine> = ["rotate", "circle-dot", "rotate-clockwise"]
                    .iter()
                    .zip(&k.entries)
                    .map(|(dir, e)| KeypadSheetLine {
                        dir: s(icons::glyph(dir)),
                        glyph: s(icons::glyph(&e.icon)),
                        text: s(if e.state.is_empty() { e.label.clone() } else { format!("{} ({})", e.label, e.state) }),
                        bound: e.bound,
                        active: e.active,
                    })
                    .collect();
                cells.push(KeypadSheetCell {
                    num: s(format!("Knob {}", k.control.trim_start_matches("knob"))),
                    text: SharedString::new(),
                    glyph: SharedString::new(),
                    lines: ModelRc::from(Rc::new(VecModel::from(lines))),
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
                // The layout, binding list and icon row show its icons too.
                STATE.with(|cell| {
                    if let Some(st) = cell.borrow().as_ref() {
                        render_state(&ui, st);
                    }
                });
            }
        });
    });
}

fn set_sheet_option(ui: &MainWindow, change: impl FnOnce(&mut SheetOptions)) {
    let armed = with(|st| {
        if st.config_error.is_some() {
            return false;
        }
        let before = st.config.sheet_options(&st.sheet_defaults);
        let mut o = before.clone();
        change(&mut o);
        if let Err(e) = st.config.set_sheet_options(&o) {
            st.set_message(e, true);
            return false;
        }
        let note = if o.click_through && !before.click_through && before.auto_hide_ms == 0 {
            format!(" It now hides after {} s, because it can't be clicked away.", o.auto_hide_ms / 1000)
        } else if before.click_through && !o.click_through && o.auto_hide_ms == 0 {
            " Click-through is off, so a click can close it.".to_string()
        } else {
            String::new()
        };
        // Click-through is a window name, not a simple option: the file path.
        let options = if o.click_through != before.click_through { None } else { Some(sheet_option_changes(&before, &o)) };
        commit(st, options, &format!("Cheatsheet options changed.{note}"))
    })
    .unwrap_or(false);
    if armed {
        arm_options(ui);
    }
    render(ui);
}

/// The keypad app's option keys and values (as text) for what changed.
fn sheet_option_changes(before: &SheetOptions, after: &SheetOptions) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if after.opacity != before.opacity {
        out.push(("cheatsheet.opacity".to_string(), config::number(after.opacity)));
    }
    if after.auto_hide_ms != before.auto_hide_ms {
        out.push(("cheatsheet.autoHideMs".to_string(), after.auto_hide_ms.to_string()));
    }
    if after.position != before.position {
        out.push(("cheatsheet.position".to_string(), after.position.clone()));
    }
    if after.overlay != before.overlay {
        out.push(("cheatsheet.eww".to_string(), after.overlay.to_string()));
    }
    out
}

/// Whether the running keypad app can write `options` in place: it is on
/// the bus, knows them (`features.options`) and runs on this file.
fn direct_route(st: &State, options: &[(String, String)]) -> bool {
    let same_file = st.status.config_path.as_deref().is_some_and(|daemon| {
        let ours = super::config_path();
        let canon = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        canon(std::path::Path::new(daemon)) == canon(&ours)
    });
    !options.is_empty()
        && st.status.app.bus
        && same_file
        && st.exists
        && st.config_error.is_none()
        && options.iter().all(|(k, _)| st.direct_options.contains(k))
}

/// After a change to the in-memory config: simple options go to the keypad
/// app (`SetOption`, written in place with comments kept and applied at
/// once); anything else waits for Save. Returns whether to arm the timer.
fn commit(st: &mut State, options: Option<Vec<(String, String)>>, what: &str) -> bool {
    match options {
        Some(options) if direct_route(st, &options) => {
            for (key, value) in options {
                st.pending_options.retain(|(k, _)| *k != key);
                st.pending_options.push((key, value));
            }
            st.set_message(format!("{what} Saving…"), false);
            true
        }
        Some(options) if options.is_empty() => false,
        _ => {
            st.dirty = true;
            st.set_message(format!("{what} Not saved yet."), false);
            false
        }
    }
}

thread_local! {
    static OPTION_TIMER: Timer = Timer::default();
}

/// Sends queued options once the user pauses (a slider drag sends one).
fn arm_options(ui: &MainWindow) {
    let weak = ui.as_weak();
    OPTION_TIMER.with(|t| {
        t.start(TimerMode::SingleShot, Duration::from_millis(350), move || {
            if let Some(ui) = weak.upgrade() {
                flush_options(&ui);
            }
        })
    });
}

fn flush_options(ui: &MainWindow) {
    let Some((changes, snapshot)) = with(|st| {
        if st.pending_options.is_empty() {
            return None;
        }
        Some((std::mem::take(&mut st.pending_options), st.loaded_text.clone()))
    })
    .flatten() else {
        return;
    };
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let path = super::config_path();
        let before = std::fs::read_to_string(&path).ok();
        let results: Vec<(String, super::OptionResult)> =
            changes.iter().map(|(k, v)| (k.clone(), super::set_option(k, v))).collect();
        let after = std::fs::read_to_string(&path).ok();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            with(|st| option_results(st, &results, snapshot, before, after));
            sync_editor_text(&ui);
            render(&ui);
        });
    });
}

fn option_label(key: &str) -> String {
    match key {
        "input" => "Input mode".into(),
        "cheatsheet.opacity" => "Cheatsheet opacity".into(),
        "cheatsheet.autoHideMs" => "Cheatsheet hiding".into(),
        "cheatsheet.position" => "Cheatsheet position".into(),
        "cheatsheet.eww" => "Cheatsheet overlay".into(),
        other => {
            let name = other.trim_start_matches("settings.");
            config::TUNINGS.iter().find(|t| t.key == name).map_or(name.to_string(), |t| t.label.to_string())
        }
    }
}

/// Takes in what `SetOption` did. The file it wrote becomes the baseline
/// for Save's "changed on disk" check, unless something else changed the
/// file meanwhile; a clean editor re-reads it, so it shows the file. What the
/// keypad app couldn't set stays an unsaved change for Save.
fn option_results(
    st: &mut State,
    results: &[(String, super::OptionResult)],
    snapshot: Option<String>,
    before: Option<String>,
    after: Option<String>,
) {
    use super::OptionResult::*;
    let wrote = results.iter().any(|(_, r)| matches!(r, Set { changed: true, .. } | Unapplied(_)));
    // Kept in the editor for Save, so it must not be re-read from the file.
    if results.iter().any(|(_, r)| matches!(r, Unsupported | Refused(_))) {
        st.dirty = true;
    }
    if before == snapshot && st.loaded_text == snapshot {
        st.loaded_text = after.clone();
        st.exists = after.is_some();
        if !st.dirty && st.pending_options.is_empty() {
            if let Some(cfg) = after.as_deref().and_then(|t| KeypadConfig::parse(t).ok()) {
                st.config = cfg;
                st.profile = st.profile.min(st.config.profiles().len().saturating_sub(1));
                st.load_editor();
            }
        }
    } else if wrote && !st.dirty && st.pending_options.is_empty() {
        load_config(st);
    }
    let names = |pick: &dyn Fn(&super::OptionResult) -> bool| {
        results.iter().filter(|(_, r)| pick(r)).map(|(k, _)| option_label(k)).collect::<Vec<_>>().join(", ")
    };
    if let Some((key, Refused(m))) = results.iter().find(|(_, r)| matches!(r, Refused(_))) {
        st.dirty = true;
        st.set_message(format!("The keypad app refused {}: {m}. Not saved yet.", option_label(key)), true);
    } else if results.iter().any(|(_, r)| *r == Unsupported) {
        st.dirty = true;
        st.set_message(
            format!("{}: the keypad app can't set this directly, so it's an unsaved change. Save writes it.", names(&|r| *r == Unsupported)),
            false,
        );
    } else if let Some((_, Unapplied(m))) = results.iter().find(|(_, r)| matches!(r, Unapplied(_))) {
        st.set_message(format!("Saved, but the keypad app couldn't apply it: {m}"), true);
    } else {
        let backup = results
            .iter()
            .find_map(|(_, r)| match r {
                Set { backup: Some(b), .. } => Some(b.clone()),
                _ => None,
            })
            .map(|b| format!(" Previous file: {}.", std::path::Path::new(&b).file_name().map_or(b.clone(), |n| n.to_string_lossy().into_owned())))
            .unwrap_or_default();
        let others = if st.dirty { " Your other changes are still unsaved." } else { "" };
        st.set_message(format!("{}: saved and applied.{backup}{others}", names(&|r| matches!(r, Set { .. }))), false);
    }
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
    // Held layers: what the layout marks as held now, and what can be held.
    let held_now: Vec<String> = st.held_scope().map(|h| h.controls()).unwrap_or_default();
    let holding: Vec<String> = st.config.held_layers(st.profile).iter().flat_map(|h| h.controls()).collect();
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
            let glyph = if !mappable {
                String::new()
            } else if knob {
                let glyphs: Vec<&str> =
                    ["ccw", "press", "cw"].iter().map(|e| icons::glyph(&st.slot_icon(&format!("{id}.{e}")))).collect();
                if glyphs.iter().all(|g| g.is_empty()) {
                    String::new()
                } else {
                    glyphs.iter().map(|g| if g.is_empty() { "\u{2009}" } else { g }).collect::<Vec<_>>().join(" ")
                }
            } else {
                icons::glyph(&st.slot_icon(&id)).to_string()
            };
            KeypadControl {
                held: held_now.contains(&id),
                holds: st.layer.is_none() && holding.contains(&id),
                glyph: s(if held_now.contains(&id) { String::new() } else { glyph }),
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
    if !st.dirty || ui.get_kp_profile_name().is_empty() {
        ui.set_kp_profile_name(s(current.as_ref().map_or("", |p| p.name.as_str())));
    }
    if !st.dirty {
        ui.set_kp_profile_title(s(current.as_ref().map_or("", |p| p.title.as_str())));
    }
    ui.set_kp_profile_fallthrough(current.as_ref().is_none_or(|p| p.fallthrough));
    ui.set_kp_profile_kdenlive(current.as_ref().is_some_and(|p| p.kdenlive));
    ui.set_kp_profile_key_fallback(current.as_ref().is_some_and(|p| p.key_fallback));
    ui.set_kp_profile_note(s(current.as_ref().map_or(String::new(), |p| {
        let context = st.config.context_layer_count(st.profile);
        if context > 0 {
            format!("This profile also has {context} context layers (edit them in the file). A layer's binding wins over the one set here while its context is active.")
        } else if p.global {
            "Global applies to every app without its own profile, and to controls an app profile leaves unset.".into()
        } else {
            String::new()
        }
    })));
    let shift = if st.shift_supported == Some(true) { Vec::new() } else { st.config.shift_bindings(st.profile) };
    ui.set_kp_shift_note(s(if shift.is_empty() {
        String::new()
    } else {
        format!(
            "Turn-while-pressed bindings on {}: the keypad's firmware ignores turns while a knob is pressed, so they never fire, and they hold back that knob's press until you let go.",
            shift.join(", ")
        )
    }));
    let keys = st.layout().0.max(1);
    ui.set_kp_shift_keys(strings((1..=keys).map(|k| format!("Key {k}"))));
    ui.set_kp_shift_key(st.shift_key.min(keys - 1) as i32);
    let held = st.config.held_layers(st.profile);
    let scope = st.held_scope();
    ui.set_kp_held_supported(st.held_supported);
    ui.set_kp_hold_glyph(s(icons::glyph("hand-finger")));
    ui.set_kp_layer_names(strings(
        std::iter::once("Normal (no key held)".to_string()).chain(held.iter().map(|h| h.title())),
    ));
    ui.set_kp_layer_index(scope.as_ref().and_then(|c| held.iter().position(|h| h.index == c.index)).map_or(0, |i| i as i32 + 1));
    ui.set_kp_layer_held(scope.is_some());
    ui.set_kp_picking_held(st.picking_held);
    ui.set_kp_layer_note(s(match &scope {
        Some(h) if !st.held_supported => format!(
            "{}: the keypad app installed here doesn't know held layers yet, so this layer does nothing until it's updated.",
            h.title()
        ),
        Some(h) => format!(
            "{}, the controls below do what you map here; controls you leave unset keep their normal binding. {}'s own binding fires when you tap it without using anything else.",
            h.title(),
            h.controls().iter().map(|c| config::slot_label(c)).collect::<Vec<_>>().join(" and ")
        ),
        None => String::new(),
    }));
    ui.set_kp_bindings_title(s(match &scope {
        Some(h) => format!("Mapped {}", h.title().replacen("While", "while", 1)),
        None => "Mapped in this profile".to_string(),
    }));
    ui.set_kp_sheet_preview_caption(s(match &scope {
        Some(h) => format!("Preview for the selected profile {}, with your unsaved changes:", h.title().replacen("While", "while", 1)),
        None => "Preview for the selected profile, with your unsaved changes:".to_string(),
    }));
    ui.set_kp_adding_profile(st.adding);
    ui.set_kp_app_classes(strings(st.app_classes.clone()));

    // Editor
    let n: usize = st.control.trim_start_matches(char::is_alphabetic).parse().unwrap_or(1);
    let knob = st.control.starts_with("knob");
    let mappable = if knob { n <= st.max_knobs } else { n <= st.max_keys };
    let locked = st.editing_held_control();
    ui.set_kp_selected_title(s(if locked {
        format!("{} (held for this layer)", config::slot_label(&st.control))
    } else if mappable {
        config::slot_label(&st.control)
    } else {
        format!("{} (not supported by the keypad app yet)", config::slot_label(&st.control))
    }));
    ui.set_kp_editor_locked(locked);
    ui.set_kp_selected_knob(knob);
    ui.set_kp_knob_event(st.knob_event as i32);
    let kinds = st.kinds();
    let global = st.global();
    let in_layer = st.held_scope().is_some();
    ui.set_kp_action_names(strings(kinds.iter().map(|k| match (k, global) {
        (ActionKind::Inherit, _) if in_layer => "Not set (its normal binding)".to_string(),
        (ActionKind::Inherit, true) => "Not set".to_string(),
        (k, _) => k.label().to_string(),
    })));
    ui.set_kp_action_index(kinds.iter().position(|k| *k == st.editor.kind).map_or(-1, |i| i as i32));
    let choices = st.choices();
    ui.set_kp_choice_names(strings(choices.iter().map(|(_, l)| l.clone())));
    ui.set_kp_choice_index(choices.iter().position(|(k, _)| *k == st.editor.value).map_or(-1, |i| i as i32));
    let (mode, hint) = match st.editor.kind {
        _ if locked => (0, "You hold this control to use the layer, so it has nothing to do in it. Pick another key or knob."),
        ActionKind::Inherit if in_layer => (0, "While held, this control keeps its normal binding."),
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
    ui.set_kp_label_enabled(mappable && !locked && st.config_error.is_none() && st.editor.takes_label());
    ui.set_kp_needs_placeholder(s(match st.editor.kind {
        ActionKind::Command => format!(
            "Optional, e.g. {}: skip this binding while it isn't installed",
            launched_program(&st.editor.value).unwrap_or("grafium")
        ),
        _ => "Optional: programs or app ids; skip this binding while one isn't installed".to_string(),
    }));
    render_advanced(ui, st, pad);
    let (glyph, caption) = st.icon_caption();
    ui.set_kp_icon_name(s(&st.editor.icon));
    ui.set_kp_icon_glyph(s(glyph));
    ui.set_kp_icon_caption(s(caption));
    ui.set_kp_icon_picker_open(st.icon_picker);
    ui.set_kp_icons_shown(st.icons_shown);

    // Cheatsheet: options and the preview for the selected profile.
    ui.set_kp_sheet_supported(st.sheet);
    let o = st.config.sheet_options(&st.sheet_defaults);
    ui.set_kp_sheet_opacity(o.opacity as f32);
    ui.set_kp_sheet_click_through(o.click_through);
    let mut hide_names: Vec<String> = SHEET_HIDE.iter().map(|(_, l)| l.to_string()).collect();
    let hide_index = SHEET_HIDE.iter().position(|(ms, _)| *ms == o.auto_hide_ms).unwrap_or_else(|| {
        // A value set in the file: shown as is, and kept unless another is chosen.
        hide_names.push(format!("After {} s without keypad input (set in the file)", config::number(f64::from(o.auto_hide_ms) / 1000.0)));
        SHEET_HIDE.len()
    });
    ui.set_kp_sheet_hide_index(hide_index as i32);
    ui.set_kp_sheet_hide_names(strings(hide_names));
    ui.set_kp_sheet_overlay(o.overlay);
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
        .bound_slots_at(st.profile, st.held_index())
        .into_iter()
        .map(|(slot, b)| {
            let unsupported = slot.split('.').nth(1) == Some("shift") && st.shift_supported != Some(true);
            KeypadBindingRow {
                label: s(if unsupported {
                    format!("{} · Turn while pressed", config::slot_label(slot.split('.').next().unwrap_or(&slot)))
                } else {
                    config::slot_label(&slot)
                }),
                summary: s(match b.kind {
                    _ if unsupported => "never fires: the firmware ignores turns while a knob is pressed".to_string(),
                    ActionKind::Advanced => format!("advanced: {}", b.value),
                    _ => b.summary(),
                }),
                advanced: b.kind == ActionKind::Advanced,
                glyph: s(icons::glyph(&st.slot_icon(&slot))),
                warning: unsupported,
                slot: s(slot),
            }
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
    st.leave_layer();
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
        if st.identify && st.picking_held && event.event == "press" {
            selected = true;
            let control = event.control.clone();
            pick_held(st, &control);
            handled = true;
        } else if st.identify {
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
    if st.picking_held {
        pick_held(st, control);
        return;
    }
    st.control = control.to_string();
    if let Some(i) = knob_event_index(event) {
        st.knob_event = i;
    }
    st.load_editor();
}

fn apply_binding(ui: &MainWindow) {
    let text = ui.get_kp_action_text().to_string();
    let label = ui.get_kp_label_text().to_string();
    let needs = ui.get_kp_needs_text().to_string();
    with(|st| {
        if st.config_error.is_some() {
            return;
        }
        let mut binding = st.editor.clone();
        if matches!(binding.kind, ActionKind::Shortcut | ActionKind::Command) {
            binding.value = text.clone();
        }
        binding.label = if binding.takes_label() { label.trim().to_string() } else { String::new() };
        if binding.takes_label() {
            binding = binding.needing(&needs);
        } else {
            binding.icon.clear();
            binding.needs.clear();
        }
        let slot = st.slot();
        match st.config.set_binding_at(st.profile, st.held_index(), &slot, &binding) {
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

/// The program a command line starts, past launcher wrappers
/// ("focus-or-launch nemo nemo" and "gtk-launch nemo" -> "nemo").
fn launched_program(command: &str) -> Option<&str> {
    const WRAPPERS: [&str; 4] = ["focus-or-launch", "gtk-launch", "uwsm-app", "setsid"];
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.first() {
        Some(first) if WRAPPERS.contains(first) => words.last().copied().filter(|w| w != first),
        first => first.copied(),
    }
}

/// "Hold a key…" then a click (or a press while identifying): adds the layer
/// for `control` (or opens the one there is) and edits it.
fn pick_held(st: &mut State, control: &str) {
    st.picking_held = false;
    let before = st.config.held_layers(st.profile).len();
    match st.config.add_held_layer(st.profile, control) {
        Ok(index) => {
            st.layer = Some(index);
            let label = config::slot_label(control);
            if st.config.held_layers(st.profile).len() > before {
                st.dirty = true;
                st.set_message(format!("Added a layer for holding {label}: map what the other keys and knobs do meanwhile, then Save."), false);
            } else {
                st.set_message(format!("This profile already has a layer for holding {label}."), false);
            }
            // Start on a control other than the held one.
            if st.control == control {
                st.control = if control == "key1" { "key2".into() } else { "key1".into() };
            }
            st.load_editor();
        }
        Err(e) => st.set_message(e, true),
    }
}

/// Puts the selected profile's name, window class and title into their fields.
fn sync_profile_fields(ui: &MainWindow) {
    if let Some(p) = with(|st| st.profiles().get(st.profile).cloned()).flatten() {
        ui.set_kp_profile_name(s(&p.name));
        ui.set_kp_profile_class(s(&p.class));
        ui.set_kp_profile_title(s(&p.title));
    }
}

/// Switches the keypad app to keymap input for a firmware update: its raw
/// input would compete with the update tool's own raw session (its
/// flash-and-verify refuses to start otherwise). Returns the mode to put
/// back, or `None` when nothing was switched.
fn wizard_input_begin(st: &mut State) -> Option<String> {
    let in_file = st
        .loaded_text
        .as_deref()
        .and_then(|t| KeypadConfig::parse(t).ok())
        .map_or_else(|| st.config.input_mode(), |c| c.input_mode());
    let change = vec![("input".to_string(), "evdev".to_string())];
    if in_file == "evdev" || !direct_route(st, &change) || st.config.set_input_mode("evdev").is_err() {
        return None;
    }
    commit(st, Some(change), "For the firmware update the keypad app reads the keymap (Keymap input); closing the wizard puts it back.")
        .then_some(in_file)
}

/// Puts back the input mode `wizard_input_begin` switched; true to arm the timer.
fn wizard_input_end(st: &mut State, restore: Option<String>) -> bool {
    let Some(mode) = restore else { return false };
    if st.config.set_input_mode(&mode).is_err() {
        return false;
    }
    let label = config::INPUT_MODES.iter().find(|(m, _)| *m == mode).map_or(mode.as_str(), |(_, l)| l);
    commit(st, Some(vec![("input".into(), mode.clone())]), &format!("Input mode back to {label} after the firmware wizard."))
}

/// What an input mode does, for the line under the mode buttons.
fn input_help(st: &State, mode: &str, firmware: Option<(&str, &str)>) -> String {
    let text = match mode {
        "evdev" => "Reads the keys the keypad types (its keymap). Works with every firmware; a few key pairs that share a key code can't be held together.".to_string(),
        "raw" => "Reads the open firmware's own events, with snapshots that restore anything lost: fastest, and any keys can be held together.".to_string(),
        _ => {
            let now = st.input_modes.iter().find(|(m, _)| m == "auto").map(|(_, d)| d.as_str());
            match now {
                Some(d) => format!("The keypad app chooses; right now it says: {d}."),
                None => "The keypad app chooses: raw on the open firmware 2.0.2 or newer, the keymap otherwise.".to_string(),
            }
        }
    };
    let caution = match (mode, firmware) {
        ("raw", Some((kind, _))) if kind != "control-surface" => {
            " This keypad doesn't have the open firmware, so the keymap stays in use (install it under Firmware below)."
                .to_string()
        }
        ("raw", Some((_, version))) => match config::version_at_least(version, config::RAW_MIN_FIRMWARE) {
            Some(false) => format!(" This keypad has firmware {version}: raw needs 2.0.2 or newer, so the keymap stays in use."),
            None => " Raw needs firmware 2.0.2 or newer; with older firmware the keymap stays in use.".to_string(),
            Some(true) => String::new(),
        },
        _ => String::new(),
    };
    format!("{text}{caution}")
}

/// Keypad > Advanced: input mode and its health, engine tuning, overlay.
fn render_advanced(ui: &MainWindow, st: &State, pad: Option<&super::Pad>) {
    let mode = st.config.input_mode();
    ui.set_kp_input_index(config::INPUT_MODES.iter().position(|(m, _)| *m == mode).map_or(0, |i| i as i32));
    let firmware = pad.map(|p| (p.firmware.as_str(), p.version.as_str()));
    ui.set_kp_input_help(s(input_help(st, &mode, firmware)));
    let (now, health, healthy) = super::input_summary(st.status.input.as_ref(), st.status.app.running(), firmware, &mode);
    ui.set_kp_input_now(s(now));
    ui.set_kp_input_health(s(health));
    ui.set_kp_input_healthy(healthy);
    let rows = |kdenlive: bool| -> Vec<KeypadTuning> {
        config::TUNINGS
            .iter()
            .filter(|t| t.kdenlive == kdenlive)
            .map(|t| {
                let set = st.config.tuning(t.key);
                let v = set.unwrap_or(t.default);
                let off = if t.key == "accelFactor" && v <= 1.0 { " (off)" } else { "" };
                let default = if set.is_none() { " (default)" } else { "" };
                KeypadTuning {
                    key: s(t.key),
                    label: s(t.label),
                    help: s(t.help),
                    min: t.min as f32,
                    max: t.max as f32,
                    step: t.step as f32,
                    value: v as f32,
                    display: s(format!("{}{}{off}{default}", config::number(v), t.unit)),
                }
            })
            .collect()
    };
    ui.set_kp_knob_tunings(ModelRc::from(Rc::new(VecModel::from(rows(false)))));
    ui.set_kp_kdenlive_tunings(ModelRc::from(Rc::new(VecModel::from(rows(true)))));
    ui.set_kp_tunings_custom(config::TUNINGS.iter().any(|t| st.config.tuning(t.key).is_some()));
}

/// Fills the icon picker with the icons matching `query`.
fn show_icons(ui: &MainWindow, query: &str) {
    let found: Vec<KeypadIcon> =
        icons::search(query).into_iter().map(|i| KeypadIcon { name: s(&i.name), glyph: s(&i.glyph) }).collect();
    ui.set_kp_icon_results(ModelRc::from(Rc::new(VecModel::from(found))));
}

/// Puts the editor's value into the text field (only when the selection changes).
fn sync_editor_text(ui: &MainWindow) {
    if let Some((text, label, needs)) = with(|st| {
        let text = match st.editor.kind {
            ActionKind::Shortcut | ActionKind::Command => st.editor.value.clone(),
            _ => String::new(),
        };
        (text, st.editor.label.clone(), st.editor.needs.join(" "))
    }) {
        ui.set_kp_action_text(s(text));
        ui.set_kp_label_text(s(label));
        ui.set_kp_needs_text(s(needs));
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
                    st.sheet_defaults = f.sheet_defaults;
                    st.icons_shown = f.icons;
                    st.input_modes = f.input_modes;
                    st.direct_options = f.options.into_iter().collect();
                    st.shift_supported = f.shift_supported;
                    st.held_supported = f.held_layers;
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

impl State {
    fn new() -> Self {
        Self {
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
            sheet_defaults: config::SheetDefaults::default(),
            icons_shown: false,
            input_modes: Vec::new(),
            direct_options: HashSet::new(),
            pending_options: Vec::new(),
            shift_supported: None,
            held_supported: false,
            layer: None,
            picking_held: false,
            shift_key: 0,
            icon_picker: false,
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
        }
    }
}

pub fn install(ui: &MainWindow) -> KeypadTab {
    let state = State::new();
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
            set_sheet_option(&ui, |o| {
                o.auto_hide_ms = *ms;
                // "Until hidden" needs a sheet that a click can close.
                o.click_through &= *ms > 0;
            });
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_click_through(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        let fallback = with(|st| st.sheet_defaults.auto_hide_ms).filter(|ms| *ms > 0).unwrap_or(SHEET_SHOW_PREVIEW_MS);
        set_sheet_option(&ui, |o| {
            o.click_through = on;
            if on && o.auto_hide_ms == 0 {
                o.auto_hide_ms = fallback;
            }
        });
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
        // The daemon shows the saved config's sheet; a saved "until hidden"
        // would leave this preview up, so Settings takes it down itself.
        let saved = with(|st| {
            let defaults = st.sheet_defaults;
            st.loaded_text
                .as_deref()
                .and_then(|t| KeypadConfig::parse(t).ok())
                .map_or(defaults.auto_hide_ms, |c| c.sheet_options(&defaults).auto_hide_ms)
        })
        .unwrap_or(0);
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
                        (Ok(()), _) if saved == 0 => {
                            Timer::single_shot(Duration::from_millis(SHEET_SHOW_PREVIEW_MS.into()), || {
                                std::thread::spawn(|| {
                                    let _ = super::hide_sheet();
                                });
                            });
                            (
                                format!(
                                    "Cheatsheet shown for {} s. Your saved setting keeps it up until it's clicked or its key is pressed.",
                                    SHEET_SHOW_PREVIEW_MS / 1000
                                ),
                                false,
                            )
                        }
                        (Ok(()), _) => (
                            format!("Cheatsheet shown on screen. It hides after {} s (the saved setting).", saved / 1000),
                            false,
                        ),
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
            st.leave_layer();
            st.adding = false;
            st.load_editor();
        });
        sync_profile_fields(&ui);
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
                    st.leave_layer();
                    st.adding = false;
                    st.dirty = true;
                    st.load_editor();
                    st.set_message(format!("Added a profile for {class}. Map its controls, then Save."), false);
                }
                Err(e) => st.set_message(e, true),
            }
        });
        sync_profile_fields(&ui);
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
                    st.leave_layer();
                    st.dirty = true;
                    st.load_editor();
                    st.set_message(format!("Removed the {name} profile. Not saved yet."), false);
                }
                Err(e) => st.set_message(e, true),
            }
        });
        sync_profile_fields(&ui);
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_apply_profile_details(move || {
        let Some(ui) = weak.upgrade() else { return };
        let (name, class, title) =
            (ui.get_kp_profile_name().to_string(), ui.get_kp_profile_class().to_string(), ui.get_kp_profile_title().to_string());
        with(|st| {
            let Some(p) = st.profiles().get(st.profile).cloned() else { return };
            let mut result = Ok(());
            if name.trim() != p.name {
                result = st.config.set_profile_name(st.profile, &name);
            }
            if result.is_ok() && !p.global && class.trim() != p.class {
                result = st.config.set_profile_class(st.profile, &class);
            }
            if result.is_ok() && !p.global && title.trim() != p.title {
                result = st.config.set_profile_title(st.profile, &title);
            }
            match result {
                Ok(()) if st.profiles().get(st.profile) != Some(&p) => {
                    st.dirty = true;
                    st.set_message("Profile updated. Not saved yet.", false);
                }
                Ok(()) => {}
                Err(e) => st.set_message(e, true),
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_remove_shift(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let n = st.config.remove_shift_bindings(st.profile);
            if n > 0 {
                st.dirty = true;
                st.load_editor();
                st.set_message(
                    format!("Removed turn-while-pressed bindings from {n} knob(s); their presses fire at once again. Not saved yet."),
                    false,
                );
            }
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_select_layer(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.picking_held = false;
            st.layer = match i {
                i if i <= 0 => None,
                i => st.config.held_layers(st.profile).get(i as usize - 1).map(|h| h.index),
            };
            st.load_editor();
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_start_held_layer(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.picking_held = true);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_cancel_held_layer(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.picking_held = false);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_remove_layer(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            let Some(h) = st.held_scope() else { return };
            match st.config.remove_layer(st.profile, h.index) {
                Ok(()) => {
                    st.dirty = true;
                    st.leave_layer();
                    st.load_editor();
                    st.set_message(format!("Removed the layer \"{}\". Not saved yet.", h.title()), false);
                }
                Err(e) => st.set_message(e, true),
            }
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_convert_shift(move || {
        let Some(ui) = weak.upgrade() else { return };
        let key = ui.get_kp_shift_key().max(0) as usize;
        with(|st| {
            st.shift_key = key;
            let control = format!("key{}", key + 1);
            match st.config.convert_shift_to_held(st.profile, &control) {
                Ok((0, 0)) => {}
                Ok((0, kept)) => st.set_message(
                    format!(
                        "Nothing moved: the layer for holding {} already maps {kept} of these knob(s). Remove the turn-while-pressed bindings, or pick another key.",
                        config::slot_label(&control)
                    ),
                    false,
                ),
                Ok((moved, kept)) => {
                    st.dirty = true;
                    let label = config::slot_label(&control);
                    let kept = if kept > 0 {
                        format!(" {kept} knob(s) stayed as they were, because the layer for holding {label} maps them already: compare, then Remove them or pick another key.")
                    } else {
                        String::new()
                    };
                    // Show the plain layer it went to, on a knob it maps.
                    st.layer = st.config.held_layers(st.profile).iter().find(|h| h.single() == Some(control.as_str())).map(|h| h.index);
                    if st.editing_held_control() || !st.control.starts_with("knob") {
                        let knob = st.config.bound_slots_at(st.profile, st.held_index()).into_iter()
                            .find_map(|(slot, _)| slot.starts_with("knob").then(|| slot.split('.').next().unwrap_or("knob1").to_string()));
                        st.control = knob.unwrap_or_else(|| "knob1".into());
                    }
                    st.load_editor();
                    st.set_message(
                        format!("Moved {moved} knob(s) to \"hold {label} and turn\". Not saved yet.{kept}"),
                        false,
                    );
                }
                Err(e) => st.set_message(e, true),
            }
        });
        sync_editor_text(&ui);
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_fallthrough(move |on| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            if st.config.set_profile_fallthrough(st.profile, on).is_ok() {
                st.dirty = true;
                st.set_message(
                    if on {
                        "Controls this profile leaves unset now do what Global maps them to. Not saved yet."
                    } else {
                        "Controls this profile leaves unset now do nothing in this app. Not saved yet."
                    },
                    false,
                );
            }
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_input_mode(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        let armed = with(|st| {
            let Some((mode, label)) = config::INPUT_MODES.get(i.max(0) as usize) else { return false };
            if *mode == st.config.input_mode() {
                return false;
            }
            match st.config.set_input_mode(mode) {
                Ok(()) => commit(
                    st,
                    Some(vec![("input".into(), mode.to_string())]),
                    &format!("Input mode: {label}. The keypad app switches without a restart."),
                ),
                Err(e) => {
                    st.set_message(e, true);
                    false
                }
            }
        })
        .unwrap_or(false);
        if armed {
            arm_options(&ui);
        }
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_tuning(move |key, value| {
        let Some(ui) = weak.upgrade() else { return };
        let armed = with(|st| {
            let before = st.config.tuning(&key);
            match st.config.set_tuning(&key, Some(f64::from(value))) {
                Ok(()) if st.config.tuning(&key) != before => {
                    let t = config::TUNINGS.iter().find(|t| t.key == key.as_str());
                    let label = t.map_or("Setting", |t| t.label);
                    let unit = t.map_or("", |t| t.unit);
                    let v = st.config.tuning(&key).unwrap_or_default();
                    commit(
                        st,
                        Some(vec![(format!("settings.{key}"), config::number(v))]),
                        &format!("{label}: {}{unit}.", config::number(v)),
                    )
                }
                Ok(()) => false,
                Err(e) => {
                    st.set_message(e, true);
                    false
                }
            }
        })
        .unwrap_or(false);
        if armed {
            arm_options(&ui);
        }
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_reset_tunings(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            for t in config::TUNINGS {
                let _ = st.config.set_tuning(t.key, None);
            }
            st.dirty = true;
            st.set_message("Knob and Kdenlive tuning back to the keypad app's defaults. Not saved yet.", false);
        });
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_set_sheet_overlay(move |on| {
        if let Some(ui) = weak.upgrade() {
            set_sheet_option(&ui, |o| o.overlay = on);
        }
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
                    st.leave_layer();
                    st.dirty = true;
                    st.load_editor();
                    st.set_message("Loaded the recommended Kdenlive layout (with context layers). Not saved yet.", false);
                }
                None => st.set_message("The recommended layout comes with the keypad app, which isn't installed.", true),
            }
        });
        sync_profile_fields(&ui);
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
            let current = st.config.binding_at(st.profile, st.held_index(), &st.slot());
            let (label, icon, needs) = (st.editor.label.clone(), st.editor.icon.clone(), st.editor.needs.join(" "));
            st.editor = if current.kind == kind {
                current
            } else {
                Binding::new(kind, "").labelled(&label).with_icon(&icon).needing(&needs)
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
    ui.on_kp_toggle_icon_picker(move || {
        let Some(ui) = weak.upgrade() else { return };
        let open = with(|st| {
            st.icon_picker = !st.icon_picker;
            st.icon_picker
        })
        .unwrap_or(false);
        if open {
            ui.set_kp_icon_query(SharedString::new());
            show_icons(&ui, "");
        }
        render(&ui);
    });

    let weak = ui.as_weak();
    ui.on_kp_search_icons(move |query| {
        if let Some(ui) = weak.upgrade() {
            show_icons(&ui, &query);
        }
    });

    let weak = ui.as_weak();
    ui.on_kp_pick_icon(move |name| {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| {
            st.editor.icon = name.to_string();
            st.icon_picker = false;
            let what = match name.as_str() {
                "" => "Automatic icon".to_string(),
                ICON_NONE => "No icon".to_string(),
                n => format!("Icon {n}"),
            };
            st.set_message(format!("{what}: Apply to control to use it."), false);
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
        let armed = with(|st| {
            let Some(mut w) = st.wizard.take() else { return false };
            if !wizard_can_next(st, &w) {
                st.wizard = Some(w);
                return false;
            }
            if w.step + 1 >= WIZARD_TITLES.len() {
                let armed = wizard_input_end(st, w.restore_input.take());
                st.set_message(
                    format!("Firmware wizard finished: {} of {} inputs confirmed.", w.seen.len(), st.verify_total()),
                    false,
                );
                return armed;
            }
            w.step += 1;
            w.backed = false;
            // From "Unplug the keypad" on, the keypad app reads the keymap.
            let armed = w.step == 2 && w.restore_input.is_none() && {
                w.restore_input = wizard_input_begin(st);
                w.restore_input.is_some()
            };
            if w.step == 4 {
                w.log.clear();
                w.flash_ok = false;
            }
            w.hint = wizard_hint(st, &w);
            st.wizard = Some(w);
            armed
        })
        .unwrap_or(false);
        if armed {
            arm_options(&ui);
        }
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
        let armed = with(|st| {
            let mut armed = false;
            if st.wizard.as_ref().is_some_and(|w| !w.flashing) {
                let restore = st.wizard.take().and_then(|w| w.restore_input);
                armed = wizard_input_end(st, restore);
            }
            sync_identify(st, true);
            armed
        })
        .unwrap_or(false);
        if armed {
            arm_options(&ui);
        }
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

    /// A state whose keypad app runs on Settings' own config file.
    fn live_state(text: &str, options: &[&str]) -> State {
        let mut st = State::new();
        st.config = KeypadConfig::parse(text).unwrap();
        st.loaded_text = Some(text.to_string());
        st.exists = true;
        st.status.app.bus = true;
        st.status.config_path = Some(super::super::config_path().to_string_lossy().into_owned());
        st.direct_options = options.iter().map(|o| o.to_string()).collect();
        st
    }

    const LIVE: &str = r#"{"device": {"input": "raw"}, "settings": {"keyRateHz": 120}, "profiles": [{"name": "global", "bindings": {}}]}"#;

    #[test]
    fn simple_options_go_to_the_keypad_app_and_the_rest_waits_for_save() {
        let opt = |k: &str, v: &str| vec![(k.to_string(), v.to_string())];
        let mut st = live_state(LIVE, &["input", "settings.keyRateHz"]);
        assert!(commit(&mut st, Some(opt("input", "evdev")), "Input mode."));
        assert!(commit(&mut st, Some(opt("input", "auto")), "Input mode."));
        assert_eq!(st.pending_options, opt("input", "auto"), "one pending value per option, the latest");
        assert!(!st.dirty && st.message.ends_with("Saving…"));
        assert!(!commit(&mut st, Some(opt("settings.gestureIdleMs", "400")), "Edit gesture."), "not settable in place");
        assert!(st.dirty && st.message.ends_with("Not saved yet."));
        assert!(!commit(&mut st, Some(Vec::new()), "nothing"), "nothing changed");

        for (what, mut other) in [
            ("no bus", live_state(LIVE, &["input"])),
            ("another file", live_state(LIVE, &["input"])),
            ("no file yet", live_state(LIVE, &["input"])),
            ("old keypad app", live_state(LIVE, &[])),
        ] {
            match what {
                "no bus" => other.status.app.bus = false,
                "another file" => other.status.config_path = Some("/elsewhere/config.jsonc".into()),
                "no file yet" => other.exists = false,
                _ => {}
            }
            assert!(!commit(&mut other, Some(opt("input", "evdev")), "x"), "{what}");
            assert!(other.dirty && other.pending_options.is_empty(), "{what}: falls back to Save");
        }
        let mut st = live_state(LIVE, &["input"]);
        assert!(!commit(&mut st, None, "Click-through on."), "a window name isn't an option");
        assert!(st.dirty);
    }

    #[test]
    fn sheet_changes_map_to_option_keys() {
        let before = SheetOptions { opacity: 0.35, auto_hide_ms: 8000, position: "center".into(), click_through: false, overlay: true };
        let after = SheetOptions { opacity: 0.5, auto_hide_ms: 0, position: "top-right".into(), overlay: false, ..before.clone() };
        let keys: Vec<String> = sheet_option_changes(&before, &after).into_iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(keys, ["cheatsheet.opacity=0.5", "cheatsheet.autoHideMs=0", "cheatsheet.position=top-right", "cheatsheet.eww=false"]);
        assert!(sheet_option_changes(&before, &before).is_empty());
    }

    #[test]
    fn set_option_results_keep_save_and_the_editor_in_step() {
        use super::super::OptionResult::*;
        let written = LIVE.replace(r#""raw""#, r#""evdev""#);
        let ok = vec![("input".to_string(), Set { changed: true, backup: Some("/c/config.jsonc.bak".into()) })];

        // Clean editor: the written file becomes what Save compares and what the editor shows.
        let mut st = live_state(LIVE, &["input"]);
        st.config.set_input_mode("evdev").unwrap();
        st.config.set_tuning("keyRateHz", Some(300.0)).unwrap();
        option_results(&mut st, &ok, Some(LIVE.into()), Some(LIVE.into()), Some(written.clone()));
        assert_eq!(st.loaded_text.as_deref(), Some(written.as_str()));
        assert_eq!((st.config.input_mode().as_str(), st.config.tuning("keyRateHz")), ("evdev", Some(120.0)), "re-read from the file");
        assert!(!st.dirty && st.message.contains("Input mode: saved and applied") && st.message.contains("config.jsonc.bak"), "{}", st.message);

        // Unsaved edits elsewhere stay; Save's baseline follows the file.
        let mut st = live_state(LIVE, &["input"]);
        st.dirty = true;
        st.config.set_input_mode("evdev").unwrap();
        st.config.set_tuning("keyRateHz", Some(300.0)).unwrap();
        option_results(&mut st, &ok, Some(LIVE.into()), Some(LIVE.into()), Some(written.clone()));
        assert_eq!(st.loaded_text.as_deref(), Some(written.as_str()));
        assert_eq!(st.config.tuning("keyRateHz"), Some(300.0));
        assert!(st.dirty && st.message.contains("other changes are still unsaved"));

        // Someone else changed the file meanwhile: Save must still notice.
        let mut st = live_state(LIVE, &["input"]);
        st.dirty = true;
        option_results(&mut st, &ok, Some(LIVE.into()), Some("{\"edited\": 1}".into()), Some(written.clone()));
        assert_eq!(st.loaded_text.as_deref(), Some(LIVE));

        // What the keypad app couldn't set becomes an ordinary unsaved change.
        for (result, error) in [(Unsupported, false), (Refused("input: auto, evdev or raw".into()), true)] {
            let mut st = live_state(LIVE, &["input"]);
            st.config.set_input_mode("evdev").unwrap();
            option_results(&mut st, &[("input".into(), result)], Some(LIVE.into()), Some(LIVE.into()), Some(LIVE.into()));
            assert!(st.dirty && st.message_is_error == error && st.message.contains("Not saved yet") == error, "{}", st.message);
            assert_eq!(st.config.input_mode(), "evdev", "the change is kept for Save");
        }
        let mut st = live_state(LIVE, &["input"]);
        option_results(&mut st, &[("input".into(), Unapplied("no uinput".into()))], Some(LIVE.into()), Some(LIVE.into()), Some(written.clone()));
        assert!(!st.dirty && st.message_is_error && st.loaded_text.as_deref() == Some(written.as_str()), "written, not applied");
    }

    #[test]
    fn the_firmware_wizard_reads_the_keymap_and_puts_the_mode_back() {
        let mut st = live_state(LIVE, &["input"]);
        assert_eq!(wizard_input_begin(&mut st).as_deref(), Some("raw"));
        assert_eq!(st.pending_options, [("input".to_string(), "evdev".to_string())]);
        assert_eq!(st.config.input_mode(), "evdev");
        assert!(wizard_input_end(&mut st, Some("raw".into())));
        assert_eq!(st.pending_options, [("input".to_string(), "raw".to_string())]);
        assert!(!wizard_input_end(&mut st, None));

        let auto = r#"{"profiles": []}"#;
        let mut st = live_state(auto, &["input"]);
        assert_eq!(wizard_input_begin(&mut st).as_deref(), Some("auto"), "unset means auto, and auto comes back");
        let keymap = r#"{"device": {"input": "evdev"}, "profiles": []}"#;
        assert_eq!(wizard_input_begin(&mut live_state(keymap, &["input"])), None, "already the keymap");
        let mut offline = live_state(LIVE, &["input"]);
        offline.status.app.bus = false;
        assert_eq!(wizard_input_begin(&mut offline), None, "no keypad app reading the pad");
        assert_eq!(offline.config.input_mode(), "raw");
    }

    const HOLDS: &str = r#"{"profiles": [
        {"name": "brave", "match": {"class": "^brave-browser$"}, "bindings": {"key6": "alt+left"}},
        {"name": "global", "bindings": {"key1": {"cheatsheet": "hold"}, "knob1": {"ccw": "volumedown", "cw": "volumeup",
            "shift": {"ccw": "previoussong", "cw": "nextsong"}}}}]}"#;

    #[test]
    fn hold_a_key_picks_on_the_layout_and_edits_that_layer() {
        let mut st = live_state(HOLDS, &[]);
        st.held_supported = true;
        st.profile = 1;
        st.control = "key1".into();
        st.picking_held = true;
        select_slot(&mut st, "key1");
        assert!(!st.picking_held && st.dirty);
        let layer = st.held_scope().expect("editing the new layer");
        assert_eq!(layer.single(), Some("key1"));
        assert_eq!(st.control, "key2", "moved off the held key");
        // Mapping goes into the layer, not the profile.
        st.control = "knob1".into();
        st.knob_event = 0;
        st.load_editor();
        assert_eq!(st.editor.kind, ActionKind::Inherit, "the layer starts empty");
        st.config.set_binding_at(st.profile, st.held_index(), &st.slot(), &Binding::new(ActionKind::Mouse, "wheel-up")).unwrap();
        st.load_editor();
        assert_eq!(st.editor.value, "wheel-up");
        assert_eq!(st.config.binding_at(1, None, "knob1.ccw").value, "volumedown", "normal binding untouched");
        assert_eq!(st.summary("knob1"), "Scroll up | ", "the layout shows the layer's bindings");
        // The held key itself is locked in its layer.
        st.control = "key1".into();
        assert!(st.editing_held_control());
        st.leave_layer();
        assert!(!st.editing_held_control() && st.held_scope().is_none());
        // Picking a control that already has a layer opens it.
        st.picking_held = true;
        select_slot(&mut st, "key1");
        assert_eq!(st.held_scope().map(|h| h.index), Some(layer.index));
        assert!(st.message.contains("already has a layer"));
    }

    #[test]
    fn the_preview_of_a_held_layer_holds_its_key() {
        let mut st = live_state(HOLDS, &[]);
        st.profile = 1;
        assert_eq!(st.sheet_target().1, "");
        st.layer = Some(st.config.add_held_layer(1, "key13").unwrap());
        assert_eq!(st.sheet_target().1, r#"{"$held":"key13"}"#);
    }

    #[test]
    fn if_installed_suggests_the_launched_program() {
        assert_eq!(launched_program("focus-or-launch nemo nemo"), Some("nemo"));
        assert_eq!(launched_program("gtk-launch grafium"), Some("grafium"));
        assert_eq!(launched_program("smplos-settings"), Some("smplos-settings"));
        assert_eq!(launched_program("gtk-launch"), None);
        assert_eq!(launched_program(""), None);
    }

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
