//! Slint wiring for the Mouse tab. The two models, `buttons` and
//! `pointer_state`, live on the UI thread. The `hyprctl` calls (sending the
//! pointer options, asking what Hyprland has, listing the open windows) run
//! on worker threads and report back through `invoke_from_event_loop`, after
//! which the page is drawn again from the models. Nothing here runs until the
//! tab is first opened.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};

use super::buttons::{Buttons, ButtonView, PanelView, RowView};
use super::devices;
use super::pointer::{self, Pointer};
use super::pointer_state::PointerState;
use super::Paths;
use crate::assign::flow;
use crate::display::hyprctl::SystemRunner;
use crate::{AssignBindingRow, DeviceButton, DevicePanel, MainWindow};

const MOUSE_TAB: i32 = 12;

/// How long after the last slider movement the options go to Hyprland. The
/// file is written at every movement; only the compositor call waits.
const APPLY_DELAY: Duration = Duration::from_millis(250);

struct State {
    buttons: Buttons,
    pointer: PointerState,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static APPLY_TIMER: Timer = Timer::default();
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| cell.borrow_mut().as_mut().map(f))
}

fn s(text: impl AsRef<str>) -> SharedString {
    SharedString::from(text.as_ref())
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(items)))
}

fn strings(items: &[String]) -> ModelRc<SharedString> {
    model(items.iter().map(s).collect())
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn render(ui: &MainWindow) {
    render_pointer(ui);
    render_buttons(ui);
}

fn render_pointer(ui: &MainWindow) {
    let Some(view) = with(|st| st.pointer.view()) else { return };
    ui.set_ms_sensitivity(view.sensitivity as f32);
    ui.set_ms_sensitivity_text(s(&view.sensitivity_text));
    ui.set_ms_accel_names(strings(&view.accel_names));
    ui.set_ms_accel_index(view.accel_index);
    ui.set_ms_natural_scroll(view.natural_scroll);
    ui.set_ms_scroll_factor(view.scroll_factor as f32);
    ui.set_ms_scroll_factor_text(s(&view.scroll_factor_text));
    ui.set_ms_pointer_at_defaults(view.at_defaults);
    ui.set_ms_pointer_status(s(&view.status));
    ui.set_ms_pointer_status_is_error(view.status_is_error);
}

fn device_panel(panel: &PanelView) -> DevicePanel {
    DevicePanel { label: s(&panel.label), x: panel.rect.x, y: panel.rect.y, w: panel.rect.w, h: panel.rect.h }
}

fn device_button(button: &ButtonView) -> DeviceButton {
    DeviceButton {
        id: s(&button.id),
        caption: s(&button.caption),
        summary: s(&button.summary),
        x: button.rect.x,
        y: button.rect.y,
        w: button.rect.w,
        h: button.rect.h,
        selected: button.selected,
        assigned: button.assigned,
        inherited: button.inherited,
    }
}

fn binding_row(row: &RowView) -> AssignBindingRow {
    AssignBindingRow {
        slot: s(&row.slot),
        label: s(&row.label),
        summary: s(&row.summary),
        advanced: row.advanced,
        glyph: s(&row.glyph),
        warning: row.warning,
    }
}

fn render_buttons(ui: &MainWindow) {
    let Some(view) = with(|st| st.buttons.view()) else { return };
    ui.set_ms_device_names(strings(&view.device_names));
    ui.set_ms_device_index(view.device_index);
    ui.set_ms_device_note(s(&view.device_note));
    ui.set_ms_aspect(view.aspect);
    ui.set_ms_panels(model(view.panels.iter().map(device_panel).collect()));
    ui.set_ms_buttons(model(view.buttons.iter().map(device_button).collect()));

    ui.set_ms_profile_names(strings(&view.profile_names));
    ui.set_ms_profile_index(view.profile_index);
    ui.set_ms_profile_global(view.profile_global);
    ui.set_ms_profile_fallthrough(view.fallthrough);
    ui.set_ms_adding_profile(view.adding);
    ui.set_ms_app_classes(strings(&view.app_classes));

    ui.set_ms_editing(view.editing);
    ui.set_ms_selected_title(s(&view.selected_title));
    ui.set_ms_selected_note(s(&view.selected_note));
    ui.set_ms_action_names(strings(&view.action_names));
    ui.set_ms_action_index(view.action_index);
    ui.set_ms_choice_names(strings(&view.choice_names));
    ui.set_ms_choice_index(view.choice_index);
    ui.set_ms_editor_mode(view.editor_mode);
    ui.set_ms_editor_hint(s(&view.editor_hint));
    ui.set_ms_advanced_text(s(&view.advanced_text));

    ui.set_ms_bindings_title(s(&view.rows_title));
    ui.set_ms_binding_rows(model(view.rows.iter().map(binding_row).collect()));
    ui.set_ms_status(s(&view.status));
    ui.set_ms_status_is_error(view.status_is_error);
    ui.set_ms_config_path(s(&view.path));
}

// ── Typed-in fields ──────────────────────────────────────────────────────────
//
// These are written from the model only when the thing they describe changes
// (another profile or button, an edit that was saved), never by a redraw, so
// a redraw can't wipe what is being typed.

fn sync_profile_fields(ui: &MainWindow) {
    if let Some((name, class, title)) = with(|st| st.buttons.profile_fields()) {
        ui.set_ms_profile_name(s(name));
        ui.set_ms_profile_class(s(class));
        ui.set_ms_profile_title(s(title));
    }
}

fn sync_editor_text(ui: &MainWindow) {
    if let Some(text) = with(|st| st.buttons.editor_text()) {
        ui.set_ms_action_text(s(text));
    }
}

/// Which of the typed-in fields to fill again after an edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Refill {
    Nothing,
    Editor,
    Both,
}

