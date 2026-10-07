const UI: &str = include_str!("../ui/main.slint");

#[test]
fn backgrounds_use_glass_without_fading_foreground_controls() {
    assert!(UI.contains("background: Theme.bg.transparentize(1.0 - Theme.opacity)"));
    assert!(UI.contains("panel: bg_light.transparentize(88%)"));
    assert!(UI.matches("background: Theme.panel;").count() >= 40);
    assert!(!UI.lines().any(|line| line.trim_start().starts_with("opacity:")));
    for semantic_state in [
        "model-row.disabled ? Theme.fg_dim : Theme.fg",
        "root.wifi-scanning ? Theme.fg_dim : Theme.fg",
        "root.wifi-connecting ? Theme.fg_dim : Theme.bg",
        "root.bt-scanning ? Theme.fg_dim : Theme.fg",
        "root.bt-connecting ? Theme.fg_dim : Theme.bg",
    ] {
        assert!(UI.contains(semantic_state));
    }
}

#[test]
fn help_replaces_content_without_another_background_fill() {
    assert!(UI.contains("visible: !root.show-help;"));
    let help = UI
        .split("// ── Help overlay")
        .nth(1)
        .unwrap()
        .split("Flickable {")
        .next()
        .unwrap();
    assert!(help.contains("background: transparent;"));
    assert!(!help.contains("background: Theme.bg;"));
}

#[test]
fn readable_typography_uses_shared_tokens_without_tiny_text() {
    for token in [
        "caption: 12px",
        "secondary: 13px",
        "body: 14px",
        "heading: 16px",
        "title: 18px",
        "compact-control: 32px",
    ] {
        assert!(UI.contains(token), "missing typography token: {token}");
    }
    for size in 8..12 {
        assert!(!UI.contains(&format!("font-size: {size}px")));
    }
    assert!(UI.contains("min-width: 500px"));
    assert!(UI.contains("min-height: 350px"));
}

#[test]
fn power_controls_show_custom_duration_without_saved_or_debug_preface() {
    let power = UI
        .split("// POWER TAB")
        .nth(1)
        .unwrap()
        .split("// KEYBINDINGS TAB")
        .next()
        .unwrap();
    assert!(!power.contains("Saved:"));
    assert!(!power.contains("Saved preferences"));
    assert!(!power.contains("Active rules"));
    for action in ["lock", "dpms", "suspend", "shutdown"] {
        assert!(power.contains(&format!(
            "root.idle-{action}-index == -1 ? root.idle-{action}-custom-label"
        )));
    }
    for status in ["config", "runtime", "profile", "action"] {
        assert!(power.contains(&format!("if root.power-{status}-status != \"\": Text")));
    }
}

#[test]
fn sidebar_uses_theme_tinted_outline_assets_only_for_requested_icons() {
    let sidebar = UI
        .split("// ── Left sidebar")
        .nth(1)
        .unwrap()
        .split("// Sidebar separator")
        .next()
        .unwrap();
    for (name, svg) in [
        ("wifi", include_str!("../ui/assets/wifi-outline.svg")),
        (
            "bluetooth",
            include_str!("../ui/assets/bluetooth-outline.svg"),
        ),
        ("hints", include_str!("../ui/assets/hints-outline.svg")),
    ] {
        assert!(sidebar.contains(&format!("assets/{name}-outline.svg")));
        assert!(svg.contains("fill=\"none\""));
        assert!(svg.contains("stroke-width=\"1.7\""));
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
    }
    assert_eq!(sidebar.matches("use-outline-icon: true").count(), 3);
    assert!(UI.contains("colorize: active ? Theme.accent : Theme.fg_dim"));
}

#[test]
fn every_tab_and_sidebar_remain_scrollable_at_minimum_window_size() {
    assert!(UI.contains("sidebar-flick := Flickable"));
    for tab in 0..=11 {
        assert!(UI.contains(&format!("tab{tab}-flick := Flickable")));
        assert!(UI.contains(&format!("viewport-x <=> tab{tab}-flick.viewport-x")));
        assert!(
            UI.contains(&format!("viewport-height: tab{tab}-flick.viewport-height"))
                || (tab == 9 && UI.contains("tab9-flick.viewport-height > tab9-flick.height"))
        );
    }
}

