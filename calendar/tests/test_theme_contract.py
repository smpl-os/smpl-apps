"""Headless source contracts; run with python -m unittest discover -s calendar/tests."""

from pathlib import Path
import re
import unittest


ROOT = Path(__file__).resolve().parents[1]


class ThemeContractTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rust = (ROOT / "src/main.rs").read_text()
        cls.ui = (ROOT / "ui/main.slint").read_text()

    def test_shared_popup_watcher_is_retained_for_both_modes(self):
        self.assertIn(
            "let _theme_timer = theme::watch(ThemeRole::Popup, move |palette|",
            self.rust,
        )
        self.assertIn("let ui_weak = ui.as_weak();", self.rust)
        self.assertIn("apply_theme(&ui, palette);", self.rust)
        self.assertNotIn("load_theme_from_eww_scss", self.rust)
        self.assertNotIn("mod theme;", self.rust)
        self.assertFalse((ROOT / "src/theme.rs").exists())
        self.assertIn(
            'if start_details { "smpl-calendar-details" } else { "smpl-calendar" }',
            self.rust,
        )
        self.assertIn("smpl_common::init(app_id, init_w, init_h)?;", self.rust)

    def test_background_alpha_does_not_dim_content(self):
        self.assertIn("no-frame: true;", self.ui)
        self.assertEqual(
            self.ui.count("Theme.bg.transparentize(1.0 - Theme.opacity)"), 1
        )
        self.assertNotRegex(self.ui, r"(?m)^\s*opacity\s*:")
        self.assertNotRegex(
            self.ui, r"(?:Theme\.(?:fg|fg-dim)|color:)[^\n;]*transparentize"
        )
        self.assertIn("surface: bg-light.transparentize(0.90);", self.ui)
        self.assertIn("surface-hover: bg-lighter.transparentize(0.84);", self.ui)
        self.assertEqual(self.ui.count("background: Theme.surface;"), 5)
        self.assertEqual(
            self.ui.count("padding-right: root.show-day-panel ? 310px : 0px;"), 2
        )

    def test_modal_and_picker_readability_and_lifecycle_are_preserved(self):
        form = self.ui.split("component EventForm inherits Rectangle {", 1)[1]
        form = form.split("// ── Main window", 1)[0]
        self.assertIn("background: Theme.bg-light;", form)
        self.assertIn("if root.show-form: Rectangle {", self.ui)
        self.assertIn("background: #000000aa;", self.ui)
        self.assertIn("clicked => { root.cancel-form(); }", self.ui)
        self.assertIn("save()   => { root.save-event(); }", self.ui)
        self.assertIn("cancel() => { root.cancel-form(); }", self.ui)
        self.assertEqual(
            len(re.findall(
                r"if root.open: Rectangle \{.*?background: Theme.bg-lighter;",
                self.ui,
                re.DOTALL,
            )),
            4,
        )


if __name__ == "__main__":
    unittest.main()
