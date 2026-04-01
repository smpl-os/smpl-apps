mod backend;
mod theme;

use backend::{delete_all_webapps, delete_webapp, list_vpn_interfaces, save_webapp, scan_webapps, WebApp};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{Model, ModelRc, SharedString, VecModel};
use smpl_common::keybindings;
use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

fn to_ui_item(app: &WebApp, hk_map: &std::collections::HashMap<String, (String, String)>) -> WebAppItem {
    let (label, combo) = hk_map.get(&app.slug)
        .cloned()
        .unwrap_or_default();
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
        has_hotkey: !label.is_empty(),
        hotkey_label: label.into(),
        hotkey_combo: combo.into(),
    }
}

/// Extract the webapp slug from a binding's args string, if it is a webapp binding.
///
/// Handles both quoted form:   `launch-webapp "--name" "slug" "https://..."`
/// and unquoted form:          `launch-webapp --name slug https://...`
///
/// Extracted so it can be unit-tested independently of the filesystem.
fn extract_webapp_slug(args: &str) -> Option<String> {
    if !args.contains("launch-webapp") {
        return None;
    }
    let pos = args.find("--name")?;
    let after = &args[pos + 6..];
    // Skip optional trailing quote of the flag token itself (e.g. "--name")
    let after = after.strip_prefix('"').unwrap_or(after).trim_start();
    let slug = if let Some(stripped) = after.strip_prefix('"') {
        stripped.split('"').next().unwrap_or_default()
    } else {
        after.split_whitespace().next().unwrap_or_default()
    };
    if slug.is_empty() { None } else { Some(slug.to_string()) }
}

/// Build the `exec` args string for a webapp keybinding.
/// This must include ALL flags from the webapp config so launching via hotkey
/// behaves identically to launching via the desktop file.
/// Regression guard: if this function omits flags, the binding will silently
/// ignore them (e.g. --clear-on-exit won't clear data when launched by hotkey).
fn build_launch_args(slug: &str, url: &str, secure: bool, clear_on_exit: bool, vpn: &str, vpn_required: bool) -> String {
    let mut parts = String::from("launch-webapp");
    if secure        { parts.push_str(" \"--secure\""); }
    if clear_on_exit { parts.push_str(" \"--clear-on-exit\""); }
    if vpn_required  { parts.push_str(" \"--vpn-required\""); }
    if !vpn.is_empty() {
        parts.push_str(&format!(" \"--vpn-interface\" \"{vpn}\""));
    }
    parts.push_str(&format!(" \"--name\" \"{slug}\" \"{url}\""));
    parts
}

