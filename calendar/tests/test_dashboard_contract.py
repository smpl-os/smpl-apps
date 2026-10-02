"""Headless structure checks complement Rust tests and the isolated native fixture."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


class DashboardContractTests(unittest.TestCase):
    def test_font_is_embedded_and_license_is_shipped(self):
        ui = (ROOT / "ui/main.slint").read_text()
        self.assertIn('import "assets/DSEG7Modern-Regular.ttf";', ui)
        self.assertGreater((ROOT / "ui/assets/DSEG7Modern-Regular.ttf").stat().st_size, 20000)
        self.assertIn("SIL OPEN FONT LICENSE", (ROOT / "ui/assets/DSEG-LICENSE.txt").read_text())
        self.assertIn("--font-license", (ROOT / "src/main.rs").read_text())

    def test_forecast_precedes_clocks_and_calendar(self):
        ui = (ROOT / "ui/main.slint").read_text().split("// COMPACT VIEW", 1)[1]
        self.assertLess(ui.index("for day[i] in root.forecast"), ui.index("for clock[i] in root.clocks"))
        self.assertLess(ui.index("for clock[i] in root.clocks"), ui.index("MiniMonthGrid"))

    def test_accessible_details_action_and_animation_gate(self):
        ui = (ROOT / "ui/main.slint").read_text()
        self.assertIn('icon: @image-url("assets/expand.svg"); label: "Open detailed calendar";', ui)
        self.assertIn("clicked => { root.open-details(); }", ui)
        self.assertIn("accessible-action-default => { root.clicked(); }", ui)
        self.assertIn("root.animate-weather && !root.is-details", ui)
        self.assertIn("root.count == 1 ? halo : halo / 3", ui)
        self.assertIn("cw: root.width / 7", ui)

    def test_city_preferences_are_actionable(self):
        ui = (ROOT / "ui/main.slint").read_text()
        for callback in ("search-city", "add-city", "remove-city", "select-primary", "set-fahrenheit"):
            self.assertIn(f"callback {callback}(", ui)
        self.assertIn('accessible-label: "City name"', ui)
        self.assertIn("No automatic location lookup", ui)

    def test_back_is_icon_only_with_accessible_keyboard_action(self):
        ui = (ROOT / "ui/main.slint").read_text()
        self.assertIn('icon: @image-url("assets/back.svg");', ui)
        self.assertIn('label: "Back to calendar";', ui)
        self.assertIn("clicked => { root.close-details(); }", ui)
        button = ui.split("component OutlineButton", 1)[1].split("\ncomponent ", 1)[0]
        self.assertIn("width: 30px; height: 30px;", button)
        self.assertNotIn("show-label", button)
        self.assertIn("colorize: Theme.fg", button)
        self.assertIn("accessible-label: root.label", button)
        self.assertIn('event.text == Key.Return || event.text == " "', button)

    def test_event_dates_have_foreground_rings_in_both_grids(self):
        ui = (ROOT / "ui/main.slint").read_text()
        grids = ui.split("component MiniMonthGrid", 1)[1].split("// ── EventCard", 1)[0]
        self.assertEqual(grids.count("if cell.has-events: Rectangle"), 2)
        self.assertEqual(grids.count("width: 26px; height: 26px; border-radius: 13px;"), 2)
        self.assertEqual(grids.count("border-color: cell.is-other-month ? Theme.fg-dim : Theme.fg;"), 2)
        self.assertNotIn("width: 4px; height: 4px;", grids)
        self.assertIn('cell.event-count + " events"', grids)


if __name__ == "__main__":
    unittest.main()
