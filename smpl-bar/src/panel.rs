use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use i_slint_backend_winit::winit::window::Window;
use serde::Serialize;
use smithay_client_toolkit::reexports::client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
    Connection, QueueHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState, Region},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use smpl_shell::config::{PanelEdge, PanelMode};
use smpl_shell::monitor::Monitor;
use std::io;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Duration;
use x11rb::connection::Connection as X11Connection;
use x11rb::protocol::shape::ConnectionExt as ShapeConnectionExt;
use x11rb::protocol::xproto::ConnectionExt as XprotoConnectionExt;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

const RESERVATION_NAMESPACE: &str = "smpl-bar-reservation";
const PANEL_STATUS_CAPACITY: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelGeometry {
    pub monitor: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub bar_height: u32,
    pub edge: PanelEdge,
    pub mode: PanelMode,
    pub margin: i32,
}

impl PanelGeometry {
    pub fn for_monitor(
        monitor: &Monitor,
        edge: PanelEdge,
        mode: PanelMode,
        margin: usize,
        bar_height: usize,
    ) -> Self {
        let scale = monitor.scale.max(0.5);
        let width = ((monitor.width as f64) / scale).round().max(1.0) as u32;
        let height = ((monitor.height as f64) / scale).round().max(1.0) as u32;
        let margin = margin as i32;
        let (x, y, panel_width, panel_height) = match edge {
            PanelEdge::Top => (
                monitor.x + margin,
                monitor.y + margin,
                width.saturating_sub(2 * margin.max(0) as u32),
                bar_height as u32,
            ),
            PanelEdge::Bottom => (
                monitor.x + margin,
                monitor
                    .y
                    .saturating_add_unsigned(height)
                    .saturating_sub_unsigned(bar_height as u32)
                    .saturating_sub(margin),
                width.saturating_sub(2 * margin.max(0) as u32),
                bar_height as u32,
            ),
            PanelEdge::Left => (
                monitor.x + margin,
                monitor.y + margin,
                bar_height as u32,
                height.saturating_sub(2 * margin.max(0) as u32),
            ),
            PanelEdge::Right => (
                monitor
                    .x
                    .saturating_add_unsigned(width)
                    .saturating_sub_unsigned(bar_height as u32)
                    .saturating_sub(margin),
                monitor.y + margin,
                bar_height as u32,
                height.saturating_sub(2 * margin.max(0) as u32),
            ),
        };
        Self {
            monitor: monitor.connector.clone(),
            x,
            y,
            width: panel_width,
            height: panel_height,
            bar_height: bar_height as u32,
            edge,
            mode,
            margin,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelUpdate {
    pub geometry: PanelGeometry,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PanelStatus {
    pub active: bool,
    pub transport: &'static str,
    pub message: String,
}

impl PanelStatus {
    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            active: false,
            transport: "wayland-layer-shell",
            message: message.into(),
        }
    }
}

pub struct PanelReservation {
    commands: Option<SyncSender<PanelUpdate>>,
    statuses: Receiver<PanelStatus>,
}

impl PanelReservation {
    pub fn spawn() -> Self {
        let (commands_tx, commands_rx) = mpsc::sync_channel(4);
        let (status_tx, status_rx) = mpsc::sync_channel(PANEL_STATUS_CAPACITY);
        let commands = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            let worker_status = status_tx.clone();
            match std::thread::Builder::new()
                .name("panel-layer".to_owned())
                .spawn(move || wayland_panel_loop(commands_rx, worker_status))
            {
                Ok(_) => Some(commands_tx),
                Err(error) => {
                    let _ = status_tx.send(PanelStatus::unavailable(format!(
                        "cannot start reservation worker: {error}"
                    )));
                    None
                }
            }
        } else {
            let _ = status_tx.send(PanelStatus {
                active: false,
                transport: "wayland-layer-shell",
                message: "not running on Wayland".to_owned(),
            });
            None
        };
        Self {
            commands,
            statuses: status_rx,
        }
    }

    pub fn update(&self, update: PanelUpdate) {
        if let Some(commands) = &self.commands {
            let _ = commands.try_send(update);
        }
    }

    pub fn latest_status(&self) -> Option<PanelStatus> {
        let mut latest = None;
        loop {
            match self.statuses.try_recv() {
                Ok(status) => latest = Some(status),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return latest,
            }
        }
    }
}

fn wayland_panel_loop(commands: Receiver<PanelUpdate>, status: SyncSender<PanelStatus>) {
    let mut pending_update = None;
    loop {
        let connection = match Connection::connect_to_env() {
            Ok(connection) => connection,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "Wayland connection unavailable: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let (globals, mut events) = match registry_queue_init(&connection) {
            Ok(values) => values,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "Wayland registry unavailable: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let queue = events.handle();
        let layer_shell = match LayerShell::bind(&globals, &queue) {
            Ok(shell) => shell,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "compositor has no wlr-layer-shell protocol: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let compositor = match CompositorState::bind(&globals, &queue) {
            Ok(compositor) => compositor,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "compositor unavailable: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let shm = match Shm::bind(&globals, &queue) {
            Ok(shm) => shm,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "shared-memory transport unavailable: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let registry_state = RegistryState::new(&globals);
        let output_state = OutputState::new(&globals, &queue);
        let pool = match SlotPool::new(256 * 1024, &shm) {
            Ok(pool) => pool,
            Err(error) => {
                let _ = status.try_send(PanelStatus::unavailable(format!(
                    "cannot allocate reservation buffer: {error}"
                )));
                if recv_retry(&commands, &mut pending_update) {
                    return;
                }
                continue;
            }
        };
        let mut state = WaylandPanelState {
            registry_state,
            output_state,
            compositor,
            layer_shell,
            shm,
            pool,
            update: pending_update.clone(),
            surface: None,
            configured_size: (0, 0),
            status: status.clone(),
            dirty: true,
            last_status: None,
        };
        if events.roundtrip(&mut state).is_err() {
            continue;
        }
        loop {
            match commands.recv_timeout(Duration::from_millis(100)) {
                Ok(update) => {
                    pending_update = Some(update.clone());
                    state.update = Some(update);
                    state.dirty = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if events.roundtrip(&mut state).is_err() {
                break;
            }
            state.reconcile(&queue);
        }
    }
}

fn recv_retry(commands: &Receiver<PanelUpdate>, pending_update: &mut Option<PanelUpdate>) -> bool {
    match commands.recv_timeout(Duration::from_secs(2)) {
        Ok(update) => {
            *pending_update = Some(update);
            false
        }
        Err(mpsc::RecvTimeoutError::Timeout) => false,
        Err(mpsc::RecvTimeoutError::Disconnected) => true,
    }
}

struct WaylandPanelState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    update: Option<PanelUpdate>,
    surface: Option<LayerSurface>,
    configured_size: (u32, u32),
    status: SyncSender<PanelStatus>,
    dirty: bool,
    last_status: Option<PanelStatus>,
}

impl WaylandPanelState {
    fn reconcile(&mut self, queue: &QueueHandle<Self>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(update) = self.update.clone() else {
            return;
        };
        let should_reserve = update.visible && update.geometry.mode == PanelMode::Pinned;
        let target = self.output_state.outputs().find(|output| {
            self.output_state
                .info(output)
                .and_then(|info| info.name)
                .as_deref()
                == Some(update.geometry.monitor.as_str())
        });
        if !should_reserve {
            self.surface = None;
            self.publish(PanelStatus {
                active: false,
                transport: "wayland-layer-shell",
                message: if update.visible {
                    "floating overlay: no exclusive zone".to_owned()
                } else {
                    "hidden".to_owned()
                },
            });
            return;
        }
        let Some(output) = target else {
            self.surface = None;
            self.publish(PanelStatus::unavailable(format!(
                "waiting for output {}",
                update.geometry.monitor
            )));
            return;
        };
        self.surface = None;
        let surface = self.compositor.create_surface(queue);
        let layer = self.layer_shell.create_layer_surface(
            queue,
            surface,
            Layer::Top,
            Some(RESERVATION_NAMESPACE),
            Some(&output),
        );
        let anchor = match update.geometry.edge {
            PanelEdge::Top => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            PanelEdge::Bottom => Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            PanelEdge::Left => Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
            PanelEdge::Right => Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
        };
        layer.set_anchor(anchor);
        let margin = update.geometry.margin.max(0);
        match update.geometry.edge {
            PanelEdge::Top => layer.set_margin(margin, 0, 0, 0),
            PanelEdge::Bottom => layer.set_margin(0, 0, margin, 0),
            PanelEdge::Left => layer.set_margin(0, 0, 0, margin),
            PanelEdge::Right => layer.set_margin(0, margin, 0, 0),
        }
        layer.set_exclusive_zone(
            i32::try_from(
                update
                    .geometry
                    .bar_height
                    .saturating_add(margin.max(0) as u32),
            )
            .unwrap_or(i32::MAX),
        );
        layer.set_size(
            if matches!(update.geometry.edge, PanelEdge::Left | PanelEdge::Right) {
                update.geometry.bar_height
            } else {
                0
            },
            if matches!(update.geometry.edge, PanelEdge::Top | PanelEdge::Bottom) {
                update.geometry.bar_height
            } else {
                0
            },
        );
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        if let Ok(region) = Region::new(&self.compositor) {
            layer.set_input_region(Some(region.wl_region()));
        }
        layer.commit();
        self.surface = Some(layer);
        self.configured_size = (update.geometry.width, update.geometry.bar_height);
        self.publish(PanelStatus::unavailable(format!(
            "waiting for {} edge layer configuration on {}",
            edge_name(update.geometry.edge),
            update.geometry.monitor
        )));
    }

    fn output_changed(&mut self) {
        self.dirty = true;
    }

    fn draw(&mut self) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        let (width, height) = self.configured_size;
        if width == 0 || height == 0 {
            return false;
        }
        let stride = i32::try_from(width.saturating_mul(4)).unwrap_or(i32::MAX);
        let (buffer, canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(values) => values,
            Err(error) => {
                self.publish(PanelStatus::unavailable(format!(
                    "cannot create transparent reservation buffer: {error}"
                )));
                return false;
            }
        };
        canvas.fill(0);
        surface
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        if buffer.attach_to(surface.wl_surface()).is_ok() {
            surface.commit();
            true
        } else {
            false
        }
    }

    fn publish(&mut self, status: PanelStatus) {
        if self.last_status.as_ref() != Some(&status) {
            self.last_status = Some(status.clone());
            let _ = self.status.try_send(status);
        }
    }
}

fn edge_name(edge: PanelEdge) -> &'static str {
    match edge {
        PanelEdge::Top => "top",
        PanelEdge::Bottom => "bottom",
        PanelEdge::Left => "left",
        PanelEdge::Right => "right",
    }
}

impl CompositorHandler for WaylandPanelState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for WaylandPanelState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.output_changed();
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.output_changed();
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        self.output_changed();
    }
}

