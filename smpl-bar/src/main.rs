mod system;

use chrono::Local;
use i_slint_backend_winit::winit::dpi::LogicalSize;
use i_slint_backend_winit::WinitWindowAccessor;
use serde::Deserialize;
use slint::{ModelRc, VecModel};
use smpl_common::theme::{self, ThemePalette, ThemeRole};
use smpl_shell::compositor::{self, Action, CompositorHandle};
use smpl_shell::config::{
    self, BarConfigFile, ClockDateFormat, ClockFormat, ConfigPaths, Module, WorkspaceStyle,
};
use smpl_shell::ipc::{self, Popup, Request, Response, Server};
use smpl_shell::launch;
use smpl_shell::monitor::CompositorState;
use smpl_shell::telemetry::Telemetry;
use std::cell::{Cell, RefCell};
use std::io;
use std::path::Path;
use std::process::{Child, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use system::{SystemSampler, SystemState, UsbDevice};

slint::include_modules!();

const INITIAL_WIDTH: f64 = 1600.0;
const CONTROL_INTERVAL: Duration = Duration::from_millis(50);
const EXTERNAL_PROBE_INTERVAL: Duration = Duration::from_secs(15);
const PROCESS_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

fn main() {
    if let Err(error) = real_main() {
        eprintln!("smpl-bar: {error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), Box<dyn std::error::Error>> {
    configure_allocator();
    let arguments = Arguments::parse(std::env::args().skip(1))?;
    if arguments.version {
        println!("smpl-bar v{}", env!("CARGO_PKG_VERSION"));
        println!(
            "features: resident hyprland x11 niri-process-gate typed-config telemetry sni-gated"
        );
        return Ok(());
    }
    let paths = config_path()?;
    let config = config::load(&paths)?;
    if arguments.check {
        let mut sampler = SystemSampler::new();
        let system = sampler.sample();
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "config": config,
                "compositor": compositor::detect().as_str(),
                "modules": module_names(&config),
                "system": system_summary(&system),
                "ipc": ipc::socket_path(),
            }))?
        );
        return Ok(());
    }
    if let Some(request) = arguments.control_request {
        return control_existing(request);
    }
    run_resident(config, paths, arguments.hidden, arguments.preview)
}

fn config_path() -> Result<ConfigPaths, io::Error> {
    ConfigPaths::from_env()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME must be an absolute path"))
}

fn configure_allocator() {
    // Keep the resident process from creating one large glibc arena per
    // short-lived helper thread. The compositor adapter and module probe are
    // serialized and do not benefit from allocator parallelism.
    // SAFETY: mallopt accepts the documented GNU integer tuning keys.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, 2);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 64 * 1024);
        libc::mallopt(libc::M_TOP_PAD, 0);
    }
}

fn control_existing(request: Request) -> Result<(), Box<dyn std::error::Error>> {
    let socket = ipc::socket_path().ok_or_else(|| io::Error::other("XDG runtime unavailable"))?;
    let response = ipc::send(&socket, &request)?;
    if !response.ok {
        return Err(
            io::Error::other(response.error.unwrap_or_else(|| "control failed".into())).into(),
        );
    }
    if let Some(status) = response.status {
        println!("{}", serde_json::to_string(&status)?);
    }
    Ok(())
}

