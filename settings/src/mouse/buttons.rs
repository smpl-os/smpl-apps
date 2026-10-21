//! The Buttons card of the Mouse tab, with no Slint in it: which mouse is
//! shown, which button is selected, which profile is being edited and what
//! each edit does to `mouse.json`.
//!
//! There is no Save button. An edit is applied to a copy of the assignments,
//! the file is written, and only then does the page say "Saved". When the
//! file refuses the write (somebody else changed it, the disk said no) the
//! edit is dropped and the file is read again, so the page never shows what
//! isn't on disk. A file this Settings can't safely edit is never written:
//! the page stays read-only until it is fixed and refreshed.
//!
//! `ui` copies a [`View`] into the window's properties.

use std::path::PathBuf;

use super::config::MouseConfig;
use super::devices::{Device, Input, Rect};
use super::store::{self, Backups};
use crate::assign::action::{ActionKind, Binding, ICON_NONE};
use crate::assign::flow::{self, Done};
use crate::assign::picker::{self, Wording};
use crate::assign::profile::{ProfileInfo, Profiles};
use crate::assign::Saving;
use crate::keypad::icons;

/// What "not set" means on a mouse: a button nobody assigned keeps its normal job.
pub const WORDING: Wording = Wording {
    unset_global: "Not set (normal button)",
    hint_unset_global: "Unassigned: the button does its normal job.",
    hint_unset_app: "Uses the Global assignment.",
    hint_advanced: "Written in a form this editor can't show. Edit mouse.json, or pick another action to replace it.",
};

const SAVING: Saving = Saving::Immediate;

fn unset() -> Binding {
    Binding::new(ActionKind::Inherit, "")
}

/// A labelled outline on the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelView {
    pub label: String,
    pub rect: Rect,
}

/// One button on the picture.
#[derive(Clone, Debug, PartialEq)]
pub struct ButtonView {
    pub id: String,
    pub caption: String,
    /// What it does in the edited profile (or, dimmed, what Global makes it do).
    pub summary: String,
    pub rect: Rect,
    pub selected: bool,
    /// The edited profile sets it.
    pub assigned: bool,
    /// It doesn't, but the profile falls through to a Global assignment.
    pub inherited: bool,
}

/// One line of "what this profile assigns".
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub slot: String,
    pub label: String,
    pub summary: String,
    pub advanced: bool,
    pub glyph: String,
    pub warning: bool,
}

/// Everything the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// A button is selected and the file can be edited.
    pub editing: bool,
    pub device_names: Vec<String>,
    pub device_index: i32,
    pub device_note: String,
    pub aspect: f32,
    pub panels: Vec<PanelView>,
    pub buttons: Vec<ButtonView>,
    pub profile_names: Vec<String>,
    pub profile_index: i32,
    pub profile_global: bool,
    pub fallthrough: bool,
    pub adding: bool,
    pub app_classes: Vec<String>,
    pub selected_title: String,
    pub selected_note: String,
    pub action_names: Vec<String>,
    pub action_index: i32,
    pub choice_names: Vec<String>,
    pub choice_index: i32,
    pub editor_mode: i32,
    pub editor_hint: String,
    pub advanced_text: String,
    pub rows_title: String,
    pub rows: Vec<RowView>,
    pub status: String,
    pub status_is_error: bool,
    pub path: String,
}

pub struct Buttons {
    devices: Vec<Device>,
    path: PathBuf,
    config: MouseConfig,
    /// The file as last read or written: what the next write expects to find.
    on_disk: Option<String>,
    /// Why the file can't be edited; the page is read-only while it is set.
    blocked: Option<String>,
    device: usize,
    input: Option<String>,
    profile: usize,
    editor: Binding,
    adding: bool,
    app_classes: Vec<String>,
    message: String,
    message_is_error: bool,
}

impl Buttons {
    /// Reads `path`. The first device that has assignments in it is shown,
    /// else the first device.
    pub fn load(devices: Vec<Device>, path: PathBuf) -> Self {
        let mut buttons = Buttons {
            devices,
            path,
            config: MouseConfig::empty(),
            on_disk: None,
            blocked: None,
            device: 0,
            input: None,
            profile: 0,
            editor: unset(),
            adding: false,
            app_classes: Vec::new(),
            message: String::new(),
            message_is_error: false,
        };
        buttons.read_file();
        buttons.device = buttons.devices.iter().position(|d| buttons.config.has_assignments(&d.id)).unwrap_or(0);
        buttons.enter_device();
        buttons
    }

    // ── Files ────────────────────────────────────────────────────────────────