impl ShmHandler for WaylandPanelState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl LayerShellHandler for WaylandPanelState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.surface = None;
        self.publish(PanelStatus::unavailable(
            "layer reservation closed by compositor",
        ));
        self.dirty = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (width, height) = configure.new_size;
        if width > 0 && height > 0 {
            self.configured_size = (width, height);
            if self.draw() {
                if let Some(update) = self.update.clone() {
                    self.publish(PanelStatus {
                        active: true,
                        transport: "wayland-layer-shell",
                        message: format!(
                            "exclusive {} edge on {}",
                            edge_name(update.geometry.edge),
                            update.geometry.monitor
                        ),
                    });
                }
            }
        }
    }
}

impl ProvidesRegistryState for WaylandPanelState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

delegate_compositor!(WaylandPanelState);
delegate_output!(WaylandPanelState);
delegate_shm!(WaylandPanelState);
delegate_layer!(WaylandPanelState);
delegate_registry!(WaylandPanelState);

pub struct X11Panel {
    connection: x11rb::rust_connection::RustConnection,
    root: u32,
    last_window: Option<u32>,
}

impl X11Panel {
    pub fn connect() -> Option<Self> {
        let (connection, screen_number) =
            x11rb::rust_connection::RustConnection::connect(None).ok()?;
        let root = connection.setup().roots.get(screen_number)?.root;
        Some(Self {
            connection,
            root,
            last_window: None,
        })
    }

