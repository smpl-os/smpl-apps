mod catalog;
mod installer;
mod sources;
mod theme;

use catalog::{merge_results, AppEntry, Source};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

slint::include_modules!();

fn to_ui_item(app: &AppEntry) -> AppItem {
    AppItem {
        name: app.name.clone().into(),
        id: app.id.clone().into(),
        version: app.version.clone().into(),
        description: app.description.clone().into(),
        source: SharedString::from(app.source_label()),
        icon_path: app.icon_path.clone().into(),
        homepage: app.homepage.clone().into(),
        votes: app.votes as i32,
        popularity: app.popularity as f32,
        installed: app.installed,
        has_update: false,  // Will be set when checking for updates
        selected: false,    // User can select apps for batch update
        update_progress: 0.0,  // Progress tracking for individual updates
    }
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

/// Perform a search across enabled sources.
fn do_search(query: &str, aur: bool, flatpak: bool, appimage: bool, pacman: bool) -> Vec<AppEntry> {
    if query.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();

    if pacman {
        results.extend(sources::pacman::search(query));
    }
    if aur {
        results.extend(sources::aur::search(query));
    }
    if flatpak {
        results.extend(sources::flathub::search(query));
    }
    if appimage {
        results.extend(sources::appimage::search(query));
    }

    merge_results(results, query)
}

fn update_results(
    ui: &MainWindow,
    state: &Rc<RefCell<Vec<AppEntry>>>,
    model: &Rc<VecModel<AppItem>>,
    results: Vec<AppEntry>,
) {
    *state.borrow_mut() = results.clone();
    model.set_vec(results.iter().map(to_ui_item).collect::<Vec<_>>());
    ui.set_results(ModelRc::from(model.clone()));

    let len = model.row_count() as i32;
    if len > 0 {
        ui.set_selected_index(0);
    } else {
        ui.set_selected_index(-1);
    }

    ui.set_status_text(
        if len > 0 {
            SharedString::from(format!("{} results", len))
        } else {
            SharedString::default()
        },
    );
    ui.set_searching(false);
}

/// Get list of installed packages and check for updates
fn get_installed_packages() -> Vec<AppItem> {
    let mut apps = Vec::new();

    // Query pacman for installed packages with updates
    if let Ok(output) = std::process::Command::new("pacman")
        .args(["-Qu"])
        .output()
    {
        let updates_text = String::from_utf8_lossy(&output.stdout);
        let mut has_update_set = HashSet::new();
        for line in updates_text.lines() {
            if let Some(name) = line.split_whitespace().next() {
                has_update_set.insert(name.to_string());
            }
        }

        // Get all pacman packages
        if let Ok(output) = std::process::Command::new("pacman")
            .args(["-Q"])
            .output()
        {
            let packages_text = String::from_utf8_lossy(&output.stdout);
            for line in packages_text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    let name = parts[0].to_string();
                    let version = parts[1].to_string();
                    let has_update = has_update_set.contains(&name);

                    apps.push(AppItem {
                        name: name.clone().into(),
                        id: name.into(),
                        version: version.into(),
                        description: "Pacman package".into(),
                        source: "Pacman".into(),
                        icon_path: "P".into(),
                        homepage: String::new().into(),
                        votes: 0,
                        popularity: 0.0,
                        installed: true,
                        has_update,
                        selected: false,
                        update_progress: 0.0,
                    });
                }
            }
        }
    }

    // Query paru for AUR packages
    if let Ok(output) = std::process::Command::new("paru")
        .args(["-Q"])
        .output()
    {
        let packages_text = String::from_utf8_lossy(&output.stdout);
        let mut paru_packages = HashSet::new();
        
        for line in packages_text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                paru_packages.insert(parts[0].to_string());
            }
        }

        // Check which ones are AUR vs pacman
        if let Ok(output) = std::process::Command::new("pacman")
            .args(["-Sg", "base", "base-devel"])
            .output()
        {
            let base_text = String::from_utf8_lossy(&output.stdout);
            for line in base_text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    paru_packages.remove(parts[1]);
                }
            }
        }

        // Check for AUR updates
        if let Ok(output) = std::process::Command::new("paru")
            .args(["-Qu"])
            .output()
        {
            let updates_text = String::from_utf8_lossy(&output.stdout);
            let mut aur_updates = HashSet::new();
            for line in updates_text.lines() {
                if let Some(name) = line.split_whitespace().next() {
                    aur_updates.insert(name.to_string());
                }
            }

            // Add AUR packages that aren't already in our list
            for name in paru_packages {
                if !apps.iter().any(|a| a.id == name) {
                    if let Ok(output) = std::process::Command::new("pacman")
                        .args(["-Q", &name])
                        .output()
                    {
                        let query_text = String::from_utf8_lossy(&output.stdout);
                        if let Some(line) = query_text.lines().next() {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 2 {
                                let has_update = aur_updates.contains(&name);
                                apps.push(AppItem {
                                    name: name.clone().into(),
                                    id: name.clone().into(),
                                    version: parts[1].to_string().into(),
                                    description: "AUR package".into(),
                                    source: "AUR".into(),
                                    icon_path: "A".into(),
                                    homepage: String::new().into(),
                                    votes: 0,
                                    popularity: 0.0,
                                    installed: true,
                                    has_update,
                                    selected: false,
                                    update_progress: 0.0,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Query flatpak for installed apps
    if let Ok(output) = std::process::Command::new("flatpak")
        .args(["list", "--app", "--columns=application,version"])
        .output()
    {
        let flatpak_text = String::from_utf8_lossy(&output.stdout);
        for line in flatpak_text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                let app_id = parts[0].to_string();
                let version = parts[parts.len() - 1].to_string();
                
                // TODO: Check flatpak remotes for updates
                let has_update = false;

                apps.push(AppItem {
                    name: app_id.split('.').next_back().unwrap_or(&app_id).to_string().into(),
                    id: app_id.into(),
                    version: version.into(),
                    description: "Flatpak application".into(),
                    source: "Flatpak".into(),
                    icon_path: "F".into(),
                    homepage: String::new().into(),
                    votes: 0,
                    popularity: 0.0,
                    installed: true,
                    has_update,
                    selected: false,
                    update_progress: 0.0,
                });
            }
        }
    }

    apps
}

fn main() -> Result<(), slint::PlatformError> {
    for arg in std::env::args() {
        if arg == "-v" || arg == "--version" {
            println!("app-center v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    smpl_common::init("app-center", 560.0, 620.0)?;

    let ui = MainWindow::new()?;
    apply_theme(&ui);
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());

    let state: Rc<RefCell<Vec<AppEntry>>> = Rc::new(RefCell::new(Vec::new()));
    let model = Rc::new(VecModel::<AppItem>::default());

    // -- Search callback --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_search(move |query| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let q = query.to_string();

            if q.is_empty() {
                update_results(&ui, &state, &model, Vec::new());
                return;
            }

            ui.set_searching(true);

            let aur = ui.get_filter_aur();
            let flatpak = ui.get_filter_flatpak();
            let appimage = ui.get_filter_appimage();
            let pacman = ui.get_filter_pacman();

            // Run search (blocking but fast for AUR/Pacman; local for cached Flatpak/AppImage)
            let results = do_search(&q, aur, flatpak, appimage, pacman);
            update_results(&ui, &state, &model, results);
        });
    }

    // -- Filter changed: re-run current search --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_filter_changed(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let q = ui.get_search_text().to_string();
            if q.is_empty() {
                return;
            }

            let aur = ui.get_filter_aur();
            let flatpak = ui.get_filter_flatpak();
            let appimage = ui.get_filter_appimage();
            let pacman = ui.get_filter_pacman();

            let results = do_search(&q, aur, flatpak, appimage, pacman);
            update_results(&ui, &state, &model, results);
        });
    }

    // -- Tab switching --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        ui.on_switch_tab(move |tab_index| {
            let Some(ui) = ui_weak.upgrade() else { return };

            if tab_index == 1 {
                let recommended = sources::recommended::get_recommended();
                update_results(&ui, &state, &model, recommended);
            } else {
                let q = ui.get_search_text().to_string();
                if q.is_empty() {
                    update_results(&ui, &state, &model, Vec::new());
                } else {
                    let aur = ui.get_filter_aur();
                    let flatpak = ui.get_filter_flatpak();
                    let appimage = ui.get_filter_appimage();
                    let pacman = ui.get_filter_pacman();
                    let results = do_search(&q, aur, flatpak, appimage, pacman);
                    update_results(&ui, &state, &model, results);
                }
            }
        });
    }

    // -- Select app: show detail view --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        ui.on_select_app(move |index| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let idx = index as usize;
            let borrowed = state.borrow();
            if idx >= borrowed.len() {
                return;
            }

            let app = &borrowed[idx];
            ui.set_selected_index(index);
            ui.set_install_status(SharedString::default());
            ui.set_console_output(SharedString::default());
            ui.set_process_finished(false);
            ui.set_process_success(false);

            // For Flatpak apps, fetch richer details
            if app.source == Source::Flatpak && !app.id.is_empty() {
                if let Some(detail) = sources::flathub::get_details(&app.id) {
                    ui.set_detail_description(SharedString::from(&detail.description));
                } else {
                    ui.set_detail_description(SharedString::default());
                }
            } else {
                ui.set_detail_description(SharedString::default());
            }

            ui.set_show_detail(true);
        });
    }

    // -- Active process state --
    struct ActiveInstall {
        idx: usize,
        is_install: bool,
        process: installer::StreamingProcess,
    }
    let active_install: Rc<RefCell<Option<ActiveInstall>>> = Rc::new(RefCell::new(None));

    // Helper: handle immediate (non-streaming) results
    fn handle_immediate(
        ui: &MainWindow,
        state: &Rc<RefCell<Vec<AppEntry>>>,
        model: &Rc<VecModel<AppItem>>,
        idx: usize,
        is_install: bool,
        result: &installer::ImmediateResult,
    ) {
        ui.set_installing(false);
        ui.set_process_finished(true);
        ui.set_process_success(result.success);
        ui.set_console_output(SharedString::from(&result.message));
        if result.success {
            let new_state = is_install;
            let mut borrowed = state.borrow_mut();
            if let Some(entry) = borrowed.get_mut(idx) {
                entry.installed = new_state;
            }
            drop(borrowed);
            if let Some(mut item) = model.row_data(idx) {
                item.installed = new_state;
                model.set_row_data(idx, item);
            }
            // Refresh start-menu app cache so newly installed apps appear immediately
            let _ = std::process::Command::new("rebuild-app-cache").spawn();
        }
    }

    // -- Install app --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        let active = active_install.clone();
        ui.on_install_app(move |index| {
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_installing() { return; }
            let idx = index as usize;
            let borrowed = state.borrow();
            if idx >= borrowed.len() { return; }

            let app = borrowed[idx].clone();
            drop(borrowed);

            ui.set_installing(true);
            ui.set_console_output(SharedString::default());
            ui.set_console_last_line(SharedString::default());
            ui.set_process_finished(false);
            ui.set_process_success(false);

            match installer::spawn_install(&app.source, &app.id) {
                installer::SpawnResult::Streaming(process) => {
                    *active.borrow_mut() = Some(ActiveInstall {
                        idx,
                        is_install: true,
                        process,
                    });
                }
                installer::SpawnResult::Immediate(result) => {
                    handle_immediate(&ui, &state, &model, idx, true, &result);
                }
            }
        });
    }

    // -- Uninstall app --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        let active = active_install.clone();
        ui.on_uninstall_app(move |index| {
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_installing() { return; }
            let idx = index as usize;
            let borrowed = state.borrow();
            if idx >= borrowed.len() { return; }

            let app = borrowed[idx].clone();
            drop(borrowed);

            ui.set_installing(true);
            ui.set_console_output(SharedString::default());
            ui.set_console_last_line(SharedString::default());
            ui.set_process_finished(false);
            ui.set_process_success(false);

            match installer::spawn_uninstall(&app.source, &app.id, &app.name) {
                installer::SpawnResult::Streaming(process) => {
                    *active.borrow_mut() = Some(ActiveInstall {
                        idx,
                        is_install: false,
                        process,
                    });
                }
                installer::SpawnResult::Immediate(result) => {
                    handle_immediate(&ui, &state, &model, idx, false, &result);
                }
            }
        });
    }

    // -- Console input --
    {
        let active = active_install.clone();
        ui.on_send_console_input(move |text| {
            if let Some(ref mut ai) = *active.borrow_mut() {
                ai.process.send_input(&text);
            }
        });
    }

    // -- Cancel install --
    {
        let ui_weak = ui.as_weak();
        let active = active_install.clone();
        ui.on_cancel_install(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut guard = active.borrow_mut();
            if let Some(ref mut ai) = *guard {
                ai.process.kill();
            }
            *guard = None;
            drop(guard);

            ui.set_installing(false);
            ui.set_process_finished(true);
            ui.set_process_success(false);
            ui.set_console_output(SharedString::from("Cancelled by user"));
            ui.set_console_last_line(SharedString::default());
        });
    }

    // -- Poll active process for output and completion --
    {
        let ui_weak = ui.as_weak();
        let state = state.clone();
        let model = model.clone();
        let active = active_install.clone();
        // Accumulate full output in Rust (not on the UI during install)
        let full_output: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let poll_timer = slint::Timer::default();
        poll_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                let Some(ui) = ui_weak.upgrade() else { return };
                let mut guard = active.borrow_mut();
                let Some(ref mut ai) = *guard else { return };

                // Append new output lines, track last meaningful line
                let new_lines = ai.process.poll_output();
                if !new_lines.is_empty() {
                    let mut buf = full_output.borrow_mut();
                    for line in &new_lines {
                        if !buf.is_empty() {
                            buf.push('\n');
                        }
                        buf.push_str(line);

                        // Update live status with last non-junk line
                        let trimmed = line.trim();
                        if !trimmed.is_empty()
                            && !trimmed.chars().all(|c| c == '#' || c == ' ')
                        {
                            ui.set_console_last_line(SharedString::from(trimmed));
                        }
                    }
                }

                // Check if process finished
                if let Some(success) = ai.process.try_wait() {
                    let idx = ai.idx;
                    let is_install = ai.is_install;
                    drop(guard);

                    ui.set_installing(false);
                    ui.set_process_finished(true);
                    ui.set_process_success(success);

                    // Set full output (shown only on failure in the UI)
                    ui.set_console_output(SharedString::from(
                        full_output.borrow().as_str(),
                    ));
                    full_output.borrow_mut().clear();

                    if success {
                        let new_state = is_install;
                        let mut borrowed = state.borrow_mut();
                        if let Some(entry) = borrowed.get_mut(idx) {
                            entry.installed = new_state;
                        }
                        drop(borrowed);
                        if let Some(mut item) = model.row_data(idx) {
                            item.installed = new_state;
                            model.set_row_data(idx, item);
                        }
                        // Refresh start-menu app cache so newly installed apps appear immediately
                        let _ = std::process::Command::new("rebuild-app-cache").spawn();
                    }

                    *active.borrow_mut() = None;
                }
            },
        );
        std::mem::forget(poll_timer);
    }

    // -- Open homepage --
    {
        let state = state.clone();
        ui.on_open_homepage(move |index| {
            let idx = index as usize;
            let borrowed = state.borrow();
            if let Some(app) = borrowed.get(idx) {
                if !app.homepage.is_empty() {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&app.homepage)
                        .spawn();
                }
            }
        });
    }

    // -- Refresh catalogs --
    {
        let ui_weak = ui.as_weak();
        ui.on_refresh_catalog(move || {
            if let Some(ui) = ui_weak.upgrade() {
                // Delete cache files to force re-download
                let cache = catalog::cache_dir();
                let _ = std::fs::remove_file(cache.join("appimage-catalog.json"));
                ui.set_status_text("Catalogs cleared - search again to refresh".into());
            }
        });
    }

    // -- Close --
    {
        ui.on_close(move || {
            std::process::exit(0);
        });
    }

    // -- Check for Updates (Installed tab) --
    {
        let ui_weak = ui.as_weak();
        ui.on_check_for_updates(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_checking_for_updates(true);

            // Query installed packages in background
            let ui_weak_clone = ui_weak.clone();
            std::thread::spawn(move || {
                let apps = get_installed_packages();
                let apps_with_updates = apps.iter().filter(|a| a.has_update).count() as i32;

                if let Some(ui) = ui_weak_clone.upgrade() {
                    let model = Rc::new(VecModel::<AppItem>::from(apps));
                    ui.set_installed_apps(ModelRc::from(model));
                    ui.set_apps_with_updates(apps_with_updates);
                    ui.set_checking_for_updates(false);
                }
            });
        });
    }

    // -- Toggle App Selection --
    {
        let ui_weak = ui.as_weak();
        ui.on_toggle_app_select(move |idx| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let index = idx as usize;
            
            // Get current installed apps model
            if let Some(mut item) = ui.get_installed_apps().row_data(index) {
                item.selected = !item.selected;
                ui.get_installed_apps().set_row_data(index, item);
            }
        });
    }

    // -- Update Selected Apps --
    {
        ui.on_update_selected_apps(move || {
            // TODO: Run updates for selected apps
            eprintln!("update_selected_apps: would update selected apps");
        });
    }

    // -- Update OS (full system update, kernel + drivers) --
    {
        ui.on_update_os(move || {
            let _ = std::process::Command::new("smplos-update")
                .args(["--mode", "full"])
                .spawn();
        });
    }

    // -- Drag --
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

    // -- Periodic theme refresh --
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

    ui.invoke_focus_search();
    ui.run()
}