/// Runs an edit of the buttons model and draws the card again.
fn edit_buttons(weak: &Weak<MainWindow>, edit: impl FnOnce(&mut Buttons) -> Refill) {
    let Some(ui) = weak.upgrade() else { return };
    let Some(refill) = with(|st| edit(&mut st.buttons)) else { return };
    if refill == Refill::Both {
        sync_profile_fields(&ui);
    }
    if refill != Refill::Nothing {
        sync_editor_text(&ui);
    }
    render_buttons(&ui);
}

// ── Loading ──────────────────────────────────────────────────────────────────

/// Builds the models the first time the tab is opened. True when that
/// happened just now.
fn ensure_loaded(ui: &MainWindow) -> bool {
    if STATE.with(|cell| cell.borrow().is_some()) {
        return false;
    }
    let paths = Paths::standard();
    ui.set_ms_pointer_path(s(paths.pointer.display().to_string()));
    let pointer = PointerState::load(paths.pointer);
    let ask_hyprland = pointer.needs_live_read();
    let buttons = Buttons::load(devices::builtin(), paths.assignments);
    STATE.with(|cell| *cell.borrow_mut() = Some(State { buttons, pointer }));
    if ask_hyprland {
        read_live(ui.as_weak());
    }
    true
}

/// The tab is open: build the models the first time, else read the
/// assignments again in case the file changed.
fn enter(ui: &MainWindow) {
    if !ensure_loaded(ui) {
        with(|st| st.buttons.reread());
    }
    sync_profile_fields(ui);
    sync_editor_text(ui);
    render(ui);
}

/// Asks the running Hyprland what the options are (a read-only query), for a
/// first start with nothing saved.
fn read_live(weak: Weak<MainWindow>) {
    std::thread::spawn(move || {
        let result = pointer::read_live(&SystemRunner);
        let _ = slint::invoke_from_event_loop(move || {
            with(|st| st.pointer.adopt_live(result));
            if let Some(ui) = weak.upgrade() {
                render_pointer(&ui);
            }
        });
    });
}

// ── Sending the pointer options to Hyprland ──────────────────────────────────

/// Waits for the slider to rest, then sends the newest values.
fn schedule_apply(weak: &Weak<MainWindow>) {
    let weak = weak.clone();
    APPLY_TIMER.with(|timer| timer.start(TimerMode::SingleShot, APPLY_DELAY, move || start_apply(&weak)));
}

fn start_apply(weak: &Weak<MainWindow>) {
    if let Some(Some(options)) = with(|st| st.pointer.begin_apply()) {
        run_apply(weak.clone(), options);
    }
}

/// Sends `options` on a worker thread. When the answer comes, any change made
/// meanwhile is sent next, so the compositor ends with the newest values.
fn run_apply(weak: Weak<MainWindow>, options: Pointer) {
    std::thread::spawn(move || {
        let result = pointer::apply(&SystemRunner, &options);
        let _ = slint::invoke_from_event_loop(move || {
            let next = with(|st| st.pointer.finish_apply(result)).flatten();
            if let Some(ui) = weak.upgrade() {
                render_pointer(&ui);
            }
            if let Some(next) = next {
                run_apply(weak, next);
            }
        });
    });
}

/// Runs one change of the pointer options. When it changed something, the
/// card is drawn again and the compositor call is scheduled.
fn change_pointer(weak: &Weak<MainWindow>, change: impl FnOnce(&mut PointerState) -> bool) {
    let Some(ui) = weak.upgrade() else { return };
    if with(|st| change(&mut st.pointer)) == Some(true) {
        render_pointer(&ui);
        schedule_apply(weak);
    }
}

