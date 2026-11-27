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

    def test_settings_preferences_win_without_replacing_native_layout(self):
        config = (ROOT / "smpl-shell/src/config.rs").read_text()
        settings = (ROOT / "settings/src/taskbar.rs").read_text()
        settings_ui = (ROOT / "settings/ui/main.slint").read_text()
        panel = (BAR / "src/panel.rs").read_text()
        ui = (BAR / "ui/main.slint").read_text()

        self.assertLess(
            config.index("if paths.native.exists()"),
            config.index("if paths.legacy.exists()"),
        )
        self.assertIn('"bar_edge"', settings)
        self.assertIn('"bar_mode"', settings)
        self.assertIn('runtime.run("bar-ctl", &["apply"])?', settings)
        self.assertNotIn('runtime.run(\n            "eww"', settings)
        for edge in ("Bottom", "Top", "Left", "Right"):
            self.assertIn(f'"{edge}"', settings_ui)
        self.assertIn('"Pinned", "Floating"', settings_ui)
        for edge in ("Left", "Right"):
            self.assertIn(f"PanelEdge::{edge}", panel)
        self.assertIn("Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM", panel)
        self.assertIn("Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM", panel)
        self.assertIn("if root.vertical: VerticalLayout", ui)

    def test_click_through_is_real_on_x11_and_rejected_on_wayland(self):
        source = (BAR / "src/main.rs").read_text()
        panel = (BAR / "src/panel.rs").read_text()
        self.assertIn("keypad click-through rejected", source)
        self.assertIn("set_input_passthrough", panel)
        self.assertIn("shape_rectangles", panel)
        self.assertIn('ProviderAction::ToggleCheatsheet', source)
        self.assertIn("CheatsheetVisibilityChanged", (BAR / "src/providers.rs").read_text())

    def test_quick_settings_responsive_layout_actions_and_brand_contracts(self):
        ui = (BAR / "ui/main.slint").read_text()
        source = (BAR / "src/main.rs").read_text()
        quick = ui[ui.index("component QuickSettingsWindow") : ui.index(
            "component UsbWindow"
        )]
        for property_name in (
            "layout-columns",
            "layout-overflows-width",
            "layout-scrolls",
            "layout-content-width",
            "layout-viewport-width",
        ):
            self.assertIn(f"out property <", quick)
            self.assertIn(property_name, quick)
        self.assertIn("viewport-height: max(self.height, quick-content.min-height)", quick)
        for action in (
            'root.toggle("wifi")',
            'root.toggle("bluetooth")',
            'root.toggle("wired")',
            'root.toggle("airplane")',
            'root.toggle("nightlight")',
            'root.toggle("dnd")',
            'root.toggle("dictation")',
            'root.toggle("volume")',
            'root.open-tool("network")',
            'root.open-tool("settings")',
            'root.action("logout-request")',
            'root.action("logout-cancel")',
            'root.action("logout-confirmed")',
            'root.action("poweroff")',
            "root.set-volume(value)",
            "root.set-brightness(value)",
        ):
            self.assertIn(action, quick)
        for implementation in (
            '["brightness-ctl", value, "set"]',
            '["volume-ctl", "set", value]',
            '["volume-ctl", "toggle"]',
            '["nm-connection-editor"]',
            '"smplos-settings"',
            '"smplos-session-logout", "--confirmed"',
            '"systemctl", "poweroff"',
        ):
            self.assertIn(implementation, source)
        self.assertNotIn('"loginctl"', source)
        self.assertNotIn('"terminate-user"', source)
        self.assertIn("logout-confirmation", quick)
        self.assertIn("action-error", quick)
        self.assertIn('@image-url("assets/start.svg")', ui)
        self.assertNotIn('icon: "󰍜"', ui)
        self.assertNotIn("drop-shadow", quick)
        self.assertNotIn("blur", quick.lower())
        slider = ui[
            ui.index("component QuickSlider") : ui.index("component CommandAction")
        ]
        self.assertIn("height: 42px;", slider)
        self.assertIn("alignment: center;", slider)
        self.assertIn("x: 0px;", slider)
        self.assertIn("width: parent.width;", slider)
        self.assertIn(
            "width: parent.width * clamp(root.value, root.minimum, 100) / 100;",
            slider,
        )
        self.assertIn(
            "x: (parent.width - 12px) * clamp(root.value, root.minimum, 100) / 100;",
            slider,
        )
        self.assertIn("width: 12px;", slider)
        self.assertNotIn("max(6px", slider)
        self.assertIn("root.dragging = true;", slider)
        self.assertIn("event.kind == PointerEventKind.move", slider)
        self.assertIn("event.kind == PointerEventKind.up && root.dragging", slider)
        self.assertIn("scroll-event(event) =>", slider)
        self.assertIn("if (event.delta-y != 0px)", slider)
        self.assertIn("event.delta-y < 0px ? 5 : -5", slider)
        self.assertIn("root.change(root.value);", slider)
        self.assertLess(
            slider.index("root.dragging = true;"),
            slider.index("event.kind == PointerEventKind.move"),
        )
        self.assertLess(
            slider.index("event.kind == PointerEventKind.move"),
            slider.index("event.kind == PointerEventKind.up && root.dragging"),
        )
        self.assertEqual(slider.count("root.change(root.value);"), 2)
        self.assertIn("preferred-height: 560px;", quick)
        self.assertIn("dragging <=> root.volume-dragging;", quick)
        self.assertIn("dragging <=> root.brightness-dragging;", quick)
        brightness_start = source.index("ui.on_set_brightness")
        brightness_callback = source[
            brightness_start : source.index(
                "let telemetry = telemetry.clone();", brightness_start
            )
        ]
        self.assertIn("spawn_quick_command(", brightness_callback)
        self.assertIn("if !ui.get_volume_dragging()", source)
        self.assertIn("if !ui.get_brightness_dragging()", source)
        self.assertIn("430.0,\n            560.0,", source)
        start_icon = (BAR / "ui/assets/start.svg").read_text()
        self.assertIn('M 221 95', start_icon)
        self.assertIn('H 109', start_icon)

    def test_quick_settings_two_column_geometry_fits_without_scrolling(self):
        ui = (BAR / "ui/main.slint").read_text()
        quick = ui[ui.index("component QuickSettingsWindow") : ui.index(
            "component UsbWindow"
        )]
        card = ui[
            ui.index("component QuickSettingCard") : ui.index("component QuickSlider")
        ]
        action = ui[
            ui.index("component CommandAction") : ui.index("component BarWindow")
        ]
        header = ui[
            ui.index("component PopupHeader") : ui.index("component QuickSettingCard")
        ]

        self.assertIn("height: 48px;", header)
        self.assertIn("alignment: center;", header)
        self.assertIn("alignment: center;", action)
        self.assertIn("height: 66px;", card)
        self.assertEqual(quick.count("QuickSettingCard {"), 8)
        self.assertIn("root.width >= 340px ? 2 : 1", quick)

        popup_height = 560
        viewport_height = popup_height - 2 - 24 - 48 - 40 - (2 * 8)
        content_height = 4 + 20 + (2 * 42) + 32 + (4 * 66) + (3 * 7)
        self.assertLessEqual(content_height, viewport_height)

    def test_all_native_popups_are_floating_bounded_and_explicitly_dismissed(self):
        source = (BAR / "src/main.rs").read_text()
        compositor = (ROOT / "smpl-shell/src/compositor.rs").read_text()
        for title in (
            "smpl-bar:quick-settings",
            "smpl-bar:usb",
            "smpl-bar:workspace-overview",
            "smpl-bar:keypad-cheatsheet",
            "smpl-bar:tray-menu",
        ):
            self.assertIn(f'"{title}"', source)
        self.assertGreaterEqual(source.count("show_popup_at_anchor("), 6)
        self.assertIn("Request::ClosePopups", source)
        self.assertNotIn("observe_popup_focus", source)
        self.assertNotIn("focus-left-popup", source)
        self.assertIn("Action::PlacePopup", source)
        self.assertIn("resizewindowpixel exact", compositor)
        self.assertIn("set_floating", compositor)
        self.assertIn("active_window_is_popup", (ROOT / "smpl-shell/src/monitor.rs").read_text())


if __name__ == "__main__":
    unittest.main()
