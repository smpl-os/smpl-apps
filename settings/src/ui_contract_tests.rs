const UI: &str = include_str!("../ui/main.slint");

#[test]
fn mouse_identify_and_learning_have_live_controls_and_diagram_selection() {
    assert!(UI.contains("root.ms-identify(!root.ms-identifying)"));
    assert!(UI.contains("root.ms-learn()"));
    assert!(UI.contains("root.ms-cancel-learn()"));
    assert!(UI.contains("enabled: root.ms-backend-ready;"));
    assert!(!UI.contains("Button assignments are a preview:"));
    let wiring = include_str!("mouse/ui.rs");
    assert!(wiring.contains("st.buttons.select_input(&status.last_slot)"));
    assert!(wiring.contains("Command::Leave"));
    assert!(wiring.contains("st.buttons.reread()"));
}

#[test]
fn side_grid_calibration_is_in_mouse_buttons_not_the_keypad_wizard() {
    let buttons = UI
        .split("ms-buttons-card := Rectangle {")
        .nth(1)
        .unwrap()
        .split("// ── Assignment:")
        .next()
        .unwrap();
    let calibration = buttons.find("clicked => { root.ms-calibrate(); }").unwrap();
    assert!(calibration < buttons.find("DeviceCanvas {").unwrap());
    assert_eq!(UI.matches("clicked => { root.ms-calibrate(); }").count(), 1);
}

#[test]
fn full_mouse_setup_has_visible_entry_and_confirmation_controls() {
    let buttons = UI
        .split("ms-buttons-card := Rectangle {")
        .nth(1)
        .unwrap()
        .split("// ── Assignment:")
        .next()
        .unwrap();
    assert!(buttons.contains("root.ms-start-setup()"));
    let wizard = UI.split("if root.ms-setup-active: Rectangle {").nth(1).unwrap()
        .split("// ── Help overlay").next().unwrap();
    for action in ["arm", "retry", "skip", "next", "save", "cancel"] {
        assert!(wizard.contains(&format!("root.ms-{action}-setup()")), "missing visible setup action: {action}");
    }
    assert!(wizard.contains("root.ms-setup-phase == \"review\""));
    assert!(UI.contains("else if root.ms-setup-active {\n            root.ms-cancel-setup();"));
}

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
        (
            "connections",
            include_str!("../ui/assets/connections-outline.svg"),
        ),
        ("hints", include_str!("../ui/assets/hints-outline.svg")),
    ] {
        assert!(sidebar.contains(&format!("assets/{name}-outline.svg")));
        assert!(svg.contains("fill=\"none\""));
        assert!(svg.contains("stroke-width=\"1.7\""));
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
    }
    assert_eq!(sidebar.matches("use-outline-icon: true").count(), 2);
    // Wi-Fi and Bluetooth are sections of Connections now: their icons mark the
    // section switcher, not the sidebar.
    for (name, svg) in [
        ("wifi", include_str!("../ui/assets/wifi-outline.svg")),
        (
            "bluetooth",
            include_str!("../ui/assets/bluetooth-outline.svg"),
        ),
    ] {
        let asset = format!("assets/{name}-outline.svg");
        assert!(!sidebar.contains(&asset));
        assert!(UI.contains(&asset));
        assert!(svg.contains("fill=\"none\""));
        assert!(svg.contains("stroke-width=\"1.7\""));
        assert!(svg.contains("viewBox=\"0 0 24 24\""));
    }
    assert!(UI.contains("colorize: active ? Theme.accent : Theme.fg_dim"));
}