    /// Reads the file. One this Settings can't safely edit leaves the page
    /// read-only (`blocked`) and is never written.
    fn read_file(&mut self) {
        self.config = MouseConfig::empty();
        self.on_disk = None;
        self.blocked = None;
        match store::read(&self.path) {
            Ok(None) => {}
            Ok(Some(text)) => match MouseConfig::parse(&text) {
                Ok(config) => {
                    self.config = config;
                    self.on_disk = Some(text);
                }
                Err(error) => {
                    self.blocked = Some(format!(
                        "{} can't be used: {error}. Settings won't overwrite it; fix it and press Refresh.",
                        self.path.display()
                    ));
                }
            },
            Err(error) => self.blocked = Some(format!("{error}. Settings won't write until it can read the file.")),
        }
    }

    /// Reads the file again, keeping the device and button, and the profile
    /// when the file still has it (else Global).
    fn reload(&mut self) {
        let kept = self.config.profiles().get(self.profile).map(|p| (p.global, p.class.clone()));
        self.read_file();
        self.adding = false;
        if let Some(device) = self.devices.get(self.device) {
            let global = self.config.select_device(&device.id);
            self.profile = kept
                .and_then(|(is_global, class)| {
                    self.config.profiles().iter().position(|p| p.global == is_global && p.class == class)
                })
                .unwrap_or(global);
        }
        self.load_editor();
    }

    pub fn refresh(&mut self) {
        self.reload();
        self.say("Read the assignments from the file again.", false);
    }

    /// Reads the file again without a word about it: the tab was opened again.
    pub fn reread(&mut self) {
        self.reload();
        self.message.clear();
    }

    /// Runs one edit on a copy of the assignments and keeps it only when the
    /// file takes it. `edit` gets the assignments and the edited profile and
    /// says what to tell the user (`None`: it changed nothing). True when
    /// the edit is saved.
    fn change(&mut self, edit: impl FnOnce(&mut MouseConfig, usize) -> Result<Option<Done>, String>) -> bool {
        if !self.editable() {
            return false;
        }
        let before = self.config.clone();
        let done = match edit(&mut self.config, self.profile) {
            Ok(Some(done)) => done,
            Ok(None) => {
                self.config = before;
                return false;
            }
            Err(error) => {
                self.config = before;
                self.say(error, true);
                return false;
            }
        };
        let text = self.config.render();
        match store::save(&self.path, &text, self.on_disk.as_deref(), Backups::Keep) {
            Ok(()) => {
                self.on_disk = Some(text);
                if let Some(index) = done.select {
                    self.profile = index;
                }
                self.load_editor();
                self.say(done.message, false);
                true
            }
            Err(error) => {
                self.reload();
                self.say(error, true);
                false
            }
        }
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.message = text.into();
        self.message_is_error = error;
    }

    // ── Selection ────────────────────────────────────────────────────────────

    fn current_device(&self) -> Option<&Device> {
        self.devices.get(self.device)
    }

    fn current_input(&self) -> Option<&Input> {
        self.current_device()?.input(self.input.as_deref()?)
    }

    fn editable(&self) -> bool {
        self.blocked.is_none() && self.current_device().is_some()
    }

    /// Points the assignments at the shown device and starts on its Global
    /// profile with no button selected.
    fn enter_device(&mut self) {
        self.profile = match self.devices.get(self.device) {
            Some(device) => self.config.select_device(&device.id),
            None => 0,
        };
        self.input = None;
        self.adding = false;
        self.load_editor();
    }

    pub fn select_device(&mut self, index: usize) {
        if index >= self.devices.len() {
            return;
        }
        self.device = index;
        self.enter_device();
        self.message.clear();
    }

    pub fn select_input(&mut self, id: &str) {
        if self.current_device().and_then(|d| d.input(id)).is_none() {
            return;
        }
        self.input = Some(id.to_string());
        self.load_editor();
        self.message.clear();
    }

    pub fn select_profile(&mut self, index: usize) {
        if index >= self.config.profiles().len() {
            return;
        }
        self.profile = index;
        self.adding = false;
        self.load_editor();
        self.message.clear();
    }

    // ── The editor ───────────────────────────────────────────────────────────

    /// The selected button's saved assignment in the edited profile.
    fn saved_binding(&self) -> Binding {
        match &self.input {
            Some(id) => self.config.flat_binding(self.profile, id),
            None => unset(),
        }
    }

    fn load_editor(&mut self) {
        self.editor = self.saved_binding();
    }

    /// The kinds of action the selected button accepts, plus the one it has
    /// now if the file holds one the device doesn't offer.
    pub fn kinds(&self) -> Vec<ActionKind> {
        let (Some(device), Some(input)) = (self.current_device(), self.current_input()) else {
            return Vec::new();
        };
        let mut kinds = device.kinds_for(input).to_vec();
        if !kinds.contains(&self.editor.kind) {
            kinds.push(self.editor.kind);
        }
        kinds
    }

    fn choices(&self) -> Vec<picker::Choice> {
        picker::choices(&self.editor, &[])
    }

