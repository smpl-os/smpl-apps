"""Headless source contracts for the launcher, notification and hints overlays."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[2]
APPS = ("start-menu", "notif-center", "hints")


def source(app, path):
    return (ROOT / app / path).read_text()


class PopupThemeContract(unittest.TestCase):
    def test_shared_popup_watch_is_retained_with_weak_ui(self):
        for app in APPS:
            path = "src/overlay.rs" if app == "hints" else "src/main.rs"
            rust = source(app, path)
            with self.subTest(app=app):
                self.assertIn("ThemePalette, ThemeRole", rust)
                self.assertIn("palette: &ThemePalette", rust)
                self.assertIn("let ui_weak = ui.as_weak();", rust)
                self.assertIn(
                    "let _theme_timer = theme::watch(ThemeRole::Popup", rust
                )
                self.assertIn("if let Some(ui) = ui_weak.upgrade()", rust)
                self.assertIn("apply_theme(&ui, palette);", rust)
                self.assertIn("theme.set_opacity(palette.opacity);", rust)
                self.assertNotIn("load_theme_from_eww_scss", rust)
        hints = source("hints", "src/overlay.rs")
        self.assertIn("_theme_timer: slint::Timer", hints)
        self.assertIn("Ok(Self { ui, model, _theme_timer })", hints)

    def test_palette_application_keeps_all_main_window_colors(self):
        for app in ("start-menu", "notif-center"):
            rust = source(app, "src/main.rs")
            for name in (
                "bg", "fg", "fg_dim", "accent", "bg_light", "bg_lighter",
                "danger", "success", "warning", "info",
            ):
                with self.subTest(app=app, color=name):
                    self.assertRegex(rust, rf"theme\.set_{name}\(palette\.{name}")
            self.assertFalse((ROOT / app / "src/theme.rs").exists())

    def test_foreground_and_items_are_never_faded(self):
        for app in APPS:
            path = "ui/overlay.slint" if app == "hints" else "ui/main.slint"
            ui = source(app, path)
            with self.subTest(app=app):
                self.assertIn("property <float> opacity: 1.0;", ui)
                self.assertNotRegex(ui, r"(?m)^\s*opacity\s*:")
                for foreground in re.findall(
                    r"(?m)^\s*(?:color|colorize)\s*:\s*([^;]+);", ui
                ):
                    self.assertNotIn("transparentize", foreground)
                    self.assertNotIn("opacity", foreground)

    def test_main_windows_only_apply_popup_opacity_to_base_paint(self):
        for app in ("start-menu", "notif-center"):
            ui = source(app, "ui/main.slint")
            with self.subTest(app=app):
                self.assertEqual(ui.count("Theme.opacity"), 1)
                self.assertIn(
                    "background: Theme.bg.transparentize(1.0 - Theme.opacity);",
                    ui,
                )

    def test_launcher_panels_are_tints_but_popup_menus_keep_floor(self):
        ui = source("start-menu", "ui/main.slint")
        self.assertIn("background: Theme.bg_light.transparentize(0.88);", ui)
        self.assertIn("background: Theme.bg_light.transparentize(0.90);", ui)
        self.assertNotIn("background: Theme.bg_light;", ui)
        self.assertEqual(
            ui.count("background: Theme.bg.transparentize(0.05);"), 2
        )

    def test_notification_help_hides_list_without_hiding_navigation(self):
        ui = source("notif-center", "ui/main.slint")
        self.assertRegex(
            ui, r"viewport := Rectangle \{\s*visible: !root\.show-help;"
        )
        self.assertIn("root.show-help = !root.show-help;", ui)
        self.assertIn("root.show-help = false;", ui)
        self.assertNotIn("background: Theme.bg_light;", ui)
        self.assertNotIn("? Theme.bg_lighter\n", ui)
        self.assertEqual(
            ui.count("background: Theme.bg_light.transparentize(0.90);"), 2
        )

    def test_notification_polling_is_not_removed_with_theme_polling(self):
        rust = source("notif-center", "src/main.rs")
        self.assertIn("std::time::Duration::from_secs(2)", rust)
        self.assertIn("ui.invoke_refresh();", rust)
        self.assertNotIn("apply_theme(&ui);", rust)

    def test_hint_root_remains_transparent_and_only_badges_get_opacity(self):
        ui = source("hints", "ui/overlay.slint")
        root = ui.split("export component OverlayWindow inherits Window {", 1)[1]
        self.assertIn('title: "hints-overlay";', root)
        self.assertIn("background: transparent;", root)
        self.assertNotIn("Theme.opacity", root)
        self.assertEqual(ui.count("Theme.opacity"), 1)
        self.assertIn(
            "background: Theme.accent.transparentize(1.0 - Theme.opacity);", ui
        )
        rust = source("hints", "src/overlay.rs")
        self.assertIn('smpl_common::init("hints-overlay", 1920.0, 1080.0)', rust)
        self.assertIn("theme.set_accent(palette.warning);", rust)
        self.assertIn("theme.set_pill_fg(palette.bg);", rust)


if __name__ == "__main__":
    unittest.main()