#[test]
fn every_tab_and_sidebar_remain_scrollable_at_minimum_window_size() {
    assert!(UI.contains("sidebar-flick := Flickable"));
    for tab in 0..=12 {
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
    assert!(tab.contains("if root.disp-primary-settable: VerticalLayout"));
    assert!(tab.contains("Move workspace 1 here"));
    let rect = component("MonitorRect");
    assert!(rect.contains("private property <bool> narrow: size-measure.preferred-width + 8px > root.width;"));
    assert!(rect.contains("text: root.narrow ? root.sublabel-two-line : root.sublabel;"));
    assert!(!rect.contains("overflow: elide"));
}

#[test]
fn display_primary_monitor_can_be_chosen_in_place_with_or_without_the_policy() {
    let tab = display_tab();
    // No dead end: the card never sends the user to the Taskbar tab.
    assert!(!UI.contains("Taskbar settings"));
    for source in [
        include_str!("display/ui.rs"),
        include_str!("display/primary.rs"),
    ] {
        assert!(!source.contains("Change it in Taskbar settings"));
    }
    // The text is built in Rust and picked by whether the selection is primary.
    assert!(tab.contains("? root.disp-primary-note-here"));
    assert!(tab.contains(": root.disp-primary-note-elsewhere;"));
    // The button shows whenever the primary can be changed; under the policy it
    // says what it does, without it it keeps the session-only wording.
    assert!(tab.contains(
        "text: root.disp-workspace-policy ? \"Make primary\" : \"Move workspace 1 here\";"
    ));
    assert!(tab.contains("root.disp-set-primary(root.disp-selected-index);"));
    // The button and Alt+P are governed by the same state.
    assert!(UI.contains(
        "out property <bool> disp-primary-movable: root.disp-selection-valid && root.disp-primary-settable && !root.disp-busy"
    ));
    assert!(UI.contains(
        "if e.text == \"p\" && root.disp-selection-valid && root.disp-primary-settable && !root.disp-busy {"
    ));
    // The state is pushed from Rust only, and the help overlay still names the key.
    let ui_rs = include_str!("display/ui.rs");
    for property in [
        "disp-primary-settable",
        "disp-primary-note-here",
        "disp-primary-note-elsewhere",
    ] {
        assert!(
            UI.lines()
                .any(|line| line.contains("in property <") && line.contains(&format!("{property}:"))),
            "{property} is not an in property"
        );
        assert!(
            ui_rs.contains(&format!("ui.set_{}(", property.replace('-', "_"))),
            "{property} is never set from Rust"
        );
    }
    assert!(UI.contains("Alt+P   Move workspace 1 here"));
    // The deep link and the search entry that lead here keep working.
    assert!(tab.contains("root.highlight-setting == \"Primary Monitor\""));
    assert!(include_str!("main.rs").contains("(\"Primary Monitor\", \"display\", 3)"));
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
    let keypad = keypad_page();
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
    UI.split("// KEYPAD TAB").nth(1).unwrap().split("// MOUSE TAB").next().unwrap()
}

// The source of one top-level component, up to the next top-level component.
fn component_source(name: &str) -> &'static str {
    let body = UI
        .split(&format!("component {name} inherits"))
        .nth(1)
        .unwrap_or_else(|| panic!("no component {name}"));
    let end = ["\ncomponent ", "\nexport component "]
        .iter()
        .filter_map(|next| body.find(next))
        .min()
        .unwrap_or(body.len());
    &body[..end]
}

// The editor rows live once, in the shared components, and every tab that
// assigns actions binds them instead of repeating the markup.
fn assert_uses_shared_assignment_components(page: &str, tab: &str) {
    for component in ["AssignProfileEditor {", "AssignActionEditor {", "AssignBindingList {"] {
        assert_eq!(page.matches(component).count(), 1, "{tab}: {component}");
    }
    for repeated in [
        "text: \"Window class\";",
        "text: \"Window title\";",
        "text: \"Only if installed\";",
        "Apply to control",
        "for row in ",
    ] {
        assert!(!page.contains(repeated), "{tab} repeats a shared row: {repeated}");
    }
}

#[test]
fn assignment_rows_are_shared_components_not_copies() {
    assert_uses_shared_assignment_components(keypad_page(), "Keypad");
    assert_uses_shared_assignment_components(mouse_page(), "Mouse");
    let profile = component_source("AssignProfileEditor");
    for row in ["text: \"Profile\";", "text: \"Window class\";", "text: \"Window title\";", "text: \"Add app\";"] {
        assert!(profile.contains(row), "AssignProfileEditor: {row}");
    }
    let action = component_source("AssignActionEditor");
    for row in ["text: \"Action\";", "text: \"Only if installed\";", "@children"] {
        assert!(action.contains(row), "AssignActionEditor: {row}");
    }
    assert!(component_source("AssignBindingList").contains("for row in root.rows: Rectangle {"));
}