    pub fn apply(
        &mut self,
        window: &Window,
        geometry: &PanelGeometry,
        visible: bool,
    ) -> io::Result<()> {
        let Some(xid) = x11_window(window) else {
            return Err(io::Error::other("native window is not X11"));
        };
        self.last_window = Some(xid);
        self.set_window_contract(xid, geometry, visible)?;
        self.connection
            .flush()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn hide(&mut self) -> io::Result<()> {
        let Some(xid) = self.last_window else {
            return Ok(());
        };
        self.clear_strut(xid)?;
        self.connection
            .flush()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    pub fn set_input_passthrough(
        &mut self,
        window: &Window,
        enabled: bool,
        width: u32,
        height: u32,
    ) -> io::Result<()> {
        use x11rb::protocol::shape::{SK, SO};
        use x11rb::protocol::xproto::{ClipOrdering, Rectangle};

        let Some(xid) = x11_window(window) else {
            return Err(io::Error::other("native window is not X11"));
        };
        let rectangle = (!enabled).then_some(Rectangle {
            x: 0,
            y: 0,
            width: u16::try_from(width).unwrap_or(u16::MAX),
            height: u16::try_from(height).unwrap_or(u16::MAX),
        });
        self.connection
            .shape_rectangles(
                SO::SET,
                SK::INPUT,
                ClipOrdering::UNSORTED,
                xid,
                0,
                0,
                rectangle.as_slice(),
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.connection
            .flush()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn set_window_contract(
        &self,
        window: u32,
        geometry: &PanelGeometry,
        visible: bool,
    ) -> io::Result<()> {
        use x11rb::protocol::xproto::{AtomEnum, PropMode};

        let dock = self.atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
        let normal = self.atom(b"_NET_WM_WINDOW_TYPE_NORMAL")?;
        let sticky = self.atom(b"_NET_WM_STATE_STICKY")?;
        let above = self.atom(b"_NET_WM_STATE_ABOVE")?;
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_WINDOW_TYPE")?,
                AtomEnum::ATOM,
                &[if geometry.mode == PanelMode::Pinned {
                    dock
                } else {
                    normal
                }],
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))?;
        let states = if geometry.mode == PanelMode::Pinned {
            vec![sticky, above]
        } else {
            vec![]
        };
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_STATE")?,
                AtomEnum::ATOM,
                &states,
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))?;
        if visible && geometry.mode == PanelMode::Pinned {
            self.set_strut(window, geometry)
        } else {
            self.clear_strut(window)
        }
    }