fn run_resident(
    config: BarConfigFile,
    paths: ConfigPaths,
    hidden: bool,
    preview: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let process_started = Instant::now();
    let bar_height = config.height;
    let socket = ipc::socket_path().ok_or_else(|| io::Error::other("XDG runtime unavailable"))?;
    let server = match Server::bind(&socket) {
        Ok(server) => server,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let response = ipc::send(&socket, &Request::Show)?;
            if !response.ok {
                return Err(io::Error::other(
                    response
                        .error
                        .unwrap_or_else(|| "resident handoff failed".into()),
                )
                .into());
            }
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    let telemetry = Telemetry::from_env("smpl-bar");
    telemetry.emit(
        "config_loaded",
        [
            ("workspace_count", serde_json::json!(config.workspace_count)),
            ("modules", serde_json::json!(module_names(&config))),
        ],
    );
    telemetry.emit(
        "ipc_bound",
        [(
            "path",
            serde_json::json!(server.path().display().to_string()),
        )],
    );

    // TODO(layer-shell): smpl_common currently creates an xdg-toplevel. Replace
    // this with a shared layer-shell platform before claiming exclusive EWW parity.
    smpl_common::init("smpl-bar", INITIAL_WIDTH, config.height as f64)?;
    telemetry.emit(
        "backend_initialized",
        [("renderer", serde_json::json!("skia-software"))],
    );
    let bar = BarWindow::new()?;
    let state = Rc::new(RefCell::new(UiState::new(config, paths)));
    let popups = Rc::new(RefCell::new(PopupWindows::default()));
    let children = Rc::new(RefCell::new(Vec::<Child>::new()));
    let compositor = Rc::new(compositor::spawn(
        state.borrow().config.clone(),
        telemetry.clone(),
    ));
    let server = Rc::new(RefCell::new(server));
    let telemetry = Rc::new(telemetry);

    bind_bar_callbacks(&bar, &state, &children, &compositor);
    apply_config(&bar, &state.borrow().config);
    apply_system(&bar, &state.borrow().system, &state.borrow().config);
    apply_compositor(&bar, &state.borrow().compositor, &state.borrow().config);
    install_render_telemetry(bar.window(), &telemetry, None);

    let state_for_theme = state.clone();
    let bar_weak = bar.as_weak();
    let bar_theme = theme::watch(ThemeRole::Application, move |palette| {
        state_for_theme.borrow_mut().app_theme = palette.clone();
        if let Some(bar) = bar_weak.upgrade() {
            apply_app_theme(&bar, palette);
        }
    });
    let popups_for_theme = popups.clone();
    let state_for_popup_theme = state.clone();
    let popup_theme = theme::watch(ThemeRole::Popup, move |palette| {
        state_for_popup_theme.borrow_mut().popup_theme = palette.clone();
        apply_popup_theme(&popups_for_theme.borrow(), palette);
    });

    let mut tick = Instant::now();
    let mut last_external = Instant::now() - EXTERNAL_PROBE_INTERVAL;
    let mut last_sample = Instant::now() - PROCESS_SAMPLE_INTERVAL;
    let mut trimmed_heap = false;
    let control_timer = slint::Timer::default();
    let bar_weak = bar.as_weak();
    control_timer.start(slint::TimerMode::Repeated, CONTROL_INTERVAL, {
        let state = state.clone();
        let popups = popups.clone();
        let children = children.clone();
        let compositor = compositor.clone();
        let server = server.clone();
        let telemetry = telemetry.clone();
        move || {
            let loop_started = Instant::now();
            let loop_gap = tick.elapsed();
            tick = Instant::now();
            if loop_gap > Duration::from_millis(120) {
                telemetry.emit(
                    "loop_stall",
                    [("late_us", serde_json::json!(loop_gap.as_micros()))],
                );
            }
            if let Some(bar) = bar_weak.upgrade() {
                if let Some(snapshot) = compositor.latest() {
                    state.borrow_mut().compositor = snapshot.clone();
                    apply_compositor(&bar, &snapshot, &state.borrow().config);
                    fit_bar_to_monitor(&bar, &snapshot, state.borrow().config.height, preview);
                }
                let external_changed = state.borrow_mut().refresh_system();
                let system = state.borrow().system.clone();
                apply_system(&bar, &system, &state.borrow().config);
                if external_changed {
                    if let Some(ui) = &popups.borrow().quick {
                        ui.set_volume(system.volume.into());
                        ui.set_dnd_on(system.notifications_paused);
                    }
                }
                reap_children(&mut children.borrow_mut());
                let requests = server.borrow().drain();
                for (request, mut stream) in requests {
                    let response = handle_request(
                        request,
                        &bar,
                        &state,
                        &popups,
                        &children,
                        &compositor,
                        &telemetry,
                    );
                    let _ = ipc::write_response(&mut stream, &response);
                }
            }
            if last_external.elapsed() >= EXTERNAL_PROBE_INTERVAL {
                last_external = Instant::now();
                refresh_external_async(&state, &popups, &telemetry);
            }
            if last_sample.elapsed() >= PROCESS_SAMPLE_INTERVAL {
                last_sample = Instant::now();
                telemetry.emit_process_sample("periodic");
            }
            if !trimmed_heap && last_sample.elapsed() >= Duration::from_secs(1) {
                trimmed_heap = true;
                trim_allocator_caches();
                telemetry.emit("allocator_trim", []);
            }
            let work = loop_started.elapsed();
            if work > Duration::from_millis(8) {
                telemetry.emit(
                    "ui_poll_work",
                    [("duration_us", serde_json::json!(work.as_micros()))],
                );
            }
        }
    });

    if hidden {
        telemetry.emit("resident_hidden", []);
    } else {
        resize_window(bar.window(), INITIAL_WIDTH, bar_height as f64);
        bar.show()?;
        bar.window().request_redraw();
        if telemetry.enabled() {
            let weak = bar.as_weak();
            let paint_telemetry = telemetry.clone();
            slint::Timer::single_shot(Duration::from_millis(25), move || {
                if let Some(bar) = weak.upgrade() {
                    record_snapshot_paint(
                        bar.window(),
                        &paint_telemetry,
                        process_started,
                        "first_paint",
                    );
                }
            });
        }
        telemetry.emit("first_map_requested", []);
        telemetry.emit_process_sample("bar_shown");
    }
    let _theme_lifetimes = (bar_theme, popup_theme);
    slint::run_event_loop_until_quit()?;
    telemetry.emit("process_exit", []);
    Ok(())
}

fn bind_bar_callbacks(
    bar: &BarWindow,
    state: &Rc<RefCell<UiState>>,
    children: &Rc<RefCell<Vec<Child>>>,
    compositor: &Rc<CompositorHandle>,
) {
    let popup_coordinator = Rc::new(PopupCoordinator::new(children.clone()));
    let children_action = children.clone();
    bar.on_open_start(move || spawn(&children_action, ["toggle-start-menu"]));
    let compositor_action = compositor.clone();
    bar.on_select_workspace(move |workspace| {
        compositor_action.send(Action::SelectWorkspace(workspace));
    });
    let overview = popup_coordinator.clone();
    bar.on_open_overview(move || overview.request(PopupTarget::WorkspaceOverview));
    let quick = popup_coordinator.clone();
    bar.on_open_quick_settings(move || quick.request(PopupTarget::QuickSettings));
    let usb = popup_coordinator.clone();
    bar.on_open_usb(move || usb.request(PopupTarget::Usb));
    let keypad_children = children.clone();
    bar.on_open_keypad(move || spawn(&keypad_children, ["smplos-settings", "keypad"]));
    let sync_children = children.clone();
    bar.on_open_sync(move || spawn(&sync_children, ["sync-center-gui"]));
    let dictation_children = children.clone();
    bar.on_toggle_dictation(move || spawn(&dictation_children, ["dictation-toggle"]));
    let keyboard_compositor = compositor.clone();
    bar.on_next_keyboard(move || keyboard_compositor.send(Action::NextKeyboardLayout));
    let keyboard_children = children.clone();
    bar.on_open_keyboard_settings(move || {
        spawn(&keyboard_children, ["smplos-settings", "keyboard"])
    });
    let calendar = popup_coordinator.clone();
    bar.on_open_calendar(move || calendar.request(PopupTarget::Calendar));
    let notifications = popup_coordinator.clone();
    bar.on_open_notifications(move || notifications.request(PopupTarget::Notifications));
    let clear_children = children.clone();
    bar.on_clear_notifications(move || spawn(&clear_children, ["dunstctl", "history-clear"]));
    let volume_children = children.clone();
    let state_volume = state.clone();
    let bar_weak = bar.as_weak();
    bar.on_volume_scroll(move |delta| {
        let direction = if delta < 0.0 { "up" } else { "down" };
        state_volume.borrow_mut().system.volume = state_volume
            .borrow()
            .system
            .volume
            .saturating_add_signed(if direction == "up" { 5 } else { -5 });
        spawn(&volume_children, ["volume-ctl", direction, "5"]);
        if let Some(bar) = bar_weak.upgrade() {
            apply_system(
                &bar,
                &state_volume.borrow().system,
                &state_volume.borrow().config,
            );
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupTarget {
    QuickSettings,
    Usb,
    WorkspaceOverview,
    Calendar,
    Notifications,
}

impl PopupTarget {
    fn argument(self) -> &'static str {
        match self {
            Self::QuickSettings => "quick-settings",
            Self::Usb => "usb-popup",
            Self::WorkspaceOverview => "workspace-overview",
            Self::Calendar => "calendar-popup",
            Self::Notifications => "notification-hub",
        }
    }
}

struct PopupCoordinator {
    children: Rc<RefCell<Vec<Child>>>,
}

impl PopupCoordinator {
    fn new(children: Rc<RefCell<Vec<Child>>>) -> Self {
        Self { children }
    }

    fn request(&self, target: PopupTarget) {
        spawn(&self.children, ["popup-toggle", target.argument()]);
    }
}

fn handle_request(
    request: Request,
    bar: &BarWindow,
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    children: &Rc<RefCell<Vec<Child>>>,
    compositor: &Rc<CompositorHandle>,
    telemetry: &Rc<Telemetry>,
) -> Response {
    match request {
        Request::Show => {
            let _ = bar.show();
            Response::ok(None)
        }
        Request::Hide => {
            let _ = bar.hide();
            Response::ok(None)
        }
        Request::Toggle => {
            let visible = bar.window().is_visible();
            let _ = if visible { bar.hide() } else { bar.show() };
            Response::ok(None)
        }
        Request::Apply => match reload_config(&state.borrow().paths) {
            Ok(config) => {
                state.borrow_mut().config = config.clone();
                apply_config(bar, &config);
                apply_compositor(bar, &state.borrow().compositor, &config);
                telemetry.emit("config_reloaded", []);
                Response::ok(None)
            }
            Err(error) => Response::error(error.to_string()),
        },
        Request::ClosePopups => {
            popups.borrow_mut().close_all();
            Response::ok(None)
        }
        Request::Popup { name } => {
            match name {
                Popup::QuickSettings => {
                    open_quick_settings(state, popups, children, telemetry);
                    refresh_external_async(state, popups, telemetry);
                }
                Popup::Usb => open_usb(state, popups, children, telemetry),
                Popup::WorkspaceOverview => open_overview(state, popups, children, telemetry),
                Popup::KeypadCheatsheet => {}
            }

            Response::ok(None)
        }
        Request::Identify => {
            open_identify(state, popups, telemetry);
            Response::ok(None)
        }
        Request::SetKeypadPresent { present } => {
            {
                let mut state = state.borrow_mut();
                state.keypad_override = Some(present);
                state.system.keypad_present = present;
            }
            apply_system(bar, &state.borrow().system, &state.borrow().config);
            Response::ok(None)
        }
        Request::KeypadSheet { sheet } => {
            match apply_keypad_sheet(&sheet, state, popups, telemetry) {
                Ok(()) => Response::ok(None),
                Err(error) => Response::error(error.to_string()),
            }
        }
        Request::TelemetrySample => {
            telemetry.emit_process_sample("ipc");
            Response::ok(None)
        }
        Request::Status => Response::ok(Some(status_json(state))),
        Request::Quit => {
            compositor.send(Action::Quit);
            let _ = slint::quit_event_loop();
            Response::ok(None)
        }
    }
}

fn apply_config(bar: &BarWindow, config: &BarConfigFile) {
    let modules: std::collections::HashSet<_> =
        config.left.iter().chain(&config.right).copied().collect();
    let contains = |module| modules.contains(&module);
    bar.set_show_start(contains(Module::Start));
    bar.set_show_workspaces(contains(Module::Workspaces));
    bar.set_show_title(contains(Module::WindowTitle));
    bar.set_show_usb(contains(Module::Usb));
    bar.set_show_keypad(contains(Module::Keypad));
    bar.set_show_sync(contains(Module::Sync));
    bar.set_show_dictation(contains(Module::Dictation));
    bar.set_show_keyboard(contains(Module::Keyboard));
    bar.set_show_bluetooth(contains(Module::Bluetooth));
    bar.set_show_wifi(contains(Module::Wifi));
    bar.set_show_network(contains(Module::Network));
    bar.set_show_volume(contains(Module::Volume));
    bar.set_show_battery(contains(Module::Battery));
    bar.set_show_clock(contains(Module::Clock));
    bar.set_show_notifications(contains(Module::Notifications));
    bar.set_workspace_squares(config.workspace_style == WorkspaceStyle::Squares);
    bar.set_workspace_spacing(config.workspace_spacing as f32);
    bar.set_workspace_center(matches!(
        config.workspace_position,
        config::WorkspacePosition::Center
    ));
}

fn apply_system(bar: &BarWindow, system: &SystemState, config: &BarConfigFile) {
    bar.set_battery_visible(system.battery_present);
    bar.set_battery(system.battery_level.into());
    bar.set_battery_low(system.battery_low);
    bar.set_battery_charging(system.battery_charging);
    bar.set_volume(system.volume.into());
    bar.set_volume_muted(system.volume_muted);
    bar.set_usb_visible(!system.usb.is_empty());
    bar.set_keypad_visible(system.keypad_present);
    bar.set_sync_visible(system.sync_active);
    bar.set_dictation_visible(system.dictation_available);
    bar.set_keyboard_visible(!bar.get_keyboard_code().is_empty());
    bar.set_wifi_available(system.wifi_available);
    bar.set_wifi_on(system.wifi_on);
    bar.set_network_online(system.online);
    bar.set_vpn_on(system.vpn);
    bar.set_bluetooth_available(system.bluetooth_available);
    bar.set_bluetooth_on(system.bluetooth_on);
    bar.set_notification_count(system.notification_count as i32);
    bar.set_notifications_paused(system.notifications_paused);
    update_clock(bar, config);
}

fn apply_compositor(bar: &BarWindow, state: &CompositorState, config: &BarConfigFile) {
    bar.set_window_title(state.active_window_title.clone().into());
    bar.set_compositor(state.kind.as_str().into());
    bar.set_monitor_label(
        if state.focused_monitor.is_empty() {
            state
                .monitors
                .first()
                .map(|monitor| monitor.connector.as_str())
                .unwrap_or("WS")
        } else {
            &state.focused_monitor
        }
        .into(),
    );
    bar.set_keyboard_code(state.keyboard.code.clone().into());
    bar.set_keyboard_visible(state.keyboard.visible);
    let workspaces: Vec<_> = state
        .workspace_for_bar(config.workspace_count)
        .into_iter()
        .map(|workspace| WorkspaceUi {
            id: workspace.id,
            label: workspace.label(config.workspace_style).into(),
            occupied: workspace.occupied,
            visible: workspace.visible,
            focused: workspace.focused,
            temporary: workspace.temporary,
        })
        .collect();
    bar.set_workspaces(ModelRc::new(VecModel::from_slice(&workspaces)));
}

fn trim_allocator_caches() {
    // The IPC snapshots and first module probe allocate short-lived JSON and
    // command buffers. Releasing glibc's top chunk keeps a resident preload
    // from retaining those one-time pages.
    // SAFETY: malloc_trim has no pointer arguments and is process-local.
    unsafe {
        libc::malloc_trim(0);
    }
}

fn fit_bar_to_monitor(bar: &BarWindow, state: &CompositorState, height: usize, preview: bool) {
    let Some(monitor) = state.primary_monitor() else {
        return;
    };
    let scale = monitor.scale.max(0.5);
    let logical_width = (monitor.width as f64 / scale) * if preview { 0.72 } else { 0.997 };
    resize_window(bar.window(), logical_width, height as f64);
}

fn resize_window(window: &slint::Window, width: f64, height: f64) {
    window.with_winit_window(|winit: &i_slint_backend_winit::winit::window::Window| {
        let _ = winit.request_inner_size(LogicalSize::new(width, height));
    });
}

fn update_clock(bar: &BarWindow, config: &BarConfigFile) {
    let now = Local::now();
    let time_format = if config.clock_24h {
        "%H:%M"
    } else {
        "%-I:%M %p"
    };
    let date = match config.clock_format {
        ClockFormat::Time => String::new(),
        ClockFormat::Dow => now.format("%a").to_string(),
        ClockFormat::Date => now
            .format(match config.clock_date_format {
                ClockDateFormat::MonthDay => "%-m/%-d",
                ClockDateFormat::DayMonth => "%-d/%-m",
                ClockDateFormat::Iso => "%Y-%m-%d",
                ClockDateFormat::MonthNameDay => "%b %-d",
            })
            .to_string(),
    };
    bar.set_clock_main(now.format(time_format).to_string().into());
    bar.set_clock_sub(date.into());
}

fn apply_app_theme(bar: &BarWindow, palette: &ThemePalette) {
    let theme = bar.global::<Theme>();
    apply_theme_values(&theme, palette);
}

fn apply_popup_theme(windows: &PopupWindows, palette: &ThemePalette) {
    if let Some(window) = &windows.quick {
        apply_theme_values(&window.global::<Theme>(), palette);
    }
    if let Some(window) = &windows.usb {
        apply_theme_values(&window.global::<Theme>(), palette);
    }
    if let Some(window) = &windows.overview {
        apply_theme_values(&window.global::<Theme>(), palette);
    }
    if let Some(window) = &windows.keypad {
        apply_theme_values(&window.global::<Theme>(), palette);
    }
    if let Some(window) = &windows.identify {
        apply_theme_values(&window.global::<Theme>(), palette);
    }
}

fn apply_theme_values(theme: &Theme, palette: &ThemePalette) {
    theme.set_bg(palette.bg);
    theme.set_fg(palette.fg);
    theme.set_fg_dim(palette.fg_dim);
    theme.set_accent(palette.accent);
    theme.set_bg_light(palette.bg_light);
    theme.set_bg_lighter(palette.bg_lighter);
    theme.set_danger(palette.danger);
    theme.set_success(palette.success);
    theme.set_warning(palette.warning);
    theme.set_info(palette.info);
    theme.set_opacity(palette.opacity);
    theme.set_popup_opacity(palette.opacity);
}

fn install_render_telemetry(
    window: &slint::Window,
    telemetry: &Rc<Telemetry>,
    popup_started: Option<Instant>,
) {
    if !telemetry.enabled() {
        return;
    }
    let notifier_telemetry = telemetry.clone();
    let error_telemetry = telemetry.clone();
    let first_paint = Rc::new(Cell::new(false));
    if let Err(error) = window.set_rendering_notifier(move |state, _graphics| match state {
        slint::RenderingState::RenderingSetup => {
            notifier_telemetry.emit("renderer_ready", []);
            notifier_telemetry.emit_process_sample("renderer_ready");
        }
        slint::RenderingState::AfterRendering if !first_paint.replace(true) => {
            if let Some(started) = popup_started {
                notifier_telemetry.duration_since(started, "popup_first_paint", &[]);
            } else {
                notifier_telemetry.emit("first_paint", []);
            }
            notifier_telemetry.emit_process_sample("first_paint");
        }
        slint::RenderingState::RenderingTeardown => {
            notifier_telemetry.emit("renderer_teardown", [])
        }
        _ => {}
    }) {
        error_telemetry.emit(
            "renderer_notifier_unavailable",
            [("error", serde_json::json!(error.to_string()))],
        );
    }
}

fn record_snapshot_paint(
    window: &slint::Window,
    telemetry: &Telemetry,
    started: Instant,
    event: &'static str,
) {
    match window.take_snapshot() {
        Ok(snapshot) => telemetry.duration_since(
            started,
            event,
            &[
                ("source", serde_json::json!("slint-snapshot")),
                ("width", serde_json::json!(snapshot.width())),
                ("height", serde_json::json!(snapshot.height())),
            ],
        ),
        Err(error) => telemetry.emit(
            "paint_snapshot_failed",
            [("error", serde_json::json!(error.to_string()))],
        ),
    }
}

fn open_quick_settings(
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    children: &Rc<RefCell<Vec<Child>>>,
    telemetry: &Rc<Telemetry>,
) {
    let started = Instant::now();
    {
        let mut windows = popups.borrow_mut();
        windows.close_all();
        if windows.quick.is_none() {
            let ui = QuickSettingsWindow::new().expect("Slint window construction");
            bind_quick(&ui, state, children, popups, telemetry);
            apply_theme_values(&ui.global::<Theme>(), &state.borrow().popup_theme);
            windows.quick = Some(ui);
        }
        let ui = windows.quick.as_ref().expect("quick settings was created");
        let system = state.borrow().system.clone();
        ui.set_wifi_available(system.wifi_available);
        ui.set_bluetooth_available(system.bluetooth_available);
        ui.set_wired_available(system.wired_available);
        ui.set_wifi_on(system.wifi_on);
        ui.set_bluetooth_on(system.bluetooth_on);
        ui.set_wired_on(system.wired);
        ui.set_airplane_on(system.airplane);
        ui.set_nightlight_on(system.nightlight);
        ui.set_nightlight_available(
            state.borrow().compositor.kind != smpl_shell::monitor::CompositorKind::X11,
        );
        ui.set_dnd_on(system.notifications_paused);
        ui.set_dictation_on(system.dictation_active);
        ui.set_dictation_available(system.dictation_available);
        ui.set_volume(system.volume.into());
        ui.set_brightness(system.brightness.into());
        ui.set_network_label(
            if system.ssid.is_empty() {
                if system.wifi_on {
                    "Connected".to_owned()
                } else {
                    "Off".to_owned()
                }
            } else {
                system.ssid.clone()
            }
            .into(),
        );
        ui.set_bluetooth_label(if system.bluetooth_on { "On" } else { "Off" }.into());
        install_render_telemetry(ui.window(), telemetry, Some(started));
        resize_window(ui.window(), 430.0, 340.0);
        let _ = ui.show();
        if telemetry.enabled() {
            let weak = ui.as_weak();
            let paint_telemetry = telemetry.clone();
            slint::Timer::single_shot(Duration::from_millis(25), move || {
                if let Some(ui) = weak.upgrade() {
                    record_snapshot_paint(
                        ui.window(),
                        &paint_telemetry,
                        started,
                        "popup_first_paint",
                    );
                }
            });
        }
    }
    telemetry.emit(
        "popup_open_requested",
        [
            ("popup", serde_json::json!("quick-settings")),
            (
                "duration_us",
                serde_json::json!(started.elapsed().as_micros()),
            ),
        ],
    );
}

fn bind_quick(
    ui: &QuickSettingsWindow,
    state: &Rc<RefCell<UiState>>,
    children: &Rc<RefCell<Vec<Child>>>,
    popups: &Rc<RefCell<PopupWindows>>,
    telemetry: &Rc<Telemetry>,
) {
    let children_toggle = children.clone();
    let state_toggle = state.clone();
    let weak = ui.as_weak();
    ui.on_toggle(move |name| {
        let command = match name.as_str() {
            "wifi" => "wifi-toggle",
            "bluetooth" => "bt-toggle",
            "wired" => "wired-toggle",
            "airplane" => "airplane-toggle",
            "nightlight" => "nightlight-toggle",
            "dnd" => "dnd-toggle",
            "dictation" => "dictation-toggle",
            _ => return,
        };
        spawn(&children_toggle, [command]);
        if let Some(ui) = weak.upgrade() {
            match name.as_str() {
                "wifi" => {
                    ui.set_wifi_on(!ui.get_wifi_on());
                    state_toggle.borrow_mut().system.wifi_on = ui.get_wifi_on();
                }
                "bluetooth" => {
                    ui.set_bluetooth_on(!ui.get_bluetooth_on());
                    state_toggle.borrow_mut().system.bluetooth_on = ui.get_bluetooth_on();
                }
                "wired" => {
                    ui.set_wired_on(!ui.get_wired_on());
                    state_toggle.borrow_mut().system.wired = ui.get_wired_on();
                }
                "airplane" => {
                    ui.set_airplane_on(!ui.get_airplane_on());
                    state_toggle.borrow_mut().system.airplane = ui.get_airplane_on();
                }
                "nightlight" => {
                    ui.set_nightlight_on(!ui.get_nightlight_on());
                    state_toggle.borrow_mut().system.nightlight = ui.get_nightlight_on();
                }
                "dnd" => {
                    ui.set_dnd_on(!ui.get_dnd_on());
                    state_toggle.borrow_mut().system.notifications_paused = ui.get_dnd_on();
                }
                "dictation" => {
                    ui.set_dictation_on(!ui.get_dictation_on());
                    state_toggle.borrow_mut().system.dictation_active = ui.get_dictation_on();
                }
                _ => {}
            }
        }
    });
    let children_tools = children.clone();
    ui.on_open_tool(move |tool| {
        if tool.as_str() == "settings" {
            spawn(&children_tools, ["smplos-settings"]);
        }
    });
    let children_action = children.clone();
    ui.on_action(move |action| match action.as_str() {
        "logout" => {
            if let Some(user) = std::env::var_os("USER") {
                spawn_owned(
                    &children_action,
                    vec![
                        "loginctl".to_owned(),
                        "terminate-user".to_owned(),
                        user.to_string_lossy().into_owned(),
                    ],
                );
            }
        }
        "poweroff" => spawn(&children_action, ["systemctl", "poweroff"]),
        _ => {}
    });
    let children_volume = children.clone();
    let state_volume = state.clone();
    ui.on_set_volume(move |value| {
        state_volume.borrow_mut().system.volume = value as u8;
        spawn(&children_volume, ["volume-ctl", "set", &value.to_string()]);
    });
    let children_brightness = children.clone();
    let state_brightness = state.clone();
    ui.on_set_brightness(move |value| {
        state_brightness.borrow_mut().system.brightness = value as u8;
        spawn(
            &children_brightness,
            ["brightness-ctl", &value.to_string(), "set"],
        );
    });
    let telemetry = telemetry.clone();
    ui.on_close({
        let popups = popups.clone();
        move || {
            popups.borrow_mut().close_all();
            telemetry.emit(
                "popup_closed",
                [("popup", serde_json::json!("quick-settings"))],
            );
        }
    });
}

fn open_usb(
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    children: &Rc<RefCell<Vec<Child>>>,
    telemetry: &Rc<Telemetry>,
) {
    let started = Instant::now();
    let mut windows = popups.borrow_mut();
    windows.close_all();
    if windows.usb.is_none() {
        let ui = UsbWindow::new().expect("USB Slint window");
        apply_theme_values(&ui.global::<Theme>(), &state.borrow().popup_theme);
        let close = popups.clone();
        ui.on_close(move || close.borrow_mut().close_all());
        let children_eject = children.clone();
        ui.on_eject(move |name, mounted| {
            if mounted {
                spawn(
                    &children_eject,
                    [
                        "udisksctl",
                        "unmount",
                        "--block-device",
                        &format!("/dev/{name}"),
                        "--no-user-interaction",
                    ],
                );
            } else {
                spawn(
                    &children_eject,
                    [
                        "udisksctl",
                        "power-off",
                        "--block-device",
                        &format!("/dev/{name}"),
                    ],
                );
            }
        });
        windows.usb = Some(ui);
    }
    let ui = windows.usb.as_ref().expect("USB window");
    let devices: Vec<_> = state
        .borrow()
        .system
        .usb
        .iter()
        .cloned()
        .map(usb_model)
        .collect();
    ui.set_devices(ModelRc::new(VecModel::from_slice(&devices)));
    install_render_telemetry(ui.window(), telemetry, Some(started));
    resize_window(
        ui.window(),
        360.0,
        (150 + devices.len().saturating_mul(58)) as f64,
    );
    let _ = ui.show();
    if telemetry.enabled() {
        let weak = ui.as_weak();
        let paint_telemetry = telemetry.clone();
        slint::Timer::single_shot(Duration::from_millis(25), move || {
            if let Some(ui) = weak.upgrade() {
                record_snapshot_paint(ui.window(), &paint_telemetry, started, "popup_first_paint");
            }
        });
    }
    telemetry.emit(
        "popup_open_requested",
        [("popup", serde_json::json!("usb"))],
    );
}

fn open_overview(
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    children: &Rc<RefCell<Vec<Child>>>,
    telemetry: &Rc<Telemetry>,
) {
    let started = Instant::now();
    let mut windows = popups.borrow_mut();
    windows.close_all();
    if windows.overview.is_none() {
        let ui = WorkspaceOverviewWindow::new().expect("workspace overview");
        apply_theme_values(&ui.global::<Theme>(), &state.borrow().popup_theme);
        let close = popups.clone();
        ui.on_close(move || close.borrow_mut().close_all());
        let children_select = children.clone();
        let close = popups.clone();
        ui.on_select(move |workspace| {
            spawn(
                &children_select,
                ["workspace-group", &workspace.to_string()],
            );
            close.borrow_mut().close_all();
        });
        windows.overview = Some(ui);
    }
    let ui = windows.overview.as_ref().expect("overview window");
    let snapshot = state.borrow().compositor.clone();
    ui.set_focused_monitor(snapshot.focused_monitor.clone().into());
    ui.set_error(snapshot.error.clone().into());
    let monitors: Vec<_> = snapshot
        .monitors
        .iter()
        .map(|monitor| MonitorUi {
            connector: monitor.connector.clone().into(),
            description: monitor.description.clone().into(),
            active_workspace: monitor.active_workspace,
            focused: monitor.focused,
        })
        .collect();
    ui.set_monitors(ModelRc::new(VecModel::from_slice(&monitors)));
    let workspaces: Vec<_> = snapshot
        .workspace_for_bar(state.borrow().config.workspace_count)
        .into_iter()
        .map(|workspace| WorkspaceUi {
            id: workspace.id,
            label: workspace.id.to_string().into(),
            occupied: workspace.occupied,
            visible: workspace.visible,
            focused: workspace.focused,
            temporary: workspace.temporary,
        })
        .collect();
    ui.set_workspaces(ModelRc::new(VecModel::from_slice(&workspaces)));
    install_render_telemetry(ui.window(), telemetry, Some(started));
    resize_window(
        ui.window(),
        620.0,
        (170 + monitors.len().saturating_mul(50)) as f64,
    );
    let _ = ui.show();
    if telemetry.enabled() {
        let weak = ui.as_weak();
        let paint_telemetry = telemetry.clone();
        slint::Timer::single_shot(Duration::from_millis(25), move || {
            if let Some(ui) = weak.upgrade() {
                record_snapshot_paint(ui.window(), &paint_telemetry, started, "popup_first_paint");
            }
        });
    }
    telemetry.emit(
        "popup_open_requested",
        [("popup", serde_json::json!("workspace-overview"))],
    );
}

fn open_identify(
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    telemetry: &Rc<Telemetry>,
) {
    let started = Instant::now();
    let mut windows = popups.borrow_mut();
    windows.close_all();
    if windows.identify.is_none() {
        let ui = IdentifyWindow::new().expect("identify window");
        apply_theme_values(&ui.global::<Theme>(), &state.borrow().popup_theme);
        windows.identify = Some(ui);
    }
    let ui = windows.identify.as_ref().expect("identify");
    let label = state.borrow().compositor.focused_monitor.clone();
    ui.set_label(if label.is_empty() {
        "Monitor".into()
    } else {
        label.into()
    });
    install_render_telemetry(ui.window(), telemetry, Some(started));
    resize_window(ui.window(), 380.0, 140.0);
    let _ = ui.show();
    if telemetry.enabled() {
        let weak = ui.as_weak();
        let paint_telemetry = telemetry.clone();
        slint::Timer::single_shot(Duration::from_millis(25), move || {
            if let Some(ui) = weak.upgrade() {
                record_snapshot_paint(ui.window(), &paint_telemetry, started, "popup_first_paint");
            }
        });
    }
    slint::Timer::single_shot(Duration::from_secs(3), {
        let popups = popups.clone();
        move || {
            if let Some(window) = popups.borrow().identify.as_ref() {
                let _ = window.hide();
            }
        }
    });
}

fn apply_keypad_sheet(
    value: &serde_json::Value,
    state: &Rc<RefCell<UiState>>,
    popups: &Rc<RefCell<PopupWindows>>,
    telemetry: &Rc<Telemetry>,
) -> Result<(), serde_json::Error> {
    let sheet: KeypadSheet = serde_json::from_value(value.clone())?;
    let mut windows = popups.borrow_mut();
    if !sheet.visible {
        if let Some(window) = &windows.keypad {
            let _ = window.hide();
        }
        return Ok(());
    }
    windows.close_all();
    if windows.keypad.is_none() {
        let ui = KeypadCheatsheetWindow::new().expect("keypad sheet window");
        apply_theme_values(&ui.global::<Theme>(), &state.borrow().popup_theme);
        let close = popups.clone();
        ui.on_close(move || close.borrow_mut().close_all());
        windows.keypad = Some(ui);
    }
    let ui = windows.keypad.as_ref().expect("keypad sheet");
    ui.set_sheet_title(sheet.title.into());
    ui.set_notice(sheet.notice.unwrap_or_default().into());
    ui.set_click_through(
        sheet
            .options
            .and_then(|option| option.click_through)
            .unwrap_or(false),
    );
    let mut keys: Vec<_> = sheet
        .keys
        .into_iter()
        .map(|item| KeypadItemUi {
            control: item.control.into(),
            label: item.label.into(),
            icon: item.icon.unwrap_or_default().into(),
            active: item.active,
            bound: item.bound,
            state: item.state.unwrap_or_default().into(),
            row: item.row,
            column: item.column,
        })
        .collect();
    for knob in sheet.knobs {
        for (offset, direction, binding) in [
            (0, "ccw", knob.ccw),
            (1, "press", knob.press),
            (2, "cw", knob.cw),
        ] {
            keys.push(KeypadItemUi {
                control: format!("{} {direction}", knob.control).into(),
                label: binding.label.into(),
                icon: binding.icon.unwrap_or_default().into(),
                active: binding.active,
                bound: binding.bound,
                state: binding.state.unwrap_or_default().into(),
                row: knob.row * 3 + offset,
                column: 6 + knob.column,
            });
        }
    }
    ui.set_keys(ModelRc::new(VecModel::from_slice(&keys)));
    resize_window(ui.window(), 720.0, 360.0);
    let started = Instant::now();
    install_render_telemetry(ui.window(), telemetry, Some(started));
    let _ = ui.show();
    if telemetry.enabled() {
        let weak = ui.as_weak();
        let paint_telemetry = telemetry.clone();
        slint::Timer::single_shot(Duration::from_millis(25), move || {
            if let Some(ui) = weak.upgrade() {
                record_snapshot_paint(ui.window(), &paint_telemetry, started, "popup_first_paint");
            }
        });
    }
    telemetry.emit(
        "module_discovery",
        [("module", serde_json::json!("keypad-sheet"))],
    );
    Ok(())
}

fn usb_model(device: UsbDevice) -> UsbUi {
    UsbUi {
        name: device.name.into(),
        label: device.label.into(),
        path: device.path.into(),
        size: device.size.into(),
        mounted: device.mounted,
    }
}

fn refresh_external_async(
    state: &Rc<RefCell<UiState>>,
    _popups: &Rc<RefCell<PopupWindows>>,
    telemetry: &Rc<Telemetry>,
) {
    let slot = state.borrow().external_probe.clone();
    let telemetry = (**telemetry).clone();
    telemetry.emit(
        "module_discovery",
        [("phase", serde_json::json!("external-probe-start"))],
    );
    let _ = std::thread::Builder::new()
        .name("module-probe".to_owned())
        .spawn(move || {
            let probe = probe_external_state();
            telemetry.emit(
                "module_discovery",
                [
                    ("phase", serde_json::json!("external-probe-complete")),
                    ("volume", serde_json::json!(probe.volume)),
                    ("notifications", serde_json::json!(probe.notification_count)),
                ],
            );
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(probe);
        });
}

#[derive(Default)]
struct ExternalProbe {
    volume: u8,
    muted: bool,
    notification_count: u32,
    notifications_paused: bool,
    sync_active: bool,
}

fn probe_external_state() -> ExternalProbe {
    let mut probe = ExternalProbe {
        volume: 50,
        ..ExternalProbe::default()
    };
    if let Ok(output) = launch::command("pactl")
        .args(["get-sink-volume", "@DEFAULT_SINK@"])
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            probe.volume = first_percentage(&String::from_utf8_lossy(&output.stdout)).unwrap_or(50);
        }
    }
    if let Ok(output) = launch::command("pactl")
        .args(["get-sink-mute", "@DEFAULT_SINK@"])
        .stderr(Stdio::null())
        .output()
    {
        probe.muted = String::from_utf8_lossy(&output.stdout).contains("yes");
    }
    if let Ok(output) = launch::command("dunstctl")
        .args(["count", "history"])
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            probe.notification_count = String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
        }
    }
    if let Ok(output) = launch::command("dunstctl")
        .arg("is-paused")
        .stderr(Stdio::null())
        .output()
    {
        probe.notifications_paused = String::from_utf8_lossy(&output.stdout).trim() == "true";
    }
    if let Ok(output) = launch::command("busctl")
        .args([
            "--user",
            "get-property",
            "org.smpl.SyncCenter",
            "/org/smpl/SyncCenter",
            "org.smpl.SyncCenter",
            "IsActive",
        ])
        .stderr(Stdio::null())
        .output()
    {
        probe.sync_active = output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .last()
                == Some("true");
    }
    probe
}

