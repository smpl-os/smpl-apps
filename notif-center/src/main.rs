mod backend;

#[cfg(test)]
mod action_dispatch_tests;

use backend::{
    clear_all_notifications, debug_log, dismiss_notification, dispatch_notification,
    execute_dispatch, get_notifications, Notification,
};
use smpl_common::theme::{self, ThemePalette, ThemeRole};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

fn to_ui_item(n: &Notification) -> NotificationItem {
    NotificationItem {
        id: n.id,
        icon: n.icon.clone().into(),
        summary: n.summary.clone().into(),
        body: n.body.clone().into(),
        date: n.date.clone().into(),
        time: n.time.clone().into(),
    }
}

fn apply_theme(ui: &MainWindow, palette: &ThemePalette) {
    let theme = Theme::get(ui);
    theme.set_bg(palette.bg.darker(0.05)); // Match EWW's darken($bg, 5%)
    theme.set_fg(palette.fg);
    theme.set_fg_dim(palette.fg_dim);
    theme.set_accent(palette.accent);
    theme.set_bg_light(palette.bg_light);
    theme.set_bg_lighter(palette.bg_lighter);
    theme.set_danger(palette.danger);
    theme.set_success(palette.success);
    theme.set_warning(palette.warning);
    theme.set_info(palette.info);
    theme.set_opacity(palette.opacity);
}

fn refresh_model(
    ui: &MainWindow,
    state: &Rc<RefCell<Vec<Notification>>>,
    model: &Rc<VecModel<NotificationItem>>,
) {
    let notifications = get_notifications();
    *state.borrow_mut() = notifications.clone();

    model.set_vec(notifications.iter().map(to_ui_item).collect::<Vec<_>>());
    ui.set_notifications(ModelRc::from(model.clone()));

    let len = model.row_count() as i32;
    let current = ui.get_selected_index();
    let clamped = if len == 0 {
        -1
    } else if current < 0 {
        0
    } else if current >= len {
        len - 1
    } else {
        current
    };
    ui.set_selected_index(clamped);

    let title = if len > 0 {
        format!("Notifications ({})", len)
    } else {
        "Notifications".to_string()
    };
    ui.set_notif_count_text(title.into());
}

fn main() -> Result<(), slint::PlatformError> {
    for arg in std::env::args() {
        if arg == "-v" || arg == "--version" {
            println!("notif-center v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    smpl_common::init("notif-center", 384.0, 520.0)?;

    let ui = MainWindow::new()?;
    let ui_weak = ui.as_weak();
    let _theme_timer = theme::watch(ThemeRole::Popup, move |palette| {
        if let Some(ui) = ui_weak.upgrade() {
            apply_theme(&ui, palette);
        }
    });

    let state: Rc<RefCell<Vec<Notification>>> = Rc::new(RefCell::new(Vec::new()));
    let model = Rc::new(VecModel::<NotificationItem>::default());

    refresh_model(&ui, &state, &model);

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_dismiss(move |id| {
            dismiss_notification(id);
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_clear_all(move || {
            clear_all_notifications();
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
            }
        });
    }

    {
        // Row pointer trace (down/up per id) into the existing debug log,
        // so a live click can be correlated with the on_activate outcome.
        // move/other are skipped so mouse movement does not add
        // synchronous per-event filesystem writes.
        ui.on_debug_pointer(|id, kind| {
            if kind != "down" && kind != "up" {
                return;
            }
            debug_log(&format!("debug_pointer: id={id} kind={kind:?}"));
        });
    }

    {
        let state = state.clone();
        ui.on_activate(move |id| {
            let outcome = {
                let state = state.borrow();
                dispatch_notification(&state, id, &mut execute_dispatch)
            };
            debug_log(&format!("on_activate: id={id} outcome={outcome:?}"));
        });
    }

    {
        ui.on_close(move || {
            std::process::exit(0);
        });
    }

    {
        let ui_weak = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().with_winit_window(|winit_win: &i_slint_backend_winit::winit::window::Window| {
                    let _ = winit_win.drag_window();
                });
            }
        });
    }

    {
        let ui_weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(2),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.invoke_refresh();
                }
            },
        );
        std::mem::forget(timer);
    }

    ui.invoke_focus_list();
    ui.run()
}
