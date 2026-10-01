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
    for tab in 0..=10 {
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
