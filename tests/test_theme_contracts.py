"""Source guardrails; native pixel checks are a separate renderer fixture."""

import pathlib
import re
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
GUI_SOURCES = {
    "start-menu/src/main.rs": ("start-menu", "Popup"),
    "notif-center/src/main.rs": ("notif-center", "Popup"),
    "settings/src/main.rs": ("settings", "Application"),
    "app-center/src/main.rs": ("app-center", "Application"),
    "webapp-center/src/main.rs": ("webapp-center", "Application"),
    "sync-center/src/bin/gui.rs": ("sync-center", "Application"),
    "calendar/src/main.rs": ("smpl-calendar", "Popup"),
    "hints/src/overlay.rs": ("hints-overlay", "Popup"),
}


class ThemeContractTests(unittest.TestCase):
    def test_all_gui_entrypoints_use_shared_theme_roles_and_existing_ids(self):
        for path, (app_id, role) in GUI_SOURCES.items():
            with self.subTest(path=path):
                source = (ROOT / path).read_text()
                self.assertIn(f'"{app_id}"', source)
                self.assertIn("smpl_common::init(", source)
                self.assertIn(f"ThemeRole::{role}", source)
                self.assertNotIn("load_theme_from_eww_scss", source)
        self.assertIn(
            '"smpl-calendar-details"',
            (ROOT / "calendar/src/main.rs").read_text(),
        )

    def test_backend_is_centralized_and_preserves_alpha_requirements(self):
        backend = (ROOT / "smpl-common/src/lib.rs").read_text()
        for requirement in [
            '.with_renderer_name("femtovg")',
            ".with_decorations(false)",
            ".with_name(app_id, app_id)",
        ]:
            self.assertIn(requirement, backend)
        for source in ROOT.glob("*/src/**/*.rs"):
            if source != ROOT / "smpl-common/src/lib.rs":
                self.assertNotIn(
                    "Backend::builder()", source.read_text(), str(source)
                )
        manifest = (ROOT / "Cargo.toml").read_text()
        self.assertNotIn("renderer-software", manifest)
        self.assertNotIn("renderer-skia", manifest)

    def test_ui_does_not_fade_foreground_or_ancestors(self):
        for source in ROOT.glob("*/ui/*.slint"):
            with self.subTest(path=source):
                text = source.read_text()
                self.assertIsNone(
                    re.search(r"^\s*opacity\s*:", text, re.MULTILINE),
                    "Use background alpha or opaque semantic foreground colors",
                )
                self.assertIsNone(
                    re.search(
                        r"^\s*(?:color|colorize)\s*:[^;]*transparentize",
                        text,
                        re.MULTILINE,
                    ),
                    "Text/icon interiors must not inherit an alpha fade",
                )

    def test_palette_parser_exists_only_in_shared_module(self):
        for crate in {path.split("/")[0] for path in GUI_SOURCES}:
            for source in (ROOT / crate / "src").rglob("*.rs"):
                self.assertNotIn("fn parse_hex_color(", source.read_text())
                self.assertNotIn("fn load_theme_from_eww_scss(", source.read_text())


if __name__ == "__main__":
    unittest.main()