#[test]
fn dropdown_keyboard_scrolling_uses_actual_row_and_viewport_dimensions() {
    assert!(!UI.contains("* 27px + 80px"));
    assert_eq!(
        UI.matches("viewport-y = root.dropdown-scroll-offset(")
            .count(),
        4
    );
    assert_eq!(
        UI.matches("Typography.compact-control / 1px, Typography.dropdown-spacing / 1px")
            .count(),
        4
    );
    for name in ["dropdown-flick", "dict-lang-flick"] {
        assert_eq!(
            UI.matches(&format!(
                "{name}.height / 1px, {name}.viewport-height / 1px"
            ))
            .count(),
            2
        );
    }
}

#[test]
fn hints_scrollbars_are_fixed_siblings_of_the_scrolling_viewport() {
    let hints = UI
        .split("if root.active-tab == 10: Rectangle {")
        .nth(1)
        .unwrap();
    let flick_start = hints.find("tab10-flick := Flickable {").unwrap();
    let body_start = flick_start + hints[flick_start..].find('{').unwrap();
    let mut depth = 0;
    let mut end = None;
    for (offset, c) in hints[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(body_start + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end.unwrap();
    assert!(hints.find("HorizontalScrollIndicator {").unwrap() > end);
    assert!(hints.find("ScrollIndicator {").unwrap() > end);
}

/// The body of a top-level `component <name>` declaration.
fn component(name: &str) -> &'static str {
    let start = UI
        .find(&format!("\ncomponent {name} inherits"))
        .unwrap_or_else(|| panic!("component {name} missing"));
    let rest = &UI[start + 1..];
    let end = rest[1..]
        .find("\ncomponent ")
        .map_or(rest.len(), |offset| offset + 1);
    &rest[..end]
}

fn display_tab() -> &'static str {
    UI.split("// DISPLAY TAB")
        .nth(1)
        .unwrap()
        .split("// POWER TAB")
        .next()
        .unwrap()
}

#[test]
fn dropdown_and_slider_inputs_are_controlled_and_never_assigned_internally() {
    let dropdown = component("ResDropdown");
    assert!(dropdown.contains("in property <int> current-index"));
    assert!(!dropdown.contains("in-out property <int> current-index"));
    assert!(!dropdown.contains("current-index ="), "ResDropdown assigns current-index");
    assert!(dropdown.contains("root.selected(idx);"));

    let slider = component("ThemeSlider");
    assert!(slider.contains("in property <float> value"));
    assert!(!slider.contains("in-out property <float> value"));
    for assignment in ["root.value =", "root.value +=", "value = clamp", "value = round"] {
        assert!(!slider.contains(assignment), "ThemeSlider assigns its value: {assignment}");
    }
    assert!(slider.contains("root.changed(root.value-at(self.mouse-x));"));
    // The fill starts at the left edge, like the knob.
    let fill = slider.split("// Starts at the left edge").nth(1).unwrap();
    let fill = fill.split_once('\n').unwrap().1.trim_start();
    assert!(fill.starts_with("Rectangle {\n            x: 0;"), "{fill}");

    // XR sliders keep working by storing the emitted value themselves.
    for (property, callback) in [
        ("xr-radius", "xr-set-radius"),
        ("xr-fov", "xr-set-fov"),
        ("xr-curvature", "xr-set-curvature"),
        ("xr-smoothing", "xr-set-smoothing"),
    ] {
        assert!(UI.contains(&format!("value: root.{property};")));
        assert!(UI.contains(&format!("changed(v) => {{ root.{property} = v; root.{callback}(v); }}")));
    }
}

#[test]
fn display_controls_read_the_selected_row_of_disp_monitors() {
    for (property, field) in [
        ("[string]> disp-selected-modes", "available-modes"),
        ("int> disp-selected-mode-index", "current-mode-index"),
        ("float> disp-selected-scale", "scale"),
        ("[string]> disp-selected-orientations", "orientation-options"),
        ("int> disp-selected-orientation", "current-orientation-index"),
    ] {
        let declaration = format!(
            "out property <{property}: root.disp-selection-valid ? root.disp-monitors[root.disp-selected-index].{field} :"
        );
        assert!(UI.contains(&declaration), "missing derived {property}");
    }
    let tab = display_tab();
    for binding in [
        "model: root.disp-selected-modes;\n                                    current-index: root.disp-selected-mode-index;",
        "value: root.disp-selected-scale;",
        "model: root.disp-selected-orientations;\n                                    current-index: root.disp-selected-orientation;",
        "sublabel: mon.size-label;",
        "sublabel-two-line: mon.size-label-two-line;",
    ] {
        assert!(tab.contains(binding), "missing display binding: {binding}");
    }
    // The orientation list comes from the row (it can name transforms 4–7).
    assert!(!tab.contains("model: [\"Landscape\""));
    // Nothing in the tab writes a displayed value; only the selection index.
    for property in ["disp-selected-scale", "disp-selected-orientation", "disp-selected-mode-index", "disp-monitors"] {
        assert!(!UI.contains(&format!("root.{property} =")), "{property} assigned in UI");
    }
}

