const UI: &str = include_str!("../ui/main.slint");
const COMPONENTS: &str = include_str!("../ui/components.slint");
const MAIN: &str = include_str!("../src/main.rs");

#[test]
fn opacity_is_background_only() {
    for source in [UI, COMPONENTS] {
        assert!(source.contains("property <float> opacity: 1.0;"));
        assert!(!source.lines().any(|line| line.trim().starts_with("opacity:")));
        assert!(!source.lines().any(|line| {
            line.trim().starts_with("color:") && line.contains("transparentize")
        }));
        assert!(source.contains("surface: bg_light.transparentize(0.90)"));
    }
    assert_eq!(UI.matches("Theme.opacity").count(), 1);
    assert!(UI.contains("background: Theme.bg.transparentize(1.0 - Theme.opacity);"));
    assert!(UI.contains("no-frame: true;"));
}

#[test]
fn help_replaces_without_destroying_main_content() {
    assert!(UI.contains("visible: !root.show-help;"));
    assert!(UI.contains("if root.show-help: Rectangle {\n        background: transparent;"));
    assert!(!UI.contains("if !root.show-help:"));
}

#[test]
fn shared_application_theme_is_kept_alive() {
    assert!(MAIN.contains("smpl_common::init(\"app-center\", 560.0, 620.0)"));
    assert!(MAIN.contains("let _theme_timer = smpl_common::theme::watch(ThemeRole::Application"));
    assert!(MAIN.contains("fn apply_theme(ui: &MainWindow, palette: &ThemePalette)"));
    assert!(!MAIN.contains("load_theme_from_eww_scss"));
    assert!(!MAIN.contains("mod theme;"));
    assert!(MAIN.contains("let poll_timer = slint::Timer::default()"));
}