#[test]
fn keypad_tab_states_its_device_scope_first() {
    let keypad = keypad_page();
    let scope = keypad.find("// ── Which keypads this supports").expect("scope note");
    let device = keypad.find("// ── Device: keypad (sysfs) and the keypad app").unwrap();
    assert!(scope < device, "the scope note comes before the device card");
    for needle in ["text: root.kp-scope-text;", "text: root.kp-scope-help;", "clicked => { root.kp-open-scope-help(); }"] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    // The text comes from smplOS's keypad registry (generated registry.rs).
    use crate::keypad::registry::{SCOPE, SCOPE_HELP, SUPPORTED_IDS};
    assert!(SCOPE.starts_with("Supports CH552 macro keypads (USB ID 1189:8890)"), "{SCOPE}");
    assert!(SCOPE.contains("\"MINI KeyBoard\"-style pads with 3 to 16 keys and up to 3 knobs"));
    assert!(SCOPE.contains("Planned, not working yet: Elgato Stream Deck."), "planned devices are named, not supported");
    assert!(SCOPE_HELP.contains("keypad-ctl present (or lsusb) finds USB ID 1189:8890"));
    assert_eq!(SUPPORTED_IDS, [("1189", "8890")], "a planned Stream Deck is not detected as supported");
    assert!(!SUPPORTED_IDS.iter().any(|(v, _)| *v == "0fd9"));
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

#[test]
fn keypad_advanced_reaches_what_was_hand_edited() {
    let keypad = keypad_page();
    let advanced = keypad.split("// ── Advanced: what would otherwise be hand-edited in the file ──").nth(1).unwrap()
        .split("// ── Save ──").next().unwrap();
    for needle in [
        r#"for mode[idx] in ["Automatic", "Keymap (compatible)", "Raw (fastest, firmware 2.0.2+)"]: Rectangle {"#,
        "clicked => { root.kp-set-input-mode(idx); }",
        "text: root.kp-input-now;",
        "text: root.kp-input-health;",
        r#"text: root.kp-input-healthy ? "\u{2713}" : "!";"#,
        "for t in root.kp-knob-tunings: VerticalLayout {",
        "for t in root.kp-kdenlive-tunings: VerticalLayout {",
        "changed(v) => { root.kp-set-tuning(t.key, v); }",
        "clicked => { root.kp-reset-tunings(); }",
        "toggled(on) => { root.kp-set-sheet-overlay(on); }",
        "clicked => { root.kp-advanced-open = !root.kp-advanced-open; }",
    ] {
        assert!(advanced.contains(needle), "advanced: {needle}");
    }
    assert!(UI.contains("in-out property <bool> kp-advanced-open: false;"), "collapsed by default");
    // The profile and action editors are shared with the Mouse tab: the page
    // binds them to kp-* and the components own the rows.
    for needle in [
        "profile-name <=> root.kp-profile-name;",
        "window-title <=> root.kp-profile-title;",
        "set-fallthrough(on) => { root.kp-set-fallthrough(on); }",
        "needs-text <=> root.kp-needs-text;",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    let profile_editor = component_source("AssignProfileEditor");
    for needle in [
        "text <=> root.profile-name;",
        "text <=> root.window-title;",
        "toggled(on) => { root.set-fallthrough(on); }",
    ] {
        assert!(profile_editor.contains(needle), "profile editor: {needle}");
    }
    assert!(component_source("AssignActionEditor").contains("text <=> root.needs-text;"));
    assert!(!UI.contains("kp-apply-profile-class"), "one Apply for name, class and title");
    assert!(keypad.contains("clicked => { root.kp-remove-shift(); }"), "unsupported shift bindings can be removed");
    for needle in [
        "clicked => { root.kp-convert-shift(); }",
        "selected(i) => { root.kp-select-layer(i); }",
        "clicked => { root.kp-start-held-layer(); }",
        "clicked => { root.kp-cancel-held-layer(); }",
        "clicked => { root.kp-remove-layer(); }",
        "locked: root.kp-editor-locked;",
        "mode: root.kp-editor-mode;",
        "title: root.kp-bindings-title;",
        "text: root.kp-sheet-preview-caption;",
        "hold-glyph: root.kp-hold-glyph;",
        "clicked => { root.kp-move-layer-first(); }",
        "text: root.kp-picking-note;",
    ] {
        assert!(keypad.contains(needle), "held layers: {needle}");
    }
    let action_editor = component_source("AssignActionEditor");
    for needle in [
        "enabled: !root.locked && (root.mode != 3 || root.action-names.length > 1);",
        "if root.mode != 3 && !root.locked: KeypadButton {",
    ] {
        assert!(action_editor.contains(needle), "locked editor: {needle}");
    }
    let canvas = UI.split("component KeypadCanvas").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    for needle in ["if control.held: Text {", r#"text: "held";"#, "if control.holds && !control.seen: Text {", "font-italic: control.blocked;"] {
        assert!(canvas.contains(needle), "canvas: {needle}");
    }
    let advanced_rs = include_str!("keypad/ui.rs");
    // "held" is a condition like any other (control-surface d78e2bf): nothing
    // may tell the user a held layer wins regardless of order.
    for claim in ["own held layers win", "win over every other", "always win"] {
        assert!(!advanced_rs.contains(claim), "{claim}");
    }
    assert!(advanced_rs.contains("Layers apply in list order"));
    for needle in ["fn direct_route(", "super::set_option(k, v)", "wizard_input_begin(st)", "wizard_input_end(st, w.restore_input.take())"] {
        assert!(advanced_rs.contains(needle), "{needle}");
    }
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("super::input_summary(st.status.input.as_ref(), st.status.app.running(), firmware, &mode)"));
    assert!(ui_rs.contains("(set in the file)"), "a file's custom auto-hide value is shown, not blank");
}

#[test]
fn keypad_cheatsheet_card_label_and_preview_are_wired() {
    let keypad = keypad_page();
    for needle in [
        "// ── Cheatsheet ──",
        "if root.kp-label-enabled && root.kp-sheet-supported: HorizontalLayout {",
        "text <=> root.kp-label-text;",
        "changed(v) => { root.kp-set-sheet-opacity(v); }",
        "selected(i) => { root.kp-set-sheet-hide(i); }",
        "selected(i) => { root.kp-set-sheet-position(i); }",
        "selected(i) => { root.kp-set-sheet-context(i); }",
        "clicked => { root.kp-show-sheet(); }",
        "if root.kp-sheet-supported: KeypadSheetPreview {",
        "toggled(on) => { root.kp-set-sheet-click-through(on); }",
        ": \"Click anywhere on the overlay to close it.\";",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    assert!(UI.contains("in property <float> kp-sheet-opacity: 0.35;"), "the default uses the theme-colored frosted baseline");
    let preview = UI.split("component KeypadSheetPreview").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    assert!(preview.contains("background: Theme.popup-bg.transparentize(1 - min(1, min(Theme.popup-opacity, 0.35) * root.intensity));"), "frosted popup-role alpha on the background only");
    assert!(preview.contains("font-italic: c.bound && !c.active;"), "inactive is shown by shape, not color alone");
    assert!(keypad.contains("viewport-width: max(self.width, 640px, tab11-content.min-width);"), "wide previews scroll");
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains("super::sheet_preview(daemon.as_deref(), &scratch, &text, &class, &context)"));
    let on_input = ui_rs.split("fn on_input(").nth(1).unwrap().split("\nfn ").next().unwrap();
    assert!(on_input.contains("sync_editor_text(ui);"), "identify must refresh the editor's text fields");
    assert!(ui_rs.contains("if (self.sheet && config::sheet_slot(&self.slot()))"));
    for needle in [
        "if root.kp-label-enabled && root.kp-sheet-supported && root.kp-icon-picker-open: KeypadIconPicker {",
        "picked(name) => { root.kp-pick-icon(name); }",
        "search(t) => { root.kp-search-icons(t); }",
        "clicked => { root.kp-toggle-icon-picker(); }",
        "if root.kp-sheet-supported && !root.kp-icons-shown: Text {",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    let picker = UI.split("component KeypadIconPicker").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    for needle in ["clicked => { root.picked(\"\"); }", "clicked => { root.picked(\"none\"); }", "font-family: KeypadIcons.family;"] {
        assert!(picker.contains(needle), "picker: {needle}");
    }
    // Icons appear wherever bindings do: layout, binding list and preview.
    let canvas = UI.split("component KeypadCanvas").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    assert!(canvas.contains("text: control.glyph;"));
    let preview = UI.split("component KeypadSheetPreview").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    assert!(preview.contains("text: c.glyph;") && preview.contains("for line[i] in c.lines: Rectangle {"));
    assert!(component_source("AssignBindingList").contains(r#"text: row.warning ? "!" : row.glyph;"#));
    let show = ui_rs.split("ui.on_kp_show_sheet(").nth(1).unwrap().split("\n    let weak").next().unwrap();
    assert!(show.contains("super::hide_sheet()"), "Show on screen never leaves a sheet up for good");
    assert!(ui_rs.contains("o.click_through &= *ms > 0;"), "until hidden needs a clickable sheet");
}

#[test]
fn keypad_theme_intensity_preserves_storage_and_uses_popup_theme_updates() {
    let keypad = keypad_page();
    for needle in [
        r#"text: "Theme intensity";"#,
        r#"text: round(KeypadSheetStyle.intensity(root.kp-sheet-opacity) * 100) + "%";"#,
        "100% uses the theme's popup opacity capped at 35%; text stays opaque. Higher intensity can reach fully opaque.",
        "minimum: 0.05;",
        "maximum: 1.0;",
        "step: 0.05;",
        "value: root.kp-sheet-opacity;",
        "sheet-opacity: root.kp-sheet-opacity;",
    ] {
        assert!(keypad.contains(needle), "missing: {needle}");
    }
    assert!(!keypad.contains(r#"text: "Opacity";"#));
    let preview = UI.split("component KeypadSheetPreview").nth(1).unwrap().split("\ncomponent ").next().unwrap();
    assert!(preview.contains("property <float> intensity: KeypadSheetStyle.intensity(root.sheet-opacity);"));
    assert!(!preview.contains("Theme.opacity"), "Settings' application alpha is not popup alpha");
    assert!(!preview.contains("Theme.bg."), "the popup uses the unmodified theme background");
    let main = include_str!("main.rs");
    let popup = main.split("theme::watch(ThemeRole::Popup,").nth(1).unwrap().split("ui.set_active_tab").next().unwrap();
    assert!(popup.contains("theme.set_popup_bg(palette.bg);"));
    assert!(popup.contains("theme.set_popup_opacity(palette.opacity);"));
    assert!(main.contains("theme::watch(ThemeRole::Application,"), "Settings keeps its existing window theme role");
    let ui_rs = include_str!("keypad/ui.rs");
    assert!(ui_rs.contains(r#""cheatsheet.opacity" => "Cheatsheet theme intensity".into(),"#));
    assert!(ui_rs.contains(r#"out.push(("cheatsheet.opacity".to_string(), config::number(after.opacity)));"#));
    assert!(include_str!("keypad/config.rs").contains("theme intensity uses a stored value from 0.05 to 1.0 (0.35 is the frosted baseline)"));
}

#[test]
fn keypad_theme_intensity_contract_covers_quantization_and_theme_relative_alpha() {
    use smpl_common::theme::{parse_theme, ThemeRole};

    // Guard the actual Slint expressions before checking their numeric contract.
    assert!(UI.contains("let scaled = saved * 20;"));
    assert!(UI.contains("let lower = floor(scaled);"));
    assert!(UI.contains("let class = scaled - lower == 0.5 ? round(lower / 2) * 2 : round(scaled);"));
    assert!(UI.contains("return class / 7;"));
    assert!(UI.contains("min(1, min(Theme.popup-opacity, 0.35) * root.intensity)"));
    assert!(UI.contains("round(KeypadSheetStyle.intensity(root.kp-sheet-opacity) * 100)"));
    let core = "$theme-bg: #1e1e2e;\n$theme-fg: #cdd6f4;\n$theme-accent: #89b4fa;\n";
    for (popup, application) in [(1.0_f32, 0.4_f32), (0.35, 1.0), (0.20, 1.0), (0.0, 1.0)] {
        let palette = parse_theme(
            &format!("{core}$theme-popup-opacity: {popup};\n$theme-app-background-opacity: {application};"),
            ThemeRole::Popup,
        ).unwrap();
        for (saved, class, percent, frosted) in [
            (0.05_f32, 1.0_f32, 14.0_f32, 0.05_f32),
            (0.10, 2.0, 29.0, 0.10),
            (0.12499, 2.0, 29.0, 0.10),
            (0.125, 2.0, 29.0, 0.10),
            (0.12501, 3.0, 43.0, 0.15),
            (0.35, 7.0, 100.0, 0.35),
            (0.375, 8.0, 114.0, 0.40),
            (0.70, 14.0, 200.0, 0.70),
            (1.0, 20.0, 286.0, 1.0),
        ] {
            let scaled = saved * 20.0;
            let lower = scaled.floor();
            let actual_class = if scaled - lower == 0.5 {
                (lower / 2.0).round() * 2.0
            } else {
                scaled.round()
            };
            let intensity = actual_class / 7.0;
            let alpha = (palette.opacity.min(0.35) * intensity).min(1.0);
            assert_eq!(actual_class, scaled.round_ties_even(), "EWW rounding: saved={saved}");
            assert_eq!(actual_class, class, "saved={saved}");
            assert_eq!((intensity * 100.0).round(), percent, "saved={saved}");
            let expected = if popup >= 0.35 { frosted } else { popup * class / 7.0 };
            assert!((alpha - expected).abs() < 0.000001, "popup={popup}, saved={saved}");
            if saved == 0.35 {
                assert_eq!(alpha, popup.min(0.35), "neutral is frosted even for an opaque theme");
            }
        }
    }
}

// ── Mouse tab ───────────────────────────────────────────────────────────────

/// `source` with comments and the insides of string literals blanked out. It
/// is as long as `source`, so offsets into one index the other.
fn masked(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out[i] = b' ';
                    i += 1;
                }
                if bytes[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            i += 1;
        } else if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                if bytes[i] != b'\n' {
                    out[i] = b' ';
                }
                i += 1;
            }
            for blank in out.iter_mut().skip(i).take(2) {
                *blank = b' ';
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).expect("blanking whole literals and comments keeps the text valid")
}

/// From the `{` that ends `opener` to its matching `}`.
fn braced_block(source: &'static str, opener: &str) -> &'static str {
    let start = source.find(opener).unwrap_or_else(|| panic!("no `{opener}`"));
    let open = start + opener.rfind('{').expect("the opener ends at its brace");
    let code = masked(source);
    let mut depth = 0usize;
    for (offset, byte) in code.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open..=open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("`{opener}` is never closed")
}

fn mouse_page() -> &'static str {
    braced_block(UI, "if root.active-tab == 12: Rectangle {")
}

/// The `names` elements in `source` that do not set `property` themselves, as
/// short snippets for the failure message. A property of a child element, of
/// another name (`border-color` is not `color`), or inside a comment or string
/// does not count.
fn elements_without(source: &str, names: &[&str], property: &str) -> Vec<String> {
    let code = masked(source);
    let bytes = code.as_bytes();
    let in_name = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-';
    let mut found = Vec::new();
    for name in names {
        let mut from = 0;
        while let Some(at) = code[from..].find(name) {
            let start = from + at;
            from = start + name.len();
            if start > 0 && in_name(bytes[start - 1]) {
                continue;
            }
            let Some(open) = code[from..].find(|c: char| !c.is_whitespace()).map(|n| from + n) else {
                continue;
            };
            if bytes[open] != b'{' {
                continue;
            }
            let mut depth = 0usize;
            let mut set = false;
            for i in open..bytes.len() {
                match bytes[i] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ if depth == 1
                        && bytes[i..].starts_with(property.as_bytes())
                        && !in_name(bytes[i - 1]) =>
                    {
                        let after = code[i + property.len()..].trim_start();
                        set |= after.starts_with(':');
                    }
                    _ => {}
                }
            }
            if !set {
                let snippet: String = source[start..].chars().take(70).collect();
                found.push(snippet.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    found
}

/// The `Text` and `TextInput` elements in `source` that set no `color` of
/// their own. Without one Slint reads the desktop colour scheme over D-Bus;
/// that watcher panicked on the UI thread, which has no async runtime, and
/// Settings exited.
fn uncolored_texts(source: &str) -> Vec<String> {
    elements_without(source, &["Text", "TextInput"], "color")
}

#[test]
fn the_text_color_check_sees_what_it_should() {
    assert!(uncolored_texts(r#"Text { text: "a"; color: Theme.fg; }"#).is_empty());
    assert!(uncolored_texts("TextInput { text <=> x; color : Theme.fg; }").is_empty());
    assert!(uncolored_texts("if x: Text\n{ color: red; }").is_empty());
    assert_eq!(uncolored_texts("if x: Text { font-size: 12px; }").len(), 1);
    assert_eq!(uncolored_texts("TextInput { text <=> x; }").len(), 1);
    // A colour of a child, of another property, in a comment or in a string is not its own.
    assert_eq!(uncolored_texts("Text { Rectangle { color: red; } }").len(), 1);
    assert_eq!(uncolored_texts("Text { border-color: red; }").len(), 1);
    assert_eq!(uncolored_texts("Text { // color: red;\n }").len(), 1);
    assert_eq!(uncolored_texts("Text { /* color: red; */ }").len(), 1);
    assert_eq!(uncolored_texts("Text { text: \"color: red;\"; }").len(), 1);
    // Other names, and Text in a string or comment, are not elements.
    assert!(uncolored_texts("KeypadText { }  MyText { }  Texture { }  Text").is_empty());
    assert!(uncolored_texts("// Text { }\nlet s: \"Text { }\";").is_empty());
}

#[test]
fn the_property_check_wants_the_whole_property_on_the_element_itself() {
    let selection = "selection-foreground-color";
    let check = |source| elements_without(source, &["TextInput"], selection).len();
    assert_eq!(check("TextInput { selection-foreground-color: red; }"), 0);
    assert_eq!(check("TextInput { selection-foreground-color : red; }"), 0);
    assert_eq!(check("TextInput { }"), 1);
    // Another property, a child's property, and a longer name are not it.
    assert_eq!(check("TextInput { selection-background-color: red; }"), 1);
    assert_eq!(check("TextInput { Rectangle { selection-foreground-color: red; } }"), 1);
    assert_eq!(check("TextInput { my-selection-foreground-color: red; }"), 1);
    // `color` is not satisfied by the selection colours that end in it.
    let colors = "TextInput { selection-background-color: red; selection-foreground-color: red; }";
    assert_eq!(elements_without(colors, &["TextInput"], "color").len(), 1);
}

#[test]
fn every_text_and_text_input_in_the_ui_sets_a_color() {
    let missing = uncolored_texts(UI);
    assert!(
        missing.is_empty(),
        "Text without a color makes Slint read the desktop colour scheme: {missing:#?}"
    );
}

#[test]
fn every_text_input_sets_its_selection_colors() {
    // Selecting text is what first drew the style's default selection colours,
    // which come from the desktop colour scheme.
    for property in ["selection-background-color", "selection-foreground-color"] {
        let missing = elements_without(UI, &["TextInput"], property);
        assert!(missing.is_empty(), "TextInput without {property}: {missing:#?}");
    }
}

#[test]
fn text_fields_are_built_from_the_themed_text_input() {
    assert_eq!(
        UI.matches("inherits TextInput").count(),
        1,
        "build text fields from ThemedTextInput, not from TextInput"
    );
    let themed = component_source("ThemedTextInput");
    for setting in [
        "color: Theme.fg;",
        "selection-background-color: Theme.accent;",
        "selection-foreground-color: Theme.bg;",
    ] {
        assert!(themed.contains(setting), "ThemedTextInput lacks `{setting}`");
    }
    assert!(UI.contains("component SearchInput inherits ThemedTextInput {"));
}

#[test]
fn texts_in_the_mouse_tab_and_the_shared_assignment_components_set_a_color() {
    for (name, source) in [
        ("the Mouse page", mouse_page()),
        ("AssignProfileEditor", component_source("AssignProfileEditor")),
        ("AssignActionEditor", component_source("AssignActionEditor")),
        ("AssignBindingList", component_source("AssignBindingList")),
        ("DeviceCanvas", component_source("DeviceCanvas")),
    ] {
        assert!(source.contains("Text {"), "{name} has no Text to check");
        let missing = uncolored_texts(source);
        assert!(missing.is_empty(), "{name}: Text without a color: {missing:#?}");
    }
}

#[test]
fn mouse_tab_is_wired_into_the_sidebar_command_line_and_search() {
    let main = include_str!("main.rs");
    assert!(main.contains("\"mouse\" => 12,"));
    assert!(main.contains("\"mouse\" => \"Mouse\","));
    assert!(main.contains("(\"Mouse\", \"mouse\", 12)"));
    assert!(main.contains("mouse::ui::install(&ui)"));
    assert!(include_str!("mouse/ui.rs").contains("const MOUSE_TAB: i32 = 12;"));
    let button: String = UI.split("label: \"Mouse\";").nth(1).unwrap().chars().take(300).collect();
    assert!(button.contains("active: root.active-tab == 12;"), "{button}");
    assert!(button.contains("root.active-tab = 12;"), "{button}");
    assert!(UI.contains("if root.active-tab == 12 { root.ms-entered(); }"));
}

// The launcher and the search box send these as `--highlight`; each must
// light up a card, and each card label must be searchable.
const MOUSE_HIGHLIGHTS: [&str; 12] = [
    "Pointer Speed",
    "Cursor Speed",
    "Mouse Sensitivity",
    "Mouse Acceleration",
    "Scroll Direction",
    "Natural Scrolling",
    "Scroll Speed",
    "Mouse Buttons",
    "Mouse Button Mapping",
    "MMO Mouse",
    "Razer Naga",
    "Redragon Mouse",
];

#[test]
fn mouse_search_labels_and_highlights_agree() {
    use std::collections::BTreeSet;
    let expected: BTreeSet<&str> = MOUSE_HIGHLIGHTS.into_iter().collect();
    let searchable: BTreeSet<&str> = include_str!("main.rs")
        .lines()
        .filter_map(|line| {
            let (label, _) = line.trim().strip_prefix("(\"")?.split_once("\", \"mouse\", 12)")?;
            Some(label).filter(|label| *label != "Mouse")
        })
        .collect();
    assert_eq!(searchable, expected, "search entries for the tab");
    let highlighted: BTreeSet<&str> = mouse_page()
        .split("_hl == \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect();
    assert_eq!(highlighted, expected, "labels the page highlights");
}

// A cold start lays the page out before the window has its final size, so a
// highlighted card has to be scrolled to again until its position stops moving.
#[test]
fn mouse_highlight_cards_stay_in_view_while_a_cold_start_settles() {
    let page = mouse_page();
    let scroll = "tab12-flick.viewport-y = -max(0px, self.y - 20px);";
    for hook in [
        format!("init => {{ if highlighted {{ settling = true; {scroll} }} }}"),
        format!("changed _hl => {{ if highlighted {{ {scroll} }} }}"),
        format!("changed y => {{ if settling && highlighted {{ {scroll} }} }}"),
    ] {
        assert_eq!(page.matches(hook.as_str()).count(), 2, "one per highlight card: {hook}");
    }
    for card in ["ms-pointer-card", "ms-buttons-card"] {
        for needle in [
            format!("{card} := Rectangle {{"),
            format!("running: {card}.settling;"),
            format!("triggered => {{ {card}.settling = false; }}"),
        ] {
            assert!(page.contains(&needle), "missing: {needle}");
        }
    }
}

#[test]
fn mouse_page_shows_the_model_and_only_reports_changes() {
    let page = mouse_page();
    for needle in [
        "value: root.ms-sensitivity;",
        "changed(v) => { root.ms-set-sensitivity(v); }",
        "value: root.ms-scroll-factor;",
        "changed(v) => { root.ms-set-scroll-factor(v); }",
        "current-index: root.ms-accel-index;",
        "selected(i) => { root.ms-set-accel(i); }",
        "current-index: root.ms-device-index;",
        "selected(i) => { root.ms-select-device(i); }",
        "root.ms-set-natural-scroll(idx == 1);",
        "enabled: !root.ms-pointer-at-defaults;",
        "clicked => { root.ms-refresh(); }",
        "clicked(id) => { root.ms-select-button(id); }",
        "selected(slot) => { root.ms-select-button(slot); }",
        "locked: !root.ms-editing;",
        "apply => { root.ms-apply-binding(); }",
    ] {
        assert!(page.contains(needle), "missing: {needle}");
    }
    // Controlled inputs: nothing here stores a value the model owns. (Typed
    // text is bound two ways on purpose, with `<=>`.)
    let code = masked(page);
    let mut from = 0;
    while let Some(at) = code[from..].find("root.ms-") {
        let start = from + at + "root.".len();
        let end = code[start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .map_or(code.len(), |n| start + n);
        let after = code[end..].trim_start();
        assert!(!after.starts_with('=') || after.starts_with("=="), "the page assigns {}", &code[start..end]);
        from = end;
    }
    assert!(page.contains("root.ms-identifying ? \"Stop identifying\" : \"Identify buttons\""));
    assert!(page.contains("enabled: root.ms-backend-ready;"));
}
