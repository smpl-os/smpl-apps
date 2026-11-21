"""Headless source and release contracts for the native smplOS bar."""

import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
BAR = ROOT / "smpl-bar"


class SmplBarContracts(unittest.TestCase):
    def test_native_shell_uses_shared_backend_and_keeps_sni_gated(self):
        source = (BAR / "src/main.rs").read_text()
        self.assertIn("smpl_common::init(app_id", source)
        self.assertIn("ThemeRole::Application", source)
        self.assertIn("ThemeRole::Popup", source)
        self.assertIn("slint::set_xdg_app_id(app_id)", source)
        self.assertIn('"generic_sni_gate": "not-implemented"', source)
        self.assertNotIn("std::process::Command::new", source)

    def test_ui_covers_side_by_side_bar_and_native_popups(self):
        ui = (BAR / "ui/main.slint").read_text()
        for component in (
            "BarWindow",
            "QuickSettingsWindow",
            "UsbWindow",
            "WorkspaceOverviewWindow",
            "KeypadCheatsheetWindow",
            "IdentifyWindow",
        ):
            self.assertIn(f"component {component}", ui)
        self.assertIn("workspace-overview", ui)
        self.assertIn("click-through", ui)

    def test_telemetry_is_opt_in_and_measurement_is_private(self):
        telemetry = (ROOT / "smpl-shell/src/telemetry.rs").read_text()
        benchmark = (BAR / "scripts/benchmark.py").read_text()
        self.assertIn("SMPL_BAR_TELEMETRY", telemetry)
        self.assertIn("return Self { inner: None }", telemetry)
        self.assertIn("--backend=headless-backend.so", benchmark)
        self.assertIn("--renderer=pixman", benchmark)
        self.assertNotIn("/tmp", benchmark)
        self.assertNotIn("xvfb-run", benchmark)

    def test_sni_is_a_config_validation_gate_not_a_claimed_module(self):
        config = (ROOT / "smpl-shell/src/config.rs").read_text()
        self.assertIn("acceptance-gated: generic StatusNotifierItem", config)
        self.assertIn("Module::Sni", config)

    def test_popup_callbacks_route_through_os_mutual_exclusion(self):
        source = (BAR / "src/main.rs").read_text()
        callbacks = source[source.index("fn bind_bar_callbacks"):source.index("fn handle_request")]
        self.assertIn('["popup-toggle", target.argument()]', callbacks)
        for target in (
            "start-menu",
            "quick-settings",
            "usb-popup",
            "workspace-overview",
            "calendar-popup",
            "notification-hub",
        ):
            self.assertIn(f'"{target}"', callbacks)
        self.assertIn('spawn_after_global_close(["toggle-start-menu"])', callbacks)
        self.assertNotIn('["toggle-calendar"]', callbacks)
        self.assertNotIn('["toggle-notif-center"]', callbacks)

    def test_preview_has_distinct_identity_and_rejects_non_hyprland(self):
        source = (BAR / "src/main.rs").read_text()
        self.assertIn('PREVIEW_APP_ID: &str = "smpl-bar-preview"', source)
        self.assertIn(
            "preview requires Hyprland", source
        )
        self.assertIn("preview_offset_y", source)


if __name__ == "__main__":
    unittest.main()