// ── Callbacks ────────────────────────────────────────────────────────────────

pub fn install(ui: &MainWindow) {
    if ui.get_active_tab() == MOUSE_TAB {
        enter(ui);
    }

    let weak = ui.as_weak();
    ui.on_ms_entered(move || {
        if let Some(ui) = weak.upgrade() {
            enter(&ui);
        }
    });

    install_pointer(ui);
    install_devices(ui);
    install_profiles(ui);
    install_editor(ui);
}

fn install_pointer(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_ms_set_sensitivity(move |value| change_pointer(&weak, |p| p.set_sensitivity(f64::from(value))));

    let weak = ui.as_weak();
    ui.on_ms_set_accel(move |index| change_pointer(&weak, |p| p.set_accel(index.max(0) as usize)));

    let weak = ui.as_weak();
    ui.on_ms_set_natural_scroll(move |on| change_pointer(&weak, |p| p.set_natural_scroll(on)));

    let weak = ui.as_weak();
    ui.on_ms_set_scroll_factor(move |value| change_pointer(&weak, |p| p.set_scroll_factor(f64::from(value))));

    let weak = ui.as_weak();
    ui.on_ms_reset_pointer(move || change_pointer(&weak, PointerState::reset));
}

fn install_devices(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_ms_refresh(move || {
        edit_buttons(&weak, |b| {
            b.refresh();
            Refill::Both
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_select_device(move |index| {
        edit_buttons(&weak, |b| {
            b.select_device(index.max(0) as usize);
            Refill::Both
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_select_button(move |id| {
        edit_buttons(&weak, |b| {
            b.select_input(&id);
            Refill::Editor
        })
    });
}

fn install_profiles(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_ms_select_profile(move |index| {
        edit_buttons(&weak, |b| {
            b.select_profile(index.max(0) as usize);
            Refill::Both
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_start_add_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        with(|st| st.buttons.start_add());
        ui.set_ms_new_app(SharedString::default());
        render_buttons(&ui);
        // The open windows, to pick one from: a read-only `hyprctl clients`.
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let classes = flow::running_app_classes();
            let _ = slint::invoke_from_event_loop(move || {
                with(|st| st.buttons.set_app_classes(classes));
                if let Some(ui) = weak.upgrade() {
                    render_buttons(&ui);
                }
            });
        });
    });

    let weak = ui.as_weak();
    ui.on_ms_pick_app(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        if let Some(class) = with(|st| st.buttons.app_class(index.max(0) as usize).map(str::to_string)).flatten() {
            ui.set_ms_new_app(s(class));
        }
    });

    let weak = ui.as_weak();
    ui.on_ms_confirm_add_profile(move || {
        let Some(ui) = weak.upgrade() else { return };
        let class = ui.get_ms_new_app().to_string();
        edit_buttons(&weak, |b| {
            b.confirm_add(&class);
            Refill::Both
        });
    });

    let weak = ui.as_weak();
    ui.on_ms_cancel_add_profile(move || {
        edit_buttons(&weak, |b| {
            b.cancel_add();
            Refill::Nothing
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_remove_profile(move || {
        edit_buttons(&weak, |b| {
            b.remove_profile();
            Refill::Both
        })
    });

    // The typed name, window class and title stay as typed: if the edit is
    // refused the text is still there to fix.
    let weak = ui.as_weak();
    ui.on_ms_apply_profile_details(move || {
        let Some(ui) = weak.upgrade() else { return };
        let name = ui.get_ms_profile_name().to_string();
        let class = ui.get_ms_profile_class().to_string();
        let title = ui.get_ms_profile_title().to_string();
        edit_buttons(&weak, |b| {
            b.apply_details(&name, &class, &title);
            Refill::Nothing
        });
    });

    let weak = ui.as_weak();
    ui.on_ms_set_fallthrough(move |on| {
        edit_buttons(&weak, |b| {
            b.set_fallthrough(on);
            Refill::Nothing
        })
    });
}

fn install_editor(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_ms_select_action(move |index| {
        edit_buttons(&weak, |b| {
            b.select_action(index.max(0) as usize);
            Refill::Editor
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_select_choice(move |index| {
        edit_buttons(&weak, |b| {
            b.select_choice(index.max(0) as usize);
            Refill::Nothing
        })
    });

    let weak = ui.as_weak();
    ui.on_ms_apply_binding(move || {
        let Some(ui) = weak.upgrade() else { return };
        let text = ui.get_ms_action_text().to_string();
        edit_buttons(&weak, |b| if b.apply(&text) { Refill::Editor } else { Refill::Nothing });
    });
}