    pub fn select_action(&mut self, index: usize) {
        let Some(kind) = self.kinds().get(index).copied() else { return };
        if let Some(editor) = picker::change_kind(self.saved_binding(), &self.editor, kind) {
            self.editor = editor;
            let choices = self.choices();
            picker::default_choice(&mut self.editor, &choices);
        }
    }

    pub fn select_choice(&mut self, index: usize) {
        if let Some((value, _)) = self.choices().get(index) {
            self.editor.value = value.clone();
        }
    }

    /// What the text box holds for the editor: only shortcuts and commands
    /// are typed.
    pub fn editor_text(&self) -> String {
        match self.editor.kind {
            ActionKind::Shortcut | ActionKind::Command => self.editor.value.clone(),
            _ => String::new(),
        }
    }

    /// Assigns the editor's action (with the text typed into its box) to the
    /// selected button of the edited profile. True when it was saved.
    pub fn apply(&mut self, text: &str) -> bool {
        let Some(id) = self.input.clone() else { return false };
        let label = self.current_input().map(|i| i.label.clone()).unwrap_or_else(|| id.clone());
        let binding = picker::compose_binding(&self.editor, text, &self.editor.label, &self.editor.needs.join(" "));
        let message = picker::assigned_message(&label, &binding, None, SAVING);
        self.change(|config, profile| {
            config.set_flat_binding(profile, &id, &binding)?;
            Ok(Some(Done { select: None, message }))
        })
    }

    // ── Profiles ─────────────────────────────────────────────────────────────

    /// The name, window class and window title of the edited profile, for the
    /// profile fields.
    pub fn profile_fields(&self) -> (String, String, String) {
        self.config
            .profiles()
            .get(self.profile)
            .map(|p| (p.name.clone(), p.class.clone(), p.title.clone()))
            .unwrap_or_default()
    }

    pub fn start_add(&mut self) {
        self.adding = true;
    }

    pub fn cancel_add(&mut self) {
        self.adding = false;
    }

    /// The classes of the open windows, for the "Add app" picker.
    pub fn set_app_classes(&mut self, classes: Vec<String>) {
        self.app_classes = classes;
    }

    pub fn app_class(&self, index: usize) -> Option<&str> {
        self.app_classes.get(index).map(String::as_str)
    }

    pub fn confirm_add(&mut self, class: &str) {
        if self.change(|config, _| flow::add_app(config, class, SAVING).map(Some)) {
            self.adding = false;
        }
    }

    pub fn remove_profile(&mut self) {
        self.change(|config, profile| flow::remove(config, profile, SAVING).map(Some));
    }

    pub fn apply_details(&mut self, name: &str, class: &str, title: &str) {
        self.change(|config, profile| flow::apply_details(config, profile, name, class, title, SAVING));
    }

    pub fn set_fallthrough(&mut self, on: bool) {
        self.change(|config, profile| Ok(flow::set_fallthrough(config, profile, on, SAVING)));
    }

    // ── What the page shows ──────────────────────────────────────────────────

    pub fn view(&self) -> View {
        let device = self.current_device();
        let input = self.current_input();
        let profiles = self.config.profiles();
        let current = profiles.get(self.profile);
        let global = current.is_none_or(|p| p.global);
        let editable = self.editable();
        let kinds = self.kinds();
        let choices = self.choices();

        let (editor_mode, editor_hint) = if !editable {
            (picker::MODE_NONE, "Fix the file first: the message above says what is wrong.".to_string())
        } else if input.is_none() {
            (picker::MODE_NONE, "Pick a button on the picture first.".to_string())
        } else {
            let (mode, hint) = picker::describe(self.editor.kind, global, &WORDING);
            (mode, hint.to_string())
        };
        let (status, status_is_error) = match &self.blocked {
            Some(why) => (why.clone(), true),
            None => (self.message.clone(), self.message_is_error),
        };

        View {
            editing: editable && input.is_some(),
            device_names: self.devices.iter().map(Device::title).collect(),
            device_index: if device.is_some() { self.device as i32 } else { -1 },
            device_note: device.map(|d| d.description.clone()).unwrap_or_default(),
            aspect: device.map_or(1.6, |d| d.aspect),
            panels: device
                .map(|d| d.panels.iter().map(|p| PanelView { label: p.label.clone(), rect: p.rect }).collect())
                .unwrap_or_default(),
            buttons: device.map(|d| self.button_views(d, &profiles)).unwrap_or_default(),
            profile_names: profiles.iter().map(ProfileInfo::title).collect(),
            profile_index: if current.is_some() { self.profile as i32 } else { -1 },
            profile_global: global,
            fallthrough: current.is_none_or(|p| p.fallthrough),
            adding: self.adding,
            app_classes: self.app_classes.clone(),
            selected_title: input.map_or_else(|| "No button selected".to_string(), |i| i.label.clone()),
            selected_note: input.map(input_note).unwrap_or_default(),
            action_names: kinds.iter().map(|k| picker::action_name(*k, global, &WORDING)).collect(),
            action_index: kinds.iter().position(|k| *k == self.editor.kind).map_or(-1, |i| i as i32),
            choice_names: choices.iter().map(|(_, label)| label.clone()).collect(),
            choice_index: choices.iter().position(|(value, _)| *value == self.editor.value).map_or(-1, |i| i as i32),
            editor_mode,
            editor_hint,
            advanced_text: if self.editor.kind == ActionKind::Advanced { self.editor.value.clone() } else { String::new() },
            rows_title: format!(
                "Assigned in {}",
                current.map_or_else(String::new, |p| if p.global { "Global".to_string() } else { p.name.clone() })
            ),
            rows: device.map(|d| self.row_views(d)).unwrap_or_default(),
            status,
            status_is_error,
            path: self.path.display().to_string(),
        }
    }