    fn set_strut(&self, window: u32, geometry: &PanelGeometry) -> io::Result<()> {
        use x11rb::protocol::xproto::{AtomEnum, PropMode};

        let screen = self
            .connection
            .setup()
            .roots
            .iter()
            .find(|screen| screen.root == self.root)
            .ok_or_else(|| io::Error::other("X11 root screen disappeared"))?;
        let (partial, strut) = strut_values(
            i32::from(screen.width_in_pixels),
            i32::from(screen.height_in_pixels),
            geometry,
        );
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_STRUT_PARTIAL")?,
                AtomEnum::CARDINAL,
                &partial,
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_STRUT")?,
                AtomEnum::CARDINAL,
                &strut,
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn clear_strut(&self, window: u32) -> io::Result<()> {
        use x11rb::protocol::xproto::{AtomEnum, PropMode};

        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_STRUT")?,
                AtomEnum::CARDINAL,
                &[0, 0, 0, 0],
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))?;
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.atom(b"_NET_WM_STRUT_PARTIAL")?,
                AtomEnum::CARDINAL,
                &[0; 12],
            )
            .map_err(|error| io::Error::other(error.to_string()))?
            .check()
            .map_err(|error| io::Error::other(error.to_string()))
    }

    fn atom(&self, name: &[u8]) -> io::Result<u32> {
        self.connection
            .intern_atom(false, name)
            .map_err(|error| io::Error::other(error.to_string()))?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

fn strut_values(
    screen_width: i32,
    screen_height: i32,
    geometry: &PanelGeometry,
) -> ([u32; 12], [u32; 4]) {
    let start_x = geometry.x.max(0) as u32;
    let end_x = geometry
        .x
        .saturating_add_unsigned(geometry.width.saturating_sub(1))
        .max(geometry.x)
        .max(0) as u32;
    let start_y = geometry.y.max(0) as u32;
    let end_y = geometry
        .y
        .saturating_add_unsigned(geometry.height.saturating_sub(1))
        .max(geometry.y)
        .max(0) as u32;
    let strut = match geometry.edge {
        PanelEdge::Top => [
            0,
            0,
            (geometry.y + geometry.bar_height as i32).max(0) as u32,
            0,
        ],
        PanelEdge::Bottom => [
            0,
            0,
            0,
            screen_height
                .saturating_sub(geometry.y)
                .max(geometry.bar_height as i32)
                .max(0) as u32,
        ],
        PanelEdge::Left => [
            (geometry.x + geometry.bar_height as i32).max(0) as u32,
            0,
            0,
            0,
        ],
        PanelEdge::Right => [
            0,
            screen_width
                .saturating_sub(geometry.x)
                .max(geometry.bar_height as i32)
                .max(0) as u32,
            0,
            0,
        ],
    };
    let mut partial = [
        strut[0], strut[1], strut[2], strut[3], 0, 0, 0, 0, 0, 0, 0, 0,
    ];
    match geometry.edge {
        PanelEdge::Left => {
            partial[4] = start_y;
            partial[5] = end_y;
        }
        PanelEdge::Right => {
            partial[6] = start_y;
            partial[7] = end_y;
        }
        PanelEdge::Top => {
            partial[8] = start_x;
            partial[9] = end_x;
        }
        PanelEdge::Bottom => {
            partial[10] = start_x;
            partial[11] = end_x;
        }
    }
    (partial, strut)
}

fn x11_window(window: &Window) -> Option<u32> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(handle) => u32::try_from(handle.window).ok(),
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_anchors_top_and_bottom_on_the_selected_monitor() {
        let monitor = Monitor {
            connector: "DP-2".to_owned(),
            x: 1920,
            y: 100,
            width: 2560,
            height: 1440,
            scale: 2.0,
            ..Monitor::default()
        };
        let bottom =
            PanelGeometry::for_monitor(&monitor, PanelEdge::Bottom, PanelMode::Pinned, 2, 32);
        assert_eq!((bottom.width, bottom.x, bottom.y), (1276, 1922, 786));
        let top = PanelGeometry::for_monitor(&monitor, PanelEdge::Top, PanelMode::Floating, 4, 36);
        assert_eq!((top.width, top.x, top.y), (1272, 1924, 104));
        let left = PanelGeometry::for_monitor(&monitor, PanelEdge::Left, PanelMode::Pinned, 3, 40);
        assert_eq!(
            (left.width, left.height, left.x, left.y),
            (40, 714, 1923, 103)
        );
        let right =
            PanelGeometry::for_monitor(&monitor, PanelEdge::Right, PanelMode::Floating, 2, 32);
        assert_eq!(
            (right.width, right.height, right.x, right.y),
            (32, 716, 3166, 102)
        );
    }

    #[test]
    fn x11_struts_reserve_each_supported_edge_and_range() {
        let monitor = Monitor {
            connector: "DP-1".to_owned(),
            x: 100,
            y: 50,
            width: 1600,
            height: 900,
            scale: 1.0,
            ..Monitor::default()
        };
        for (edge, expected_strut, range) in [
            (PanelEdge::Left, [134, 0, 0, 0], (4, 5, 52, 947)),
            (PanelEdge::Right, [0, 334, 0, 0], (6, 7, 52, 947)),
            (PanelEdge::Top, [0, 0, 84, 0], (8, 9, 102, 1697)),
            (PanelEdge::Bottom, [0, 0, 0, 84], (10, 11, 102, 1697)),
        ] {
            let geometry = PanelGeometry::for_monitor(&monitor, edge, PanelMode::Pinned, 2, 32);
            let (partial, strut) = strut_values(2000, 1000, &geometry);
            assert_eq!(strut, expected_strut);
            assert_eq!(partial[range.0], range.2);
            assert_eq!(partial[range.1], range.3);
        }
    }
}
