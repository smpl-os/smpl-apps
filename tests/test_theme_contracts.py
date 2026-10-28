"""Source guardrails; native pixel checks are a separate renderer fixture."""

import pathlib
import re
import tomllib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
PRIVATE_MOUSE_PROBE = ROOT / "settings/src/mouse/native_tests.rs"
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
            '.with_renderer_name("skia-software")',
            ".with_decorations(false)",
            ".with_name(app_id, app_id)",
        ]:
            self.assertIn(requirement, backend)
        for source in ROOT.glob("*/src/**/*.rs"):
            if source not in {ROOT / "smpl-common/src/lib.rs", PRIVATE_MOUSE_PROBE}:
                self.assertNotIn(
                    "Backend::builder()", source.read_text(), str(source)
                )
        manifest = (ROOT / "Cargo.toml").read_text()
        self.assertIn('"renderer-skia"', manifest)
        self.assertNotIn('"renderer-femtovg"', manifest)
        self.assertNotIn('"renderer-software"', manifest)
        self.assertIn('softbuffer = { path = "vendor/softbuffer" }', manifest)
        self.assertIn('i-slint-renderer-skia = { path = "vendor/i-slint-renderer-skia" }', manifest)

    def test_slint_versions_match_the_patched_renderer_exactly(self):
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
        renderer = tomllib.loads(
            (ROOT / "vendor/i-slint-renderer-skia/Cargo.toml").read_text()
        )
        for crate in ("slint", "slint-build", "i-slint-backend-winit"):
            dependency = manifest["workspace"]["dependencies"][crate]
            version = dependency["version"] if isinstance(dependency, dict) else dependency
            self.assertEqual(version, "=" + renderer["package"]["version"], crate)

    def test_cpu_presentation_preserves_premultiplied_alpha(self):
        skia = (ROOT / "vendor/i-slint-renderer-skia/software_surface.rs").read_text()
        self.assertIn("skia_safe::AlphaType::Premul", skia)
        self.assertNotIn("skia_safe::AlphaType::Opaque", skia)
        manifest = (ROOT / "vendor/i-slint-renderer-skia/Cargo.toml").read_text()
        self.assertIn('features = ["wayland-alpha"]', manifest)
        wayland = (ROOT / "vendor/softbuffer/src/backends/wayland/buffer.rs").read_text()
        self.assertIn('cfg!(feature = "wayland-alpha")', wayland)
        self.assertIn("wl_shm::Format::Argb8888", wayland)
        self.assertEqual(len(re.findall(
            r"\.create_buffer\(\s*0,\s*width,\s*height,\s*width\s*\*\s*4,\s*pixel_format\(\)",
            wayland,
        )), 2)

    def test_mouse_probe_backend_is_test_only_and_private_cpu(self):
        module = (ROOT / "settings/src/mouse/mod.rs").read_text()
        self.assertRegex(module, r"#\[cfg\(test\)\]\s+mod native_tests;")
        source = PRIVATE_MOUSE_PROBE.read_text()
        self.assertIn('.with_renderer_name("skia-software")', source)
        self.assertIn('std::env::var("SMPL_MOUSE_PROBE_PRIVATE_DISPLAY").as_deref(), Ok("1")', source)
        self.assertIn('std::env::var("DISPLAY").unwrap().starts_with("127.0.0.1:")', source)

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
