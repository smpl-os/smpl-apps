mod backend;
mod theme;

use backend::{delete_all_webapps, delete_webapp, list_vpn_interfaces, save_webapp, scan_webapps, WebApp};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Model, ModelRc, SharedString, VecModel};
use smpl_common::keybindings;
use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

fn to_ui_item(app: &WebApp, hotkey_slugs: &std::collections::HashSet<String>) -> WebAppItem {
    WebAppItem {
        name: app.name.clone().into(),
        slug: app.slug.clone().into(),
        url: app.url.clone().into(),
        secure: app.secure,
        clear_on_exit: app.clear_on_exit,
        vpn_iface: app.vpn_iface.clone().into(),
        vpn_required: app.vpn_required,
        icon: app.icon.clone().into(),
        marked: false,
        has_hotkey: hotkey_slugs.contains(&app.slug),
    }
}

/// Collect the set of webapp slugs that have a keybinding assigned.
fn hotkey_slugs() -> std::collections::HashSet<String> {
    let mut slugs = std::collections::HashSet::new();
    if let Ok(file) = keybindings::BindingsFile::load() {
        for kb in &file.bindings {
            if kb.dispatcher == "exec" && kb.args.contains("launch-webapp") {
                // Extract slug from args like: launch-webapp "--name" "chat" "https://..."
                // or: launch-webapp --name chat https://...
                if let Some(pos) = kb.args.find("--name") {
                    let after = &kb.args[pos + 6..];
                    // Skip optional closing quote of the flag itself
                    let after = after.strip_prefix('"').unwrap_or(after).trim_start();
                    // Parse quoted or unquoted value
                    let slug = if let Some(stripped) = after.strip_prefix('"') {
                        stripped.split('"').next().unwrap_or_default()
                    } else {
                        after.split_whitespace().next().unwrap_or_default()
                    };
                    if !slug.is_empty() {
                        slugs.insert(slug.to_string());
                    }
                }
            }
        }
    }
    slugs
}

fn apply_theme(ui: &MainWindow) {
    let palette = theme::load_theme_from_eww_scss(&format!(
        "{}/.config/eww/theme-colors.scss",
        std::env::var("HOME").unwrap_or_default()
    ));

    let theme = Theme::get(ui);
    theme.set_bg(palette.bg.darker(0.05));
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
    state: &Rc<RefCell<Vec<WebApp>>>,
    model: &Rc<VecModel<WebAppItem>>,
) {
    let apps = scan_webapps();
    *state.borrow_mut() = apps.clone();

    let hk_slugs = hotkey_slugs();
    model.set_vec(apps.iter().map(|a| to_ui_item(a, &hk_slugs)).collect::<Vec<_>>());
    ui.set_webapps(ModelRc::from(model.clone()));

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
        format!("Web Apps ({})", len)
    } else {
        "Web Apps".to_string()
    };
    ui.set_title_text(title.into());
}

fn refresh_vpn_list(ui: &MainWindow) {
    let ifaces = list_vpn_interfaces();
    let vpn_model: Vec<SharedString> = ifaces.into_iter().map(|s| s.into()).collect();
    ui.set_vpn_interfaces(ModelRc::from(Rc::new(VecModel::from(vpn_model))));
}

