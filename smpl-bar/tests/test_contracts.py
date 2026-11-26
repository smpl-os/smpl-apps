"""Headless source and release contracts for the native smplOS bar."""

import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[2]
BAR = ROOT / "smpl-bar"


class SmplBarContracts(unittest.TestCase):
    def test_native_shell_uses_shared_alpha_backend_and_native_panel_primitives(self):
        source = (BAR / "src/main.rs").read_text()
        panel = (BAR / "src/panel.rs").read_text()
        self.assertIn("smpl_common::init(app_id", source)
        self.assertIn("ThemeRole::Application", source)
        self.assertIn("ThemeRole::Popup", source)
        self.assertIn("slint::set_xdg_app_id(app_id)", source)
        self.assertIn("LayerShell::bind", panel)
        self.assertIn("set_exclusive_zone", panel)
        self.assertIn("_NET_WM_STRUT_PARTIAL", panel)
        self.assertNotIn("std::process::Command::new", source)

    def test_ui_covers_side_by_side_bar_tray_and_native_popups(self):
        ui = (BAR / "ui/main.slint").read_text()
        for component in (
            "BarWindow",
            "QuickSettingsWindow",
            "UsbWindow",
            "WorkspaceOverviewWindow",
            "KeypadCheatsheetWindow",
            "IdentifyWindow",
            "TrayMenuWindow",
        ):
            self.assertIn(f"component {component}", ui)
        self.assertIn("workspace-overview", ui)
        self.assertIn("click-through", ui)
        self.assertIn("sni-items", ui)
        self.assertIn("overlay-image", ui)

    def test_telemetry_is_opt_in_and_measurement_is_private(self):
        telemetry = (ROOT / "smpl-shell/src/telemetry.rs").read_text()
        benchmark = (BAR / "scripts/benchmark.py").read_text()
        self.assertIn("SMPL_BAR_TELEMETRY", telemetry)
        self.assertIn("return Self { inner: None }", telemetry)
        self.assertIn("--backend=headless-backend.so", benchmark)
        self.assertIn("--renderer=pixman", benchmark)
        self.assertNotIn("/tmp", benchmark)
        self.assertNotIn("xvfb-run", benchmark)

    def test_sni_host_has_watcher_icons_menu_activation_and_owner_recovery(self):
        config = (ROOT / "smpl-shell/src/config.rs").read_text()
        sni = (BAR / "src/sni.rs").read_text()
        self.assertNotIn("generic StatusNotifierItem support is not implemented", config)
        self.assertIn("org.kde.StatusNotifierWatcher", sni)
        self.assertIn("StatusNotifierItemRegistered", sni)
        self.assertIn("IconPixmap", sni)
        self.assertIn("AttentionIconPixmap", sni)
        self.assertIn("com.canonical.dbusmenu", sni)
        self.assertIn("NameOwnerChanged", sni)
        self.assertIn('"Activate"', sni)

    def test_24px_layout_keyboard_title_and_tray_theme_contracts(self):
        ui = (BAR / "ui/main.slint").read_text()
        source = (BAR / "src/main.rs").read_text()
        compositor = (ROOT / "smpl-shell/src/compositor.rs").read_text()
        self.assertIn("height: 20px;", ui)
        self.assertIn("padding-top: 2px;", ui)
        self.assertIn("padding-bottom: 2px;", ui)
        self.assertIn('tip: "Workspace overview on " + root.monitor-label', ui)
        self.assertNotIn("icon: root.monitor-label", ui)
        self.assertIn("if item.has-icon && item.themed-icon: Image", ui)
        self.assertIn("if item.has-icon && !item.themed-icon: Image", ui)
        self.assertIn("colorize: Theme.fg", ui)
        apply_system = source[source.index("fn apply_system"):source.index("fn apply_provider_state")]
        self.assertNotIn("set_keyboard_visible", apply_system)
        self.assertIn("state.keyboard.visible", source)
        self.assertIn("is_shell_surface", compositor)
        self.assertIn('"GitHub Copilot"', source)
        self.assertIn('"unrecolored_icons"', source)

    def test_popup_callbacks_route_through_os_mutual_exclusion(self):
        source = (BAR / "src/main.rs").read_text()
        callbacks = source[source.index("fn bind_bar_callbacks"):source.index("fn handle_request")]
        self.assertIn('["popup-toggle", target.argument()]', callbacks)
        for target in (
            "quick-settings",
            "usb-popup",
            "workspace-overview",
            "calendar-popup",
            "notification-hub",
        ):
            self.assertIn(f'"{target}"', callbacks)
        self.assertIn("signal_start_menu(libc::SIGUSR1)", callbacks)
        self.assertNotIn('spawn_after_global_close(["toggle-start-menu"])', callbacks)
        self.assertNotIn('["toggle-calendar"]', callbacks)
        self.assertNotIn('["toggle-notif-center"]', callbacks)

    def test_preview_has_distinct_identity_and_panel_config_contracts(self):
        source = (BAR / "src/main.rs").read_text()
        config = (ROOT / "smpl-shell/src/config.rs").read_text()
        compositor = (ROOT / "smpl-shell/src/compositor.rs").read_text()
        self.assertIn('PREVIEW_APP_ID: &str = "smpl-bar-preview"', source)
        self.assertIn("preview requires Hyprland", source)
        self.assertIn("preview_offset_y", source)
        self.assertIn("PanelEdge", config)
        self.assertIn("PanelMode", config)
        self.assertIn("bar_edge", config)
        self.assertIn("EventStream", compositor)
        self.assertNotIn('["msg", "event-stream"]', compositor)
        self.assertIn("PlacePopup", compositor)

    def test_click_through_is_real_on_x11_and_rejected_on_wayland(self):
        source = (BAR / "src/main.rs").read_text()
        panel = (BAR / "src/panel.rs").read_text()
        self.assertIn("keypad click-through rejected", source)
        self.assertIn("set_input_passthrough", panel)
        self.assertIn("shape_rectangles", panel)
        self.assertIn('ProviderAction::ToggleCheatsheet', source)
        self.assertIn("CheatsheetVisibilityChanged", (BAR / "src/providers.rs").read_text())


if __name__ == "__main__":
    unittest.main()