fn first_percentage(source: &str) -> Option<u8> {
    source
        .split(|character: char| !character.is_ascii_digit())
        .find_map(|number| (!number.is_empty()).then(|| number.parse().ok()).flatten())
        .map(|value: u8| value.min(100))
}

fn spawn<const N: usize>(children: &Rc<RefCell<Vec<Child>>>, arguments: [&str; N]) {
    if arguments.is_empty() {
        return;
    }
    let mut command = launch::command(arguments[0]);
    command
        .args(&arguments[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(child) => {
            let mut children = children.borrow_mut();
            if children.len() > 32 {
                children.retain_mut(|child| child.try_wait().ok().flatten().is_none());
            }
            children.push(child);
        }
        Err(error) => eprintln!("smpl-bar: cannot launch {}: {error}", arguments[0]),
    }
}

fn spawn_owned(children: &Rc<RefCell<Vec<Child>>>, arguments: Vec<String>) {
    if arguments.is_empty() {
        return;
    }
    let mut command = launch::command(&arguments[0]);
    command
        .args(&arguments[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match command.spawn() {
        Ok(child) => children.borrow_mut().push(child),
        Err(error) => eprintln!("smpl-bar: cannot launch {}: {error}", arguments[0]),
    }
}

fn reap_children(children: &mut Vec<Child>) {
    children.retain_mut(|child| child.try_wait().ok().flatten().is_none());
}

fn reload_config(paths: &ConfigPaths) -> Result<BarConfigFile, config::ConfigError> {
    config::load(paths)
}

fn status_json(state: &Rc<RefCell<UiState>>) -> serde_json::Value {
    let state = state.borrow();
    serde_json::json!({
        "config": state.config,
        "compositor": state.compositor.kind.as_str(),
        "focused_workspace": state.compositor.focused_workspace,
        "window_title": state.compositor.active_window_title,
        "system": system_summary(&state.system),
        "compositor_error": state.compositor.error,
        "generic_sni_gate": "not-implemented",
    })
}

fn system_summary(state: &SystemState) -> serde_json::Value {
    serde_json::json!({
        "battery": state.battery_level,
        "battery_present": state.battery_present,
        "brightness": state.brightness,
        "network_online": state.online,
        "wifi": state.wifi_on,
        "bluetooth": state.bluetooth_on,
        "volume": state.volume,
        "volume_muted": state.volume_muted,
        "notifications": state.notification_count,
        "usb": state.usb.len(),
        "keypad": state.keypad_present,
    })
}

fn module_names(config: &BarConfigFile) -> Vec<&'static str> {
    config
        .left
        .iter()
        .chain(&config.right)
        .map(|module| match module {
            Module::Start => "start",
            Module::Workspaces => "workspaces",
            Module::WindowTitle => "window-title",
            Module::Usb => "usb",
            Module::Keypad => "keypad",
            Module::Sync => "sync",
            Module::Dictation => "dictation",
            Module::Keyboard => "keyboard",
            Module::Bluetooth => "bluetooth",
            Module::Wifi => "wifi",
            Module::Network => "network",
            Module::Volume => "volume",
            Module::Battery => "battery",
            Module::Clock => "clock",
            Module::Notifications => "notifications",
            Module::Sni => "sni",
        })
        .collect()
}

#[derive(Default)]
struct PopupWindows {
    quick: Option<QuickSettingsWindow>,
    usb: Option<UsbWindow>,
    overview: Option<WorkspaceOverviewWindow>,
    keypad: Option<KeypadCheatsheetWindow>,
    identify: Option<IdentifyWindow>,
}

impl PopupWindows {
    fn close_all(&mut self) {
        if let Some(window) = &self.quick {
            let _ = window.hide();
        }
        if let Some(window) = &self.usb {
            let _ = window.hide();
        }
        if let Some(window) = &self.overview {
            let _ = window.hide();
        }
        if let Some(window) = &self.keypad {
            let _ = window.hide();
        }
        if let Some(window) = &self.identify {
            let _ = window.hide();
        }
    }
}

struct UiState {
    config: BarConfigFile,
    paths: ConfigPaths,
    sampler: SystemSampler,
    system: SystemState,
    compositor: CompositorState,
    app_theme: ThemePalette,
    popup_theme: ThemePalette,
    keypad_override: Option<bool>,
    last_system_sample: Instant,
    external_probe: Arc<Mutex<Option<ExternalProbe>>>,
}

impl UiState {
    fn new(config: BarConfigFile, paths: ConfigPaths) -> Self {
        let mut sampler = SystemSampler::new();
        Self {
            system: sampler.sample(),
            config,
            paths,
            sampler,
            compositor: CompositorState::default(),
            app_theme: ThemePalette::default(),
            popup_theme: ThemePalette::default(),
            keypad_override: None,
            last_system_sample: Instant::now(),
            external_probe: Arc::new(Mutex::new(None)),
        }
    }

    fn refresh_system(&mut self) -> bool {
        let probe = self
            .external_probe
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let external_changed = if let Some(probe) = probe {
            self.system.volume = probe.volume;
            self.system.volume_muted = probe.muted;
            self.system.notification_count = probe.notification_count;
            self.system.notifications_paused = probe.notifications_paused;
            self.system.sync_active = probe.sync_active;
            true
        } else {
            false
        };
        if self.last_system_sample.elapsed() < Duration::from_millis(1_000) {
            return external_changed;
        }
        self.last_system_sample = Instant::now();
        let volume = self.system.volume;
        let volume_muted = self.system.volume_muted;
        let notification_count = self.system.notification_count;
        let notifications_paused = self.system.notifications_paused;
        let sync_active = self.system.sync_active;
        self.system = self.sampler.sample();
        self.system.volume = volume;
        self.system.volume_muted = volume_muted;
        self.system.notification_count = notification_count;
        self.system.notifications_paused = notifications_paused;
        self.system.sync_active = sync_active;
        if let Some(present) = self.keypad_override {
            self.system.keypad_present = present;
        }
        external_changed
    }
}

#[derive(Deserialize, Debug)]
struct KeypadSheet {
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    title: String,
    #[serde(default)]
    notice: Option<String>,
    #[serde(default)]
    keys: Vec<KeypadBinding>,
    #[serde(default)]
    knobs: Vec<KeypadKnob>,
    #[serde(default)]
    options: Option<KeypadOptions>,
}

#[derive(Default, Deserialize, Debug)]
struct KeypadBinding {
    #[serde(default)]
    control: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    bound: bool,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    row: i32,
    #[serde(default)]
    column: i32,
}

#[derive(Deserialize, Debug)]
struct KeypadKnob {
    control: String,
    #[serde(default)]
    row: i32,
    #[serde(default)]
    column: i32,
    #[serde(default)]
    ccw: KeypadBinding,
    #[serde(default)]
    press: KeypadBinding,
    #[serde(default)]
    cw: KeypadBinding,
}

#[derive(Deserialize, Debug)]
struct KeypadOptions {
    #[serde(rename = "clickThrough", default)]
    click_through: Option<bool>,
}

struct Arguments {
    hidden: bool,
    preview: bool,
    version: bool,
    check: bool,
    control_request: Option<Request>,
}

impl Arguments {
    fn parse(arguments: impl Iterator<Item = String>) -> Result<Self, io::Error> {
        let arguments: Vec<_> = arguments.collect();
        let joined = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let mut result = Self {
            hidden: joined.contains(&"--hidden"),
            preview: joined.contains(&"--preview") || joined.contains(&"--side-by-side"),
            version: joined.contains(&"--version"),
            check: joined.contains(&"--check"),
            control_request: None,
        };
        let command = joined.iter().find(|value| !value.starts_with('-')).copied();
        result.control_request = match command {
            None | Some("--resident") => None,
            Some("start") | Some("show") | Some("open") => Some(Request::Show),
            Some("hide") | Some("close") | Some("stop") => Some(Request::Hide),
            Some("toggle") => Some(Request::Toggle),
            Some("apply") | Some("reload") => Some(Request::Apply),
            Some("close-popups") => Some(Request::ClosePopups),
            Some("ensure") => Some(Request::Status),
            Some("redraw") | Some("refresh") => Some(Request::Apply),
            Some("identify") => Some(Request::Identify),
            Some("quit") | Some("kill") => Some(Request::Quit),
            Some("status") => Some(Request::Status),
            Some("set-keypad-present") => {
                let value = joined
                    .get(1)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "set-keypad-present yes|no required",
                        )
                    })?
                    .to_ascii_lowercase();
                let present = match value.as_str() {
                    "yes" | "true" | "on" => true,
                    "no" | "false" | "off" => false,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("invalid keypad presence {value}"),
                        ));
                    }
                };
                Some(Request::SetKeypadPresent { present })
            }
            Some("keypad-sheet") => {
                let source = joined.get(1).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "keypad-sheet FILE|JSON|- required",
                    )
                })?;
                let raw = if *source == "-" {
                    let mut input = String::new();
                    use std::io::Read;
                    std::io::stdin().read_to_string(&mut input)?;
                    input
                } else if Path::new(source).is_file() {
                    std::fs::read_to_string(source)?
                } else {
                    (*source).to_owned()
                };
                let sheet = serde_json::from_str(&raw).map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("invalid keypad sheet JSON: {error}"),
                    )
                })?;
                Some(Request::KeypadSheet { sheet })
            }
            Some("quick-settings") => Some(Request::Popup {
                name: Popup::QuickSettings,
            }),
            Some("usb") => Some(Request::Popup { name: Popup::Usb }),
            Some("workspace-overview") => Some(Request::Popup {
                name: Popup::WorkspaceOverview,
            }),
            Some("popup") => {
                let name = joined.get(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "popup NAME required")
                })?;
                if *name == "--close" {
                    return Ok(Self {
                        control_request: Some(Request::ClosePopups),
                        ..result
                    });
                }
                let popup = match *name {
                    "quick-settings" => Popup::QuickSettings,
                    "usb" => Popup::Usb,
                    "workspace-overview" => Popup::WorkspaceOverview,
                    "keypad-cheatsheet" => Popup::KeypadCheatsheet,
                    _ => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("unknown popup {name}"),
                        ));
                    }
                };
                Some(Request::Popup { name: popup })
            }
            Some(value) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument or command: {value}"),
                ));
            }
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_callbacks_use_the_authoritative_os_coordinator() {
        assert_eq!(PopupTarget::QuickSettings.argument(), "quick-settings");
        assert_eq!(PopupTarget::Usb.argument(), "usb-popup");
        assert_eq!(
            PopupTarget::WorkspaceOverview.argument(),
            "workspace-overview"
        );
        assert_eq!(PopupTarget::Calendar.argument(), "calendar-popup");
        assert_eq!(PopupTarget::Notifications.argument(), "notification-hub");
    }

    #[test]
    fn percentage_probe_does_not_confuse_multiple_values() {
        assert_eq!(first_percentage("Volume: 42% / 42%"), Some(42));
        assert_eq!(first_percentage("muted"), None);
    }

    #[test]
    fn arguments_accept_bar_ctl_and_popup_compatibility() {
        let arguments =
            Arguments::parse(["--resident", "--hidden"].into_iter().map(str::to_owned)).unwrap();
        assert!(arguments.hidden);
        assert!(arguments.control_request.is_none());
        let arguments = Arguments::parse(["popup", "usb"].into_iter().map(str::to_owned)).unwrap();
        assert_eq!(
            arguments.control_request,
            Some(Request::Popup { name: Popup::Usb })
        );
        let arguments =
            Arguments::parse(["set-keypad-present", "yes"].into_iter().map(str::to_owned)).unwrap();
        assert_eq!(
            arguments.control_request,
            Some(Request::SetKeypadPresent { present: true })
        );
    }

    #[test]
    fn keypad_sheet_is_strictly_typed_but_tolerates_unbound_rows() {
        let value = serde_json::json!({
            "visible": true,
            "title": "Editor",
            "keys": [{
                "control": "key1",
                "label": "Save",
                "active": true,
                "bound": true,
                "row": 0,
                "column": 0
            }, {
                "control": "key2",
                "row": 0,
                "column": 1
            }],
            "knobs": [{
                "control": "knob1",
                "row": 0,
                "ccw": {"label": "Down", "bound": true},
                "press": {"label": "Reset", "bound": true},
                "cw": {"label": "Up", "bound": true}
            }]
        });
        let sheet: KeypadSheet = serde_json::from_value(value).unwrap();
        assert_eq!(sheet.keys.len(), 2);
        assert_eq!(sheet.keys[1].control, "key2");
        assert_eq!(sheet.knobs.len(), 1);
    }
}