fn main() -> Result<(), slint::PlatformError> {
    let mut start_on_create = false;

    for arg in std::env::args() {
        match arg.as_str() {
            "-v" | "--version" => {
                println!("webapp-center v{}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "-c" | "--create" => {
                start_on_create = true;
            }
            _ => {}
        }
    }

    smpl_common::init("webapp-center", 440.0, 520.0)?;

    let ui = MainWindow::new()?;
    apply_theme(&ui);

    let state: Rc<RefCell<Vec<WebApp>>> = Rc::new(RefCell::new(Vec::new()));
    let model = Rc::new(VecModel::<WebAppItem>::default());

    refresh_model(&ui, &state, &model);
    refresh_vpn_list(&ui);

    // Start on create screen if -c flag passed
    if start_on_create {
        ui.invoke_show_create_screen();
    }

    // Refresh callback
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_refresh(move || {
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
                refresh_vpn_list(&ui);
            }
        });
    }

    // Toggle mark on single item
    {
        let model = model.clone();
        ui.on_toggle_mark(move |index| {
            let idx = index as usize;
            if idx < model.row_count() {
                if let Some(mut item) = model.row_data(idx) {
                    item.marked = !item.marked;
                    model.set_row_data(idx, item);
                }
            }
        });
    }

    // Delete single app
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_delete_app(move |index| {
            let idx = index as usize;
            let borrowed = state.borrow();
            if idx < borrowed.len() {
                let app = borrowed[idx].clone();
                drop(borrowed);
                delete_webapp(&app);
                if let Some(ui) = ui_weak.upgrade() {
                    refresh_model(&ui, &state, &model);
                }
            }
        });
    }

    // Delete all apps
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_delete_all(move || {
            let apps = state.borrow().clone();
            delete_all_webapps(&apps);
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
            }
        });
    }

    // Delete marked items (or selected if none marked)
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_delete_marked(move |selected_index| {
            let borrowed = state.borrow();
            let marked: Vec<usize> = (0..model.row_count())
                .filter(|&i| model.row_data(i).map(|item| item.marked).unwrap_or(false))
                .collect();

            if !marked.is_empty() {
                let to_delete: Vec<WebApp> = marked
                    .iter()
                    .filter_map(|&i| borrowed.get(i).cloned())
                    .collect();
                drop(borrowed);
                for app in &to_delete {
                    delete_webapp(app);
                }
            } else {
                let idx = selected_index as usize;
                if idx < borrowed.len() {
                    let app = borrowed[idx].clone();
                    drop(borrowed);
                    delete_webapp(&app);
                } else {
                    return;
                }
            }
            if let Some(ui) = ui_weak.upgrade() {
                refresh_model(&ui, &state, &model);
            }
        });
    }

    // Save app (create or edit)
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_save_app(move |name, url, secure, clear_on_exit, vpn, vpn_required| {
            let editing_index = ui_weak.upgrade().map(|ui| ui.get_form_editing_index()).unwrap_or(-1);

            // If editing, delete old entry first
            if editing_index >= 0 {
                let idx = editing_index as usize;
                let borrowed = state.borrow();
                if idx < borrowed.len() {
                    let old = borrowed[idx].clone();
                    drop(borrowed);
                    delete_webapp(&old);
                }
            }

            match save_webapp(
                name.as_str(),
                url.as_str(),
                secure,
                clear_on_exit,
                vpn.as_str(),
                vpn_required,
            ) {
                Ok(slug) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        // Save hotkey binding if one was captured
                        let hk_mods = ui.get_form_hotkey_mods().to_string();
                        let hk_key = ui.get_form_hotkey_key().to_string();

                        if let Ok(mut file) = keybindings::BindingsFile::load() {
                            // Always remove any old binding for this webapp first
                            let launch_cmd = format!("launch-webapp --name {slug}");
                            let to_remove: Vec<usize> = file.bindings.iter().enumerate()
                                .filter(|(_, b)| b.args.contains(&launch_cmd) || b.args.contains(&format!("\"--name\" \"{slug}\"")))
                                .map(|(i, _)| i)
                                .collect();
                            for idx in to_remove.into_iter().rev() {
                                file.remove(idx);
                            }

                            if !hk_key.is_empty() {
                                // Remove any conflicting binding that uses the same key combo
                                if let Some(conflict) = file.find_conflict(hk_mods.trim(), &hk_key, "", None) {
                                    file.remove(conflict.index);
                                }

                                // Add the new binding
                                file.add(keybindings::Keybinding {
                                    bind_type: "bindd".to_string(),
                                    mods: hk_mods.trim().to_string(),
                                    key: hk_key,
                                    description: format!("Launch {}", name),
                                    dispatcher: "exec".to_string(),
                                    args: format!("launch-webapp \"--name\" \"{slug}\" \"{}\"", url),
                                    section: "Application Launchers".to_string(),
                                    submap: String::new(),
                                });
                            }

                            let _ = file.save_and_reload();
                        }

                        refresh_model(&ui, &state, &model);
                        ui.invoke_go_back_to_list();
                    }
                }
                Err(msg) => {
                    if let Some(ui) = ui_weak.upgrade() {
                        ui.set_form_error(msg.into());
                    }
                }
            }
        });
    }

    // Hotkey: start capture (enter empty submap so keys pass through)
    {
        ui.on_hotkey_start_capture(move || {
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "kb-capture"])
                .output();
        });
    }

    // Hotkey: cancel capture
    {
        let ui_weak = ui.as_weak();
        ui.on_hotkey_cancel_capture(move || {
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "reset"])
                .output();
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_form_hotkey_capturing(false);
            }
        });
    }

    // Hotkey: save combo (after key capture) with conflict check
    {
        let ui_weak = ui.as_weak();
        ui.on_hotkey_save_combo(move |mods, key_text| {
            let _ = std::process::Command::new("hyprctl")
                .args(["dispatch", "submap", "reset"])
                .output();

            let mods_clean = mods.trim().to_string();
            let hypr_key = keybindings::slint_key_to_hyprland(key_text.as_str());

            if hypr_key.is_empty() {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_form_hotkey_conflict("Unknown key - try again".into());
                }
                return;
            }

            // Check for conflicts
            if let Ok(file) = keybindings::BindingsFile::load() {
                if let Some(conflict) = file.find_conflict(&mods_clean, &hypr_key, "", None) {
                    if let Some(ui) = ui_weak.upgrade() {
                        let combo = keybindings::Keybinding {
                            bind_type: String::new(),
                            mods: mods_clean.clone(),
                            key: hypr_key.clone(),
                            description: String::new(),
                            dispatcher: String::new(),
                            args: String::new(),
                            section: String::new(),
                            submap: String::new(),
                        };
                        ui.set_form_hotkey_combo(combo.combo_display().into());
                        ui.set_form_hotkey_mods(mods_clean.into());
                        ui.set_form_hotkey_key(hypr_key.into());
                        ui.set_form_hotkey_conflict(SharedString::from(format!(
                            "Warning: {} is used by \"{}\" -- will be overwritten on save",
                            conflict.existing.combo_display(),
                            conflict.existing.description,
                        )));
                    }
                    return;
                }
            }

            // No conflict — set the combo
            if let Some(ui) = ui_weak.upgrade() {
                let combo = keybindings::Keybinding {
                    bind_type: String::new(),
                    mods: mods_clean.clone(),
                    key: hypr_key.clone(),
                    description: String::new(),
                    dispatcher: String::new(),
                    args: String::new(),
                    section: String::new(),
                    submap: String::new(),
                };
                ui.set_form_hotkey_combo(combo.combo_display().into());
                ui.set_form_hotkey_mods(mods_clean.into());
                ui.set_form_hotkey_key(hypr_key.into());
                ui.set_form_hotkey_conflict(SharedString::default());
            }
        });
    }

    // Hotkey: clear
    {
        ui.on_hotkey_clear(move || {
            // Nothing extra needed — UI already clears the properties
        });
    }

    // Load existing hotkey for a webapp by slug
    {
        let ui_weak = ui.as_weak();
        ui.on_load_hotkey(move |slug| {
            let slug_str = slug.to_string();
            if let Ok(file) = keybindings::BindingsFile::load() {
                for kb in &file.bindings {
                    let pat1 = format!("--name {slug_str}");
                    let pat2 = format!("\"--name\" \"{slug_str}\"");
                    if kb.dispatcher == "exec"
                        && (kb.args.contains(&pat1)
                            || kb.args.contains(&pat2))
                    {
                        if let Some(ui) = ui_weak.upgrade() {
                            ui.set_form_hotkey_combo(SharedString::from(kb.combo_display()));
                            ui.set_form_hotkey_mods(SharedString::from(&kb.mods));
                            ui.set_form_hotkey_key(SharedString::from(&kb.key));
                        }
                        return;
                    }
                }
            }
        });
    }

    // Close
    {
        ui.on_close(move || {
            std::process::exit(0);
        });
    }

    // Drag
    {
        let ui_weak = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = ui_weak.upgrade() {
                ui.window().with_winit_window(
                    |winit_win: &i_slint_backend_winit::winit::window::Window| {
                        let _ = winit_win.drag_window();
                    },
                );
            }
        });
    }

    // Periodic theme refresh
    {
        let ui_weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(2),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    apply_theme(&ui);
                }
            },
        );
        std::mem::forget(timer);
    }

    ui.invoke_focus_list();
    ui.run()
}