/// Build a map of webapp slug → (short badge label, full combo display string).
fn hotkey_label_map() -> std::collections::HashMap<String, (String, String)> {
    let mut map = std::collections::HashMap::new();
    if let Ok(file) = keybindings::BindingsFile::load() {
        for kb in &file.bindings {
            if kb.dispatcher == "exec" {
                if let Some(slug) = extract_webapp_slug(&kb.args) {
                    let label = keybindings::humanize_key(&kb.key);
                    let combo = kb.combo_display();
                    map.insert(slug, (label, combo));
                }
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use smpl_common::keybindings;

    // ── Struct shape guard ────────────────────────────────────────────────────
    //
    // These tests use explicit struct literal syntax. If a field is added or
    // removed from WebAppItem (in main.slint), the test FAILS TO COMPILE,
    // which will block CI. This is intentional — it forces the developer to
    // update the tests (and think about whether the change is intentional).

    #[test]
    fn webapp_item_struct_has_all_fields_including_has_hotkey() {
        // Will not compile if `has_hotkey` or `hotkey_label` is removed from the Slint struct.
        let item = WebAppItem {
            name: "Test App".into(),
            slug: "test-app".into(),
            url: "https://test.example.com".into(),
            secure: false,
            clear_on_exit: false,
            vpn_iface: "".into(),
            vpn_required: false,
            icon: "test-icon".into(),
            marked: false,
            has_hotkey: true,
            hotkey_label: "N".into(),
            hotkey_combo: "Super+Ctrl+N".into(),
        };
        assert!(item.has_hotkey);
        assert_eq!(item.hotkey_label.as_str(), "N");
        assert_eq!(item.hotkey_combo.as_str(), "Super+Ctrl+N");

        let item2 = WebAppItem { has_hotkey: false, hotkey_label: "".into(), hotkey_combo: "".into(), ..item.clone() };
        assert!(!item2.has_hotkey);
        assert!(item2.hotkey_label.as_str().is_empty());
        assert!(item2.hotkey_combo.as_str().is_empty());
    }

    // ── Slug-extraction logic ─────────────────────────────────────────────────

    #[test]
    fn extract_webapp_slug_quoted_name() {
        // Form written by save_webapp: launch-webapp "--name" "slug" "https://..."
        let args = r#"launch-webapp "--name" "discord" "https://discord.com""#;
        assert_eq!(extract_webapp_slug(args).as_deref(), Some("discord"));
    }

    #[test]
    fn extract_webapp_slug_unquoted_name() {
        // Older / manually written form
        let args = "launch-webapp --name amazon https://www.amazon.com";
        assert_eq!(extract_webapp_slug(args).as_deref(), Some("amazon"));
    }

    #[test]
    fn extract_webapp_slug_hyphenated_name() {
        let args = r#"launch-webapp "--name" "work-chat" "https://chat.example.com""#;
        assert_eq!(extract_webapp_slug(args).as_deref(), Some("work-chat"));
    }

    #[test]
    fn extract_webapp_slug_returns_none_for_non_webapp() {
        assert_eq!(extract_webapp_slug("firefox https://example.com"), None);
        assert_eq!(extract_webapp_slug("terminal"), None);
        assert_eq!(extract_webapp_slug(""), None);
    }

    // ── to_ui_item keybinding badge ───────────────────────────────────────────

    #[test]
    fn to_ui_item_sets_has_hotkey_when_slug_present() {
        let app = backend::WebApp {
            name: "Discord".into(),
            slug: "discord".into(),
            url: "https://discord.com".into(),
            secure: false,
            clear_on_exit: false,
            vpn_iface: String::new(),
            vpn_required: false,
            icon: "discord".into(),
            desktop_file: std::path::PathBuf::new(),
        };

        let mut with_binding: std::collections::HashMap<String, (String, String)> = Default::default();
        with_binding.insert("discord".to_string(), ("D".to_string(), "Super+Shift+D".to_string()));

        let item = to_ui_item(&app, &with_binding);
        assert!(item.has_hotkey, "app whose slug is in the hotkey map must have has_hotkey = true");
        assert_eq!(item.hotkey_label.as_str(), "D", "hotkey_label must be the key letter");
        assert_eq!(item.hotkey_combo.as_str(), "Super+Shift+D", "hotkey_combo must be the full combo");

        let no_bindings: std::collections::HashMap<String, (String, String)> = Default::default();
        let item2 = to_ui_item(&app, &no_bindings);
        assert!(!item2.has_hotkey, "app not in hotkey map must have has_hotkey = false");
        assert!(item2.hotkey_label.as_str().is_empty(), "hotkey_label must be empty when no binding");
        assert!(item2.hotkey_combo.as_str().is_empty(), "hotkey_combo must be empty when no binding");
    }

    // ── smpl_common::keybindings API surface ──────────────────────────────────
    //
    // These tests reference types and functions from smpl_common::keybindings.
    // If any of these are removed or renamed, the test fails to compile.

    #[test]
    fn keybinding_type_and_combo_display() {
        // Will not compile if Keybinding struct is removed or fields renamed.
        let kb = keybindings::Keybinding {
            bind_type: "bindd".to_string(),
            mods: "SUPER SHIFT".to_string(),
            key: "D".to_string(),
            description: "Launch Discord".to_string(),
            dispatcher: "exec".to_string(),
            args: r#"launch-webapp "--name" "discord" "https://discord.com""#.to_string(),
            section: "Application Launchers".to_string(),
            submap: String::new(),
        };
        assert_eq!(kb.combo_display(), "Super+Shift+D");
        // Verify our slug extractor works on this exact args format
        assert_eq!(extract_webapp_slug(&kb.args).as_deref(), Some("discord"));
    }

    #[test]
    fn slint_key_to_hyprland_converts_letters() {
        // Will not compile if slint_key_to_hyprland is removed.
        assert_eq!(keybindings::slint_key_to_hyprland("a"), "A");
        assert_eq!(keybindings::slint_key_to_hyprland("z"), "Z");
        assert_eq!(keybindings::slint_key_to_hyprland("5"), "5");
    }

    #[test]
    fn slint_key_to_hyprland_converts_special_keys() {
        assert_eq!(keybindings::slint_key_to_hyprland("\n"), "RETURN");
        assert_eq!(keybindings::slint_key_to_hyprland(" "), "SPACE");
        assert_eq!(keybindings::slint_key_to_hyprland("\u{f700}"), "UP");
        assert_eq!(keybindings::slint_key_to_hyprland("\u{f701}"), "DOWN");
        // Unknown key should return empty string (triggers "Unknown key" UI warning)
        assert_eq!(keybindings::slint_key_to_hyprland("\u{ffff}"), "");
    }

    // ── build_launch_args regression guards ───────────────────────────────────
    //
    // These tests verify that the keybinding args string includes ALL webapp
    // flags.  If a flag is omitted, launching via hotkey silently ignores it
    // (e.g. --clear-on-exit won't wipe data, --secure won't disable extensions).
    // This was a real regression: bffa03c never included the flags, so the
    // feature was broken from day one and nobody noticed until users reported it.

    #[test]
    fn build_launch_args_includes_clear_on_exit() {
        let args = build_launch_args("chat", "https://example.com", false, true, "", false);
        assert!(args.contains("--clear-on-exit"),
            "binding args must contain --clear-on-exit when the flag is set, got: {args}");
    }

    #[test]
    fn build_launch_args_includes_secure() {
        let args = build_launch_args("chat", "https://example.com", true, false, "", false);
        assert!(args.contains("--secure"),
            "binding args must contain --secure when the flag is set, got: {args}");
    }

    #[test]
    fn build_launch_args_includes_vpn_flags() {
        let args = build_launch_args("myapp", "https://example.com", false, false, "wg0", true);
        assert!(args.contains("--vpn-interface"),
            "binding args must contain --vpn-interface when vpn is set, got: {args}");
        assert!(args.contains("wg0"),
            "binding args must contain the vpn interface name, got: {args}");
        assert!(args.contains("--vpn-required"),
            "binding args must contain --vpn-required when the flag is set, got: {args}");
    }

    #[test]
    fn build_launch_args_omits_flags_when_unset() {
        let args = build_launch_args("myapp", "https://example.com", false, false, "", false);
        assert!(!args.contains("--secure"),         "must not emit --secure when unset, got: {args}");
        assert!(!args.contains("--clear-on-exit"),  "must not emit --clear-on-exit when unset, got: {args}");
        assert!(!args.contains("--vpn"),            "must not emit vpn flags when unset, got: {args}");
    }

    #[test]
    fn build_launch_args_full_flags_matches_desktop_file_exec_format() {
        // Simulates the real Chat webapp: --secure --clear-on-exit --name "chat"
        let args = build_launch_args("chat", "https://www.bing.com/chat", true, true, "", false);
        // The keybinding args must contain the same flags as the desktop Exec= line.
        // launch-webapp parses these identically whether launched from desktop or hotkey.
        assert!(args.contains("--secure"),        "got: {args}");
        assert!(args.contains("--clear-on-exit"), "got: {args}");
        assert!(args.contains("--name"),          "got: {args}");
        assert!(args.contains("\"chat\""),        "got: {args}");
        assert!(args.contains("bing.com/chat"),   "got: {args}");
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

fn refresh_model(
    ui: &MainWindow,
    state: &Rc<RefCell<Vec<WebApp>>>,
    model: &Rc<VecModel<WebAppItem>>,
) {
    let apps = scan_webapps();
    *state.borrow_mut() = apps.clone();

    let hk_map = hotkey_label_map();
    model.set_vec(apps.iter().map(|a| to_ui_item(a, &hk_map)).collect::<Vec<_>>());
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
                        let explicitly_cleared = ui.get_form_hotkey_explicitly_cleared();

                        let launch_args = build_launch_args(
                            &slug, url.as_str(),
                            secure, clear_on_exit,
                            vpn.as_str(), vpn_required
                        );

                        if let Ok(mut file) = keybindings::BindingsFile::load() {
                            // Find old bindings for this webapp
                            let to_remove: Vec<usize> = file.bindings.iter().enumerate()
                                .filter(|(_, b)| {
                                    let p1 = format!("launch-webapp --name {slug}");
                                    let p2 = format!("\"--name\" \"{slug}\"");
                                    b.args.contains(&p1) || b.args.contains(&p2)
                                })
                                .map(|(i, _)| i)
                                .collect();

                            let had_old = !to_remove.is_empty();

                            if !hk_key.is_empty() || explicitly_cleared {
                                // User deliberately changed the hotkey — remove old and maybe add new.
                                for idx in to_remove.into_iter().rev() {
                                    file.remove(idx);
                                }

                                if !hk_key.is_empty() {
                                    // Remove any conflicting binding that uses the same key combo
                                    if let Some(conflict) = file.find_conflict(hk_mods.trim(), &hk_key, "", None) {
                                        file.remove(conflict.index);
                                    }

                                    file.add(keybindings::Keybinding {
                                        bind_type: "bindd".to_string(),
                                        mods: hk_mods.trim().to_string(),
                                        key: hk_key,
                                        description: format!("Launch {}", name),
                                        dispatcher: "exec".to_string(),
                                        args: launch_args,
                                        section: "Application Launchers".to_string(),
                                        submap: String::new(),
                                    });
                                }

                                let _ = file.save_and_reload();
                            } else if had_old {
                                // Hotkey key unchanged, but app flags (secure/clear-on-exit/vpn)
                                // may have changed — update the args in-place so the binding
                                // stays accurate without requiring the user to re-assign the key.
                                for &idx in &to_remove {
                                    file.bindings[idx].args = launch_args.clone();
                                    file.bindings[idx].description = format!("Launch {}", name);
                                }
                                let _ = file.save_and_reload();
                            }
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