    /// The assignments a profile sets, without the "not set" entries.
    fn assigned_in(&self, profile: usize) -> Vec<(String, Binding)> {
        self.config.flat_bindings(profile).into_iter().filter(|(_, b)| b.kind != ActionKind::Inherit).collect()
    }

    fn button_views(&self, device: &Device, profiles: &[ProfileInfo]) -> Vec<ButtonView> {
        let own = self.assigned_in(self.profile);
        let falls_through = profiles.get(self.profile).is_some_and(|p| !p.global && p.fallthrough);
        let global = match profiles.iter().position(|p| p.global) {
            Some(index) if falls_through => self.assigned_in(index),
            _ => Vec::new(),
        };
        let find = |list: &[(String, Binding)], id: &str| {
            list.iter().find(|(slot, _)| slot == id).map(|(_, binding)| binding.clone())
        };
        device
            .inputs
            .iter()
            .map(|input| {
                let mine = find(&own, &input.id);
                let inherited = if mine.is_none() { find(&global, &input.id) } else { None };
                ButtonView {
                    id: input.id.clone(),
                    caption: input.caption().to_string(),
                    summary: mine.as_ref().or(inherited.as_ref()).map(Binding::short).unwrap_or_default(),
                    rect: input.rect,
                    selected: self.input.as_deref() == Some(input.id.as_str()),
                    assigned: mine.is_some(),
                    inherited: inherited.is_some(),
                }
            })
            .collect()
    }

    /// The edited profile's assignments in the order of the picture; a
    /// button the file names but the device lacks comes last, flagged.
    fn row_views(&self, device: &Device) -> Vec<RowView> {
        let mut assigned = self.assigned_in(self.profile);
        assigned.sort_by_key(|(slot, _)| device.inputs.iter().position(|i| i.id == *slot).unwrap_or(usize::MAX));
        assigned
            .into_iter()
            .map(|(slot, binding)| {
                let known = device.input(&slot);
                let summary = match binding.kind {
                    ActionKind::Advanced => format!("advanced: {}", binding.value),
                    _ => binding.summary(),
                };
                RowView {
                    label: known.map_or_else(|| slot.clone(), |i| i.label.clone()),
                    summary: if known.is_some() { summary } else { format!("{summary} (not a button of this mouse)") },
                    advanced: binding.kind == ActionKind::Advanced,
                    glyph: glyph_for(&binding),
                    warning: known.is_none(),
                    slot,
                }
            })
            .collect()
    }
}

/// The note a button's profile carries, then what is known of its code.
fn input_note(input: &Input) -> String {
    let code = match (&input.code, input.source) {
        (Some(code), Some(source)) => format!("Code: {code} · {}", source.label()),
        (Some(code), None) => format!("Code: {code}"),
        (None, _) => "Code: not captured yet".to_string(),
    };
    if input.note.is_empty() {
        code
    } else {
        format!("{}\n{code}", input.note)
    }
}