#[test]
fn duplicated_display_selection_setters_are_gone() {
    let rust = [
        include_str!("main.rs"),
        include_str!("display/ui.rs"),
        include_str!("display/model.rs"),
    ];
    for setter in [
        "set_disp_selected_orientation",
        "set_disp_selected_scale",
        "set_disp_selected_mode_index",
        "set_disp_selected_modes",
    ] {
        assert!(rust.iter().all(|source| !source.contains(setter)), "{setter} still exists");
    }
    for property in [
        "disp-selected-orientation:",
        "disp-selected-scale:",
        "disp-selected-mode-index:",
        "disp-selected-modes:",
    ] {
        for kind in ["in property <", "in-out property <"] {
            assert!(
                !UI.lines().any(|line| line.contains(kind) && line.contains(property)),
                "{property} is still writable"
            );
        }
    }
    assert!(include_str!("display/ui.rs").contains("ui.set_disp_selected_index(controller.model.selected_index());"));
}

#[test]
fn display_tab_never_shows_unverified_or_stale_state_as_applicable() {
    assert!(UI.contains("if root.active-tab == 3 { root.disp-tab-entered(); }"));
    assert!(UI.contains(
        "out property <bool> disp-can-apply: root.disp-has-changes && !root.disp-busy && !root.disp-stale;"
    ));
    let tab = display_tab();
    assert!(tab.contains("enabled: root.disp-can-apply;"));
    assert!(tab.contains("enabled: root.disp-can-revert;"));
    assert!(tab.contains("if !root.disp-available: Rectangle"));
    assert!(tab.contains("text: root.disp-unavailable-reason;"));
    assert!(tab.contains("if root.disp-stale: Rectangle"));
    assert!(tab.contains("if root.disp-drift-text != \"\": Rectangle"));
    assert!(tab.contains("clicked => { root.disp-use-saved(); }"));
    assert!(tab.contains("clicked => { root.disp-keep-current(); }"));
    assert!(tab.contains("if !root.disp-workspace-policy: VerticalLayout"));
    assert!(tab.contains("Move workspace 1 here"));
    let rect = component("MonitorRect");
    assert!(rect.contains("private property <bool> narrow: size-measure.preferred-width + 8px > root.width;"));
    assert!(rect.contains("text: root.narrow ? root.sublabel-two-line : root.sublabel;"));
    assert!(!rect.contains("overflow: elide"));
}

fn dictation_tab() -> &'static str {
    UI.split("// DICTATION TAB")
        .nth(1)
        .unwrap()
        .split("// DISPLAY TAB")
        .next()
        .unwrap()
}

#[test]
fn dictation_says_when_the_model_is_missing_and_how_to_get_it() {
    let tab = dictation_tab();
    for binding in [
        "if root.dictation-model-problem != \"\": Rectangle",
        "text: root.dictation-model-problem;",
        "text: root.dictation-model-help;",
        "if root.dictation-model-downloadable: Rectangle",
        "clicked => { root.download-dictation-model(); }",
        // Starting the service can't help while the model is missing.
        "if !root.dictation-service-running && !root.dictation-config-missing && root.dictation-model-problem == \"\": Rectangle",
        // Setup and Reconfigure say when the selected model still has to be downloaded.
        "if root.dictation-selected-model.label != \"\" && !root.dictation-selected-model.downloaded: Text",
        ": root.dictation-selected-model.downloaded ? \"Save\" : \"Save & Download Model\";",
    ] {
        assert!(tab.contains(binding), "missing dictation binding: {binding}");
    }
    assert!(UI.contains(
        "property <ModelEntry> dictation-selected-model: root.dictation-model-list[root.dictation-selected-model-idx];"
    ));
    let rust = include_str!("main.rs");
    assert!(rust.contains("dictation::launch_model_download(&cfg.model)"));
    // Startup, the status poll and the end of an install all refresh model state.
    assert_eq!(rust.matches("refresh_dictation_models(&ui, cfg.as_ref());").count(), 3);
}

