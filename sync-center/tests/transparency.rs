const UI: &str = include_str!("../ui/main.slint");
const MAIN: &str = include_str!("../src/bin/gui.rs");

#[test]
fn opacity_is_background_only() {
    assert!(UI.contains("property <float> opacity: 1.0;"));
    assert!(!UI.lines().any(|line| line.trim().starts_with("opacity:")));
    assert!(!UI.lines().any(|line| {
        line.trim().starts_with("color:") && line.contains("transparentize")
    }));
    assert_eq!(UI.matches("Theme.opacity").count(), 1);
    assert!(UI.contains("background: Theme.bg.transparentize(1.0 - Theme.opacity);"));
    assert!(UI.contains("surface: bg_light.transparentize(0.90)"));
    assert!(UI.contains("background: touch.has-hover ? Theme.surface-hover : Theme.surface;"));
    assert!(UI.contains("no-frame: true;"));
}

#[test]
fn shared_application_theme_is_kept_alive() {
    assert!(MAIN.contains("smpl_common::init(\"sync-center\", 800.0, 600.0)"));
    assert!(MAIN.contains("let _theme_timer = smpl_common::theme::watch(ThemeRole::Application"));
    for field in [
        "bg", "fg", "fg_dim", "accent", "bg_light", "bg_lighter", "danger",
        "success", "warning", "info", "opacity",
    ] {
        assert!(MAIN.contains(&format!("theme.set_{field}(palette.{field});")));
    }
}