/// The icon a row shows: the binding's own, else one for its kind.
fn glyph_for(binding: &Binding) -> String {
    let name = match binding.kind {
        _ if binding.icon == ICON_NONE => "",
        _ if !binding.icon.is_empty() => binding.icon.as_str(),
        ActionKind::Shortcut => "keyboard",
        ActionKind::Command => "terminal-2",
        ActionKind::Media => "player-play",
        ActionKind::Mouse => "mouse",
        _ => "",
    };
    icons::glyph(name).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assign::flow::parse_window_classes;
    use crate::assign::picker::{MODE_ADVANCED, MODE_CHOICE, MODE_NONE, MODE_TEXT};
    use crate::mouse::devices;
    use crate::mouse::testing::temp_dir;

    /// The page on a file in a folder of its own, holding `file` if given.
    fn page(name: &str, file: Option<&str>) -> (Buttons, PathBuf) {
        let path = temp_dir(name).join("smplos/mouse.json");
        if let Some(text) = file {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        (Buttons::load(devices::builtin(), path.clone()), path)
    }

    fn saved(path: &PathBuf) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn choose(page: &mut Buttons, kind: ActionKind) {
        let index = page.kinds().iter().position(|k| *k == kind).expect("the kind is offered");
        page.select_action(index);
    }

    fn button<'a>(view: &'a View, id: &str) -> &'a ButtonView {
        view.buttons.iter().find(|b| b.id == id).expect("a button")
    }

    #[test]
    fn it_opens_on_the_first_device_with_its_global_profile_and_nothing_selected() {
        let (page, path) = page("opens", None);
        let view = page.view();
        assert!(view.device_names[0].starts_with("Generic mouse"));
        assert_eq!(view.device_index, 0);
        assert_eq!(view.profile_names, ["global (all other apps)"]);
        assert!(view.profile_global && view.fallthrough);
        assert!(!view.editing);
        assert!(view.buttons.iter().all(|b| !b.selected && !b.assigned && !b.inherited));
        assert_eq!(view.selected_title, "No button selected");
        assert_eq!(view.editor_mode, MODE_NONE);
        assert!(view.rows.is_empty());
        assert_eq!((view.status.as_str(), view.status_is_error), ("", false));
        assert!(!path.exists(), "looking at the page writes nothing");
    }

    #[test]
    fn it_opens_on_the_device_that_has_assignments() {
        let text = r#"{"devices":{"mmo-12":{"profiles":[{"name":"global","bindings":{"grid-1":"ctrl+c"}}]}}}"#;
        let (page, _) = page("opens-on-saved", Some(text));
        let view = page.view();
        assert_eq!(view.device_index, 1);
        assert_eq!(view.buttons.len(), 12 + 8, "the MMO mouse's own buttons: the grid and the body");
        assert_eq!(button(&view, "grid-1").summary, "ctrl+c");
        assert_eq!(view.rows.len(), 1);
    }

    #[test]
    fn assigning_a_button_is_written_at_once() {
        let (mut page, path) = page("assign", None);
        page.select_input("btn-back");
        assert!(page.view().editing);
        choose(&mut page, ActionKind::Shortcut);
        assert_eq!(page.view().editor_mode, MODE_TEXT);
        page.apply("alt+left");

        let view = page.view();
        assert_eq!((view.status.as_str(), view.status_is_error), ("Back button → alt+left. Saved.", false));
        assert!(saved(&path).contains(r#""btn-back": "alt+left""#), "{}", saved(&path));
        assert!(button(&view, "btn-back").assigned);
        assert_eq!(button(&view, "btn-back").summary, "alt+left");
        assert_eq!(view.rows.len(), 1);
        assert_eq!(view.rows[0].slot, "btn-back");
        assert_eq!(view.rows_title, "Assigned in Global");
        assert_eq!(page.editor_text(), "alt+left");

        let again = Buttons::load(devices::builtin(), path);
        assert_eq!(button(&again.view(), "btn-back").summary, "alt+left", "it survives a restart");
    }

    #[test]
    fn a_choice_kind_starts_on_its_first_choice_and_can_be_changed() {
        let (mut page, path) = page("choices", None);
        page.select_input("btn-forward");
        choose(&mut page, ActionKind::Media);
        let view = page.view();
        assert_eq!(view.editor_mode, MODE_CHOICE);
        assert_eq!(view.choice_names[0], "Play / pause");
        assert_eq!(view.choice_index, 0);

        page.select_choice(5);
        page.apply("");
        assert!(saved(&path).contains(r#""btn-forward": {"#) || saved(&path).contains("btn-forward"));
        let view = page.view();
        assert_eq!(view.rows[0].summary, "Volume down");
        assert_eq!(view.status, "Forward button → Volume down. Saved.");
    }

    #[test]
    fn setting_a_button_back_to_not_set_removes_it_from_the_file() {
        let (mut page, path) = page("unset", None);
        page.select_input("btn-middle");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("ctrl+v");
        assert!(saved(&path).contains("btn-middle"));

        choose(&mut page, ActionKind::Inherit);
        page.apply("");
        assert!(!saved(&path).contains("btn-middle"));
        assert_eq!(page.view().status, "Wheel click → not set. Saved.");
        assert!(page.view().rows.is_empty());
    }

    #[test]
    fn the_primary_buttons_only_offer_swapping() {
        let (mut page, _) = page("primary", None);
        for id in ["btn-left", "btn-right"] {
            page.select_input(id);
            assert_eq!(page.kinds(), [ActionKind::Inherit, ActionKind::Mouse], "{id}");
            let names = page.view().action_names;
            assert_eq!(names, ["Not set (normal button)", "Mouse button"], "{id}");
        }
        page.select_input("btn-back");
        assert_eq!(page.kinds().len(), 6);
    }

    #[test]
    fn a_hand_written_assignment_a_button_does_not_offer_is_still_shown() {
        let text = r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{"btn-left":"ctrl+c"}}]}}}"#;
        let (mut page, _) = page("hand-written", Some(text));
        page.select_input("btn-left");
        assert_eq!(page.kinds(), [ActionKind::Inherit, ActionKind::Mouse, ActionKind::Shortcut]);
        let view = page.view();
        assert_eq!(view.action_index, 2, "so it can be seen and put back");
        choose(&mut page, ActionKind::Inherit);
        page.apply("");
        assert_eq!(page.kinds(), [ActionKind::Inherit, ActionKind::Mouse]);
    }

    #[test]
    fn every_device_keeps_its_own_assignments() {
        let (mut page, path) = page("per-device", None);
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+left");

        page.select_device(1);
        let view = page.view();
        assert_eq!(view.device_index, 1);
        assert!(view.rows.is_empty(), "another mouse starts blank");
        assert!(!view.editing, "and with nothing selected");
        page.select_input("grid-7");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("ctrl+c");

        page.select_device(0);
        assert_eq!(page.view().rows.len(), 1);
        assert_eq!(page.view().rows[0].slot, "btn-back");
        let file = saved(&path);
        assert!(file.contains("alt+left") && file.contains("ctrl+c"), "{file}");
    }

    #[test]
    fn an_app_profile_is_added_edited_and_removed_with_every_step_saved() {
        let (mut page, path) = page("app-profile", None);
        page.set_app_classes(parse_window_classes(br#"[{"class":"firefox"},{"class":"settings"},{"class":"firefox"}]"#));
        assert_eq!(page.app_class(0), Some("firefox"));
        assert_eq!(page.app_class(1), None, "Settings itself is left out");

        page.start_add();
        assert!(page.view().adding);
        page.confirm_add("firefox");
        let view = page.view();
        assert!(!view.adding);
        assert_eq!(view.profile_names, ["firefox", "global (all other apps)"]);
        assert_eq!(view.profile_index, 0);
        assert!(!view.profile_global);
        assert_eq!(view.status, "Added a profile for firefox. Pick a button to assign it.");
        assert!(saved(&path).contains(r#""class": "^firefox$""#), "{}", saved(&path));
        assert_eq!(page.profile_fields(), ("firefox".into(), "^firefox$".into(), String::new()));

        page.apply_details("Browser", "^firefox$", "");
        assert_eq!(page.view().profile_names[0], "Browser");
        assert_eq!(page.view().status, "Profile updated. Saved.");
        assert!(saved(&path).contains(r#""name": "Browser""#));

        page.set_fallthrough(false);
        assert!(!page.view().fallthrough);
        assert!(saved(&path).contains(r#""fallthrough": false"#));

        page.remove_profile();
        let view = page.view();
        assert_eq!(view.profile_names, ["global (all other apps)"]);
        assert_eq!(view.status, "Removed the Browser profile. Saved.");
        assert!(!saved(&path).contains("Browser"));
    }

    #[test]
    fn an_app_profile_shows_what_global_assigns_until_it_sets_the_button() {
        let (mut page, _) = page("fallthrough", None);
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+left");
        page.select_input("btn-forward");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+right");

        page.confirm_add("firefox");
        let view = page.view();
        let back = button(&view, "btn-back");
        assert!(back.inherited && !back.assigned, "falls through to Global");
        assert_eq!(back.summary, "alt+left");
        assert!(view.rows.is_empty(), "the list is only what this profile sets");

        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("ctrl+w");
        let view = page.view();
        assert!(button(&view, "btn-back").assigned && !button(&view, "btn-back").inherited);
        assert_eq!(button(&view, "btn-back").summary, "ctrl+w");
        assert!(button(&view, "btn-forward").inherited);

        page.set_fallthrough(false);
        let view = page.view();
        assert!(!button(&view, "btn-forward").inherited, "Global no longer shows through");
        assert_eq!(button(&view, "btn-forward").summary, "");
    }

    #[test]
    fn an_app_profile_says_not_set_means_use_global() {
        let (mut page, _) = page("app-wording", None);
        page.confirm_add("firefox");
        page.select_input("btn-back");
        let view = page.view();
        assert_eq!(view.action_names[0], "Not set (use Global)");
        assert_eq!(view.editor_hint, "Uses the Global assignment.");
        page.select_profile(1);
        assert_eq!(page.view().action_names[0], "Not set (normal button)");
        assert_eq!(page.view().editor_hint, "Unassigned: the button does its normal job.");
    }

    #[test]
    fn what_cannot_be_added_is_reported_and_nothing_is_written() {
        let (mut page, path) = page("refused", None);
        page.start_add();
        page.confirm_add("");
        let view = page.view();
        assert!(view.status_is_error, "{}", view.status);
        assert!(view.adding, "so it can be fixed");
        assert!(!path.exists());

        page.confirm_add("firefox");
        page.start_add();
        page.confirm_add("firefox");
        assert!(page.view().status_is_error, "an app can only have one profile");
        assert_eq!(page.view().profile_names.len(), 2);

        page.select_profile(1);
        page.remove_profile();
        assert!(page.view().status_is_error, "Global can't be removed");
        assert_eq!(page.view().profile_names.len(), 2);
    }

    #[test]
    fn an_edit_nobody_made_says_nothing_and_writes_nothing() {
        let (mut page, path) = page("no-change", None);
        page.confirm_add("firefox");
        let before = saved(&path);
        let message = page.view().status;
        let (name, class, title) = page.profile_fields();
        page.apply_details(&name, &class, &title);
        assert_eq!(page.view().status, message);
        assert_eq!(saved(&path), before);
    }

    #[test]
    fn a_file_changed_elsewhere_is_not_overwritten_and_is_read_again() {
        let (mut page, path) = page("changed-elsewhere", None);
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+left");

        let theirs = r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{"btn-middle":"ctrl+v"}}]}}}"#;
        std::fs::write(&path, theirs).unwrap();

        page.select_input("btn-forward");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+right");
        let view = page.view();
        assert!(view.status_is_error);
        assert!(view.status.contains("changed on disk"), "{}", view.status);
        assert_eq!(saved(&path), theirs, "their file is untouched");
        assert_eq!(view.rows.len(), 1, "the page now shows the file");
        assert_eq!(view.rows[0].slot, "btn-middle");
        assert_eq!(view.device_index, 0, "and stays where it was");
        assert_eq!(page.view().selected_title, "Forward button");
    }

    #[test]
    fn a_file_it_cannot_edit_is_never_written() {
        for (name, text, reason) in [
            ("not-object", "[1, 2]", "isn't a JSON object"),
            ("newer", r#"{"version": 2, "devices": {}}"#, "newer than this Settings"),
            ("broken", "{ not json", ""),
        ] {
            let (mut page, path) = page(name, Some(text));
            let view = page.view();
            assert!(view.status_is_error, "{name}");
            assert!(view.status.contains(reason), "{name}: {}", view.status);
            assert!(view.status.contains("won't overwrite"), "{name}: {}", view.status);
            page.select_input("btn-back");
            assert!(!page.view().editing, "{name}: read-only");
            assert_eq!(page.view().editor_mode, MODE_NONE);

            choose_anyway(&mut page);
            page.apply("alt+left");
            page.confirm_add("firefox");
            page.set_fallthrough(false);
            assert_eq!(saved(&path), text, "{name}: the file is left alone");
        }
    }

    /// Tries to edit through the model although the page is locked.
    fn choose_anyway(page: &mut Buttons) {
        page.select_action(2);
    }

    #[test]
    fn refresh_reads_the_file_again_and_keeps_the_selection() {
        let (mut page, path) = page("refresh", None);
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+left");

        let theirs = r#"{"devices":{"generic":{"profiles":[
            {"name":"Browser","match":{"class":"^firefox$"},"bindings":{"btn-back":"ctrl+w"}},
            {"name":"global","bindings":{"btn-back":"super+left"}}]}}}"#;
        std::fs::write(&path, theirs).unwrap();
        page.refresh();
        let view = page.view();
        assert_eq!(view.profile_names.len(), 2);
        assert_eq!(view.profile_index, 1, "the same profile stays selected when its place moved");
        assert_eq!(view.selected_title, "Back button");
        assert_eq!(view.rows[0].summary, "super+left");
        assert_eq!(view.status, "Read the assignments from the file again.");

        page.select_profile(0);
        assert_eq!(page.view().profile_names[0], "Browser");
        std::fs::write(&path, r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{}}]}}}"#).unwrap();
        page.refresh();
        let view = page.view();
        assert_eq!(view.profile_index, 0, "a profile the file no longer has falls back to Global");
        assert!(view.profile_global);
    }

    #[test]
    fn refresh_picks_up_a_file_that_was_fixed() {
        let (mut page, path) = page("fixed", Some("[]"));
        assert!(!page.view().status.is_empty() && page.view().status_is_error);
        std::fs::write(&path, r#"{"devices":{}}"#).unwrap();
        page.refresh();
        let view = page.view();
        assert!(!view.status_is_error, "{}", view.status);
        page.select_input("btn-back");
        assert!(page.view().editing);
    }

    #[test]
    fn reread_picks_up_the_file_without_saying_so() {
        let (mut page, path) = page("reread", None);
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        assert!(page.apply("alt+left"));
        assert!(!page.view().status.is_empty());

        std::fs::write(&path, r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{"btn-back":"super+left"}}]}}}"#).unwrap();
        page.reread();
        let view = page.view();
        assert_eq!(view.rows[0].summary, "super+left");
        assert_eq!(view.selected_title, "Back button", "the selection stays");
        assert_eq!(view.status, "", "opening the tab again isn't news");

        std::fs::write(&path, "[]").unwrap();
        page.reread();
        assert!(page.view().status_is_error, "a file that can't be used is still said");
    }

    #[test]
    fn apply_says_whether_it_saved() {
        let (mut page, path) = page("apply-result", None);
        assert!(!page.apply("ctrl+c"), "no button is selected");
        page.select_input("btn-back");
        choose(&mut page, ActionKind::Shortcut);
        assert!(page.apply("alt+left"));
        assert!(!saved(&path).is_empty());

        std::fs::write(&path, r#"{"devices":{}}"#).unwrap();
        assert!(!page.apply("alt+right"), "the file changed under the page: nothing is overwritten");
        assert!(page.view().status_is_error);
    }

    #[test]
    fn an_advanced_assignment_is_shown_and_can_only_be_replaced() {
        let text = r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{"btn-back":{"macro":["a","b"]}}}]}}}"#;
        let (mut page, path) = page("advanced", Some(text));
        page.select_input("btn-back");
        let view = page.view();
        assert_eq!(view.editor_mode, MODE_ADVANCED);
        assert!(view.advanced_text.contains("macro"));
        assert!(view.rows[0].advanced && view.rows[0].summary.starts_with("advanced:"));
        assert_eq!(view.action_names.last().map(String::as_str), Some("Advanced (edit in file)"));

        page.apply("");
        assert!(page.view().status_is_error, "it can't be written back");
        assert!(saved(&path).contains("macro"), "so it is not lost");

        choose(&mut page, ActionKind::Shortcut);
        page.apply("alt+left");
        assert!(!saved(&path).contains("macro"), "choosing something else replaces it");
        assert!(saved(&path).contains("alt+left"));
    }

    #[test]
    fn a_button_the_device_lacks_is_listed_last_and_flagged() {
        let text = r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{
            "thumb-9":"ctrl+z","btn-back":"alt+left","btn-left":{"mouse":"right"}}}]}}}"#;
        let (page, _) = page("unknown-button", Some(text));
        let rows = page.view().rows;
        let slots: Vec<&str> = rows.iter().map(|r| r.slot.as_str()).collect();
        assert_eq!(slots, ["btn-left", "btn-back", "thumb-9"], "picture order, strangers last");
        assert!(!rows[0].warning && !rows[1].warning);
        assert!(rows[2].warning);
        assert_eq!(rows[2].label, "thumb-9");
        assert!(rows[2].summary.contains("not a button of this mouse"));
        assert_eq!(rows[0].summary, "Right click");
    }

    #[test]
    fn rows_carry_an_icon_for_their_kind() {
        assert_eq!(glyph_for(&Binding::new(ActionKind::Shortcut, "a")), icons::glyph("keyboard"));
        assert_eq!(glyph_for(&Binding::new(ActionKind::Command, "a")), icons::glyph("terminal-2"));
        assert_eq!(glyph_for(&Binding::new(ActionKind::Media, "mute")), icons::glyph("player-play"));
        assert_eq!(glyph_for(&Binding::new(ActionKind::Mouse, "left")), icons::glyph("mouse"));
        assert_eq!(glyph_for(&Binding::new(ActionKind::Disabled, "")), "");
        assert_eq!(glyph_for(&Binding::new(ActionKind::Shortcut, "a").with_icon("click")), icons::glyph("click"));
        assert_eq!(glyph_for(&Binding::new(ActionKind::Shortcut, "a").with_icon(ICON_NONE)), "");
        assert!(!icons::glyph("keyboard").is_empty());
    }

    #[test]
    fn the_selected_button_says_what_is_known_of_its_code() {
        let (mut page, _) = page("note", None);
        page.select_input("btn-back");
        assert_eq!(page.view().selected_note, "Code: BTN_SIDE · mouse interface");
        page.select_input("wheel-tilt-left");
        assert_eq!(page.view().selected_note, "Code: not captured yet");
        page.select_input("btn-left");
        let note = page.view().selected_note;
        assert!(note.starts_with("Only \"Mouse button\" can be assigned here"), "{note}");
        assert!(note.ends_with("\nCode: BTN_LEFT · mouse interface"), "{note}");
    }

    #[test]
    fn selecting_something_that_is_not_there_changes_nothing() {
        let (mut page, _) = page("bogus-selection", None);
        page.select_input("btn-back");
        page.select_input("no-such-button");
        page.select_device(9);
        page.select_profile(9);
        page.select_action(99);
        page.select_choice(99);
        let view = page.view();
        assert_eq!(view.selected_title, "Back button");
        assert_eq!(view.device_index, 0);
        assert_eq!(view.profile_index, 0);
    }

    #[test]
    fn a_page_without_devices_is_empty_but_safe() {
        let path = temp_dir("no-devices").join("mouse.json");
        let mut page = Buttons::load(Vec::new(), path.clone());
        let view = page.view();
        assert_eq!(view.device_index, -1);
        assert!(view.buttons.is_empty() && !view.editing);
        page.select_input("btn-back");
        page.apply("x");
        page.confirm_add("firefox");
        assert!(!path.exists());
    }
}