#[test]
fn keypad_tab_is_wired_and_flashes_only_as_a_dry_run_by_default() {
    let keypad = UI
        .split("// KEYPAD TAB")
        .nth(1)
        .unwrap()
        .split("// ABOUT TAB")
        .next()
        .unwrap();
    for needle in [
        "clicked(id) => { root.kp-select-control(id); }",
        "clicked => { root.kp-save(); }",
        "clicked => { root.kp-flash(); }",
        "root.kp-dry-run ? \"Flash (dry run)\" : \"Flash\"",
        "I understand that the stock firmware can't be restored",
        "Hold down the top-left key",
        "for plugin[idx] in [\"None (key mapping)\", \"Kdenlive (D-Bus API)\"]",
        "for event[idx] in [\"Turn left\", \"Turn right\", \"Press\"]",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    assert!(!keypad.contains("Theme.bg.transparentize"), "cards use Theme.panel, not another window fill");
    assert!(UI.contains("label: \"Keypad\""));
    assert!(UI.contains("if root.active-tab == 11 { root.kp-refresh(); }"));

    let main = include_str!("main.rs");
    assert!(main.contains("\"keypad\" => 11,"));
    assert!(main.contains("(\"Keypad\", \"keypad\", 11)"));
    assert!(main.contains("keypad::ui::install(&ui)"));
    let keypad_rs = include_str!("keypad/mod.rs");
    assert!(keypad_rs.contains("std::env::var(\"SMPLOS_KEYPAD_REAL_FLASH\").as_deref() == Ok(\"1\")"));
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("let execute = super::real_flash_enabled();"));
}

fn keypad_page() -> &'static str {
    UI.split("// KEYPAD TAB").nth(1).unwrap().split("// ABOUT TAB").next().unwrap()
}

#[test]
fn keypad_tab_states_its_device_scope_first() {
    let keypad = keypad_page();
    let scope = keypad.find("// ── Which keypads this supports").expect("scope note");
    let device = keypad.find("// ── Device: keypad (sysfs) and the keypad app").unwrap();
    assert!(scope < device, "the scope note comes before the device card");
    for needle in [
        "Supports CH552-based macro keypads only: USB ID 1189:8890",
        "\\\"MINI KeyBoard\\\"-style pads with 3 to 16 keys and up to 3 knobs",
        "a supported keypad shows up as connected just below",
        "keypad-ctl present (or lsusb) finds USB ID 1189:8890",
        "clicked => { root.kp-open-scope-help(); }",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("KEYPAD.md#which-keypads-work"));
}

#[test]
fn keypad_app_status_offers_start_only_when_told_to() {
    let keypad = keypad_page();
    assert!(keypad.contains("text: root.kp-app-text;"));
    assert!(keypad.contains("if root.kp-show-start: KeypadButton {"));
    assert!(keypad.contains("clicked => { root.kp-start-app(); }"));
    for gone in ["kp-service-text", "kp-service(\"restart\")", "Keypad service", "detection"] {
        assert!(!keypad.contains(gone), "stale: {gone}");
    }
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("super::app_summary(&st.status.app, pad.is_some())"));
    let mod_rs = include_str!("keypad/mod.rs");
    assert!(mod_rs.contains("\"--user\", \"start\", SERVICE"));
    assert!(!mod_rs.contains("\"status\", \"--service\""), "status comes from sysfs, not keypad-ctl");
}

#[test]
fn keypad_variant_picker_has_previews_custom_grid_and_detected_mode() {
    let keypad = keypad_page();
    for needle in [
        "if root.kp-variant-detected != \"\": HorizontalLayout {",
        "\"Detected from the keypad: \" + root.kp-variant-detected",
        "clicked => { root.kp-override-variant(); }",
        "if root.kp-variant-picker: KeypadVariantDropdown {",
        "selected(i) => { root.kp-select-variant(i); }",
        "if root.kp-variant-picker && root.kp-custom: HorizontalLayout {",
        "label: \"Keys\";",
        "label: \"Knobs\";",
        "label: \"Columns\";",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    let dropdown = UI.split("component KeypadVariantDropdown").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    assert_eq!(dropdown.matches("KeypadMiniPad {").count(), 2, "a schematic for the choice and for every entry");
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("menu.push(entry(\"Custom…\".into(), ck, cn, cc));"));
}
