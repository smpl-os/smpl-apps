const UI: &str = include_str!("../ui/main.slint");
const MAIN: &str = include_str!("../src/main.rs");

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
    assert!(UI.contains("no-frame: true;"));
}

#[test]
fn about_hides_list_without_an_opaque_sheet() {
    assert!(UI.contains("visible: !root.show-about;"));
    let about = UI.split("// ABOUT OVERLAY").nth(1).unwrap();
    let background = about.split("background:").nth(1).unwrap();
    assert!(background.trim_start().starts_with("transparent;"));
    assert!(!UI.contains("if !root.show-about:"));
}

#[test]
fn shared_application_theme_is_kept_alive() {
    assert!(MAIN.contains("smpl_common::init(\"webapp-center\", 440.0, 520.0)"));
    assert!(MAIN.contains("let _theme_timer = smpl_common::theme::watch("));
    assert!(MAIN.contains("smpl_common::theme::ThemeRole::Application"));
    assert!(MAIN.contains("palette: &smpl_common::theme::ThemePalette"));
    assert!(!MAIN.contains("load_theme_from_eww_scss"));
    assert!(!MAIN.contains("mod theme;"));
}
