use crate::config::{BarConfigFile, PanelEdge, PanelMode};
use crate::launch;
use crate::monitor::{
    layout_code, CompositorKind, CompositorState, KeyboardState, Monitor, Workspace,
};
use crate::telemetry::Telemetry;
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_REPLY: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SelectWorkspace(i32),
    FocusMonitor(String),
    NextKeyboardLayout,
    PlacePanel {
        monitor: String,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        edge: PanelEdge,
        mode: PanelMode,
    },
    PlacePopup {
        title: String,
        monitor: String,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Quit,
}

pub struct CompositorHandle {
    pub kind: CompositorKind,
    pub states: Receiver<CompositorState>,
    actions: SyncSender<Action>,
    panel_ack: Arc<Mutex<Option<PanelPlacementTarget>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelPlacementTarget {
    pub monitor: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl PanelPlacementTarget {
    fn from_action(action: &Action) -> Option<Self> {
        let Action::PlacePanel {
            monitor,
            x,
            y,
            width,
            height,
            ..
        } = action
        else {
            return None;
        };
        Some(Self {
            monitor: monitor.clone(),
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        })
    }
}

impl CompositorHandle {
    pub fn send(&self, action: Action) {
        if let Err(TrySendError::Disconnected(_)) = self.actions.try_send(action) {
            eprintln!("smpl-bar: compositor worker stopped");
        }
    }

    pub fn latest(&self) -> Option<CompositorState> {
        let mut latest = None;
        loop {
            match self.states.try_recv() {
                Ok(state) => latest = Some(state),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return latest,
            }
        }
    }

    pub fn panel_acknowledged(&self, target: &PanelPlacementTarget) -> bool {
        self.panel_ack
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            == Some(target)
    }
}

pub fn detect() -> CompositorKind {
    if std::env::var_os("NIRI_SOCKET").is_some() {
        CompositorKind::Niri
    } else if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        CompositorKind::Hyprland
    } else if std::env::var_os("DISPLAY").is_some() {
        CompositorKind::X11
    } else {
        CompositorKind::Unavailable
    }
}

#[derive(Deserialize, Default)]
struct WorkspacePolicy {
    #[serde(default)]
    enabled: bool,
    primary: Option<String>,
    #[serde(default)]
    monitors: Vec<WorkspacePolicyMonitor>,
}

#[derive(Deserialize, Default)]
struct WorkspacePolicyMonitor {
    id: String,
    connector: String,
}

fn policy_primary() -> String {
    let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    else {
        return String::new();
    };
    let directory = home.join(".config/smplos");
    if !directory.join("workspace-policy.enabled").is_file() {
        return String::new();
    }
    let Ok(source) = std::fs::read_to_string(directory.join("workspace-policy.json")) else {
        return String::new();
    };
    let Ok(policy) = serde_json::from_str::<WorkspacePolicy>(&source) else {
        return String::new();
    };
    if !policy.enabled {
        return String::new();
    }
    let Some(primary) = policy.primary else {
        return String::new();
    };
    policy
        .monitors
        .into_iter()
        .find(|monitor| monitor.id == primary)
        .map(|monitor| monitor.connector)
        .unwrap_or_default()
}

pub fn spawn(_config: BarConfigFile, telemetry: Telemetry) -> CompositorHandle {
    let kind = detect();
    let (state_tx, states) = mpsc::sync_channel(4);
    let (actions, action_rx) = mpsc::sync_channel(32);
    let panel_ack = Arc::new(Mutex::new(None));
    let worker_panel_ack = panel_ack.clone();
    let thread_telemetry = telemetry.clone();
    let thread_kind = kind;
    let _ = std::thread::Builder::new()
        .name("compositor".to_owned())
        .spawn(move || match thread_kind {
            CompositorKind::Hyprland => {
                HyprlandWorker::new(thread_telemetry).run(state_tx, action_rx, worker_panel_ack)
            }
            CompositorKind::Niri => {
                NiriWorker::new(thread_telemetry).run(state_tx, action_rx, worker_panel_ack)
            }
            CompositorKind::X11 => {
                X11Worker::new(thread_telemetry).run(state_tx, action_rx, worker_panel_ack)
            }
            CompositorKind::Unavailable => {
                let state = CompositorState {
                    error: "No supported compositor session was found".to_owned(),
                    ..CompositorState::default()
                };
                let _ = state_tx.send(state);
                while action_rx.recv().is_ok() {}
            }
        });
    CompositorHandle {
        kind,
        states,
        actions,
        panel_ack,
    }
}

fn publish(tx: &SyncSender<CompositorState>, state: CompositorState) {
    let _ = tx.try_send(state);
}

fn error_state(kind: CompositorKind, error: impl ToString) -> CompositorState {
    CompositorState {
        kind,
        error: error.to_string(),
        ..CompositorState::default()
    }
}

struct HyprlandWorker {
    directory: PathBuf,
    telemetry: Telemetry,
}

impl HyprlandWorker {
    fn new(telemetry: Telemetry) -> Self {
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));
        let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
        Self {
            directory: runtime.join("hypr").join(signature),
            telemetry,
        }
    }

    fn run(
        self,
        state_tx: SyncSender<CompositorState>,
        actions: Receiver<Action>,
        panel_ack: Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) {
        self.telemetry.emit(
            "compositor_detected",
            [("kind", serde_json::json!("hyprland"))],
        );
        let mut failure = String::new();
        loop {
            let snapshot = self.snapshot();
            match snapshot {
                Ok(state) => {
                    failure.clear();
                    publish(&state_tx, state);
                    self.telemetry.emit(
                        "compositor_snapshot",
                        [("source", serde_json::json!("hyprland-ipc"))],
                    );
                }
                Err(error) => {
                    let message = error.to_string();
                    if failure != message {
                        publish(&state_tx, error_state(CompositorKind::Hyprland, &message));
                        failure = message;
                    }
                }
            }

            let event_path = self.directory.join(".socket2.sock");
            let stream = match UnixStream::connect(&event_path) {
                Ok(stream) => stream,
                Err(error) => {
                    self.telemetry.emit(
                        "compositor_subscribe",
                        [
                            ("kind", serde_json::json!("hyprland")),
                            ("ok", serde_json::json!(false)),
                            ("error", serde_json::json!(error.to_string())),
                        ],
                    );
                    if self.wait_for_action(&actions, &panel_ack) {
                        return;
                    }
                    continue;
                }
            };
            self.telemetry.emit(
                "compositor_subscribe",
                [
                    ("kind", serde_json::json!("hyprland")),
                    ("ok", serde_json::json!(true)),
                ],
            );
            let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
            let mut reader = BufReader::new(stream);
            let mut buffer = Vec::new();
            loop {
                if self.drain_actions(&actions, &panel_ack) {
                    return;
                }
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer) {
                    Ok(0) => break,
                    Ok(_) => {
                        let event = String::from_utf8_lossy(&buffer);
                        if event_needs_snapshot(&event) {
                            match self.snapshot() {
                                Ok(state) => publish(&state_tx, state),
                                Err(error) => {
                                    publish(&state_tx, error_state(CompositorKind::Hyprland, error))
                                }
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) => {}
                    Err(_) => break,
                }
            }
        }
    }

    fn wait_for_action(
        &self,
        actions: &Receiver<Action>,
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) -> bool {
        match actions.recv_timeout(Duration::from_millis(500)) {
            Ok(Action::Quit) => true,
            Ok(action) => {
                self.handle_action(action, panel_ack);
                false
            }
            Err(_) => false,
        }
    }

    fn drain_actions(
        &self,
        actions: &Receiver<Action>,
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) -> bool {
        loop {
            match actions.try_recv() {
                Ok(Action::Quit) => return true,
                Ok(action) => self.handle_action(action, panel_ack),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn handle_action(
        &self,
        action: Action,
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) {
        let panel_target = PanelPlacementTarget::from_action(&action);
        let started = std::time::Instant::now();
        let result = match action {
            Action::SelectWorkspace(workspace) => self.select_workspace(workspace),
            Action::FocusMonitor(connector) => {
                self.dispatch(format!("hl.dsp.focus({{monitor='{connector}'}})"))
            }
            Action::NextKeyboardLayout => self
                .request("switchxkblayout current next")
                .map(|_| ())
                .map_err(WorkerError::from),
            Action::PlacePanel {
                monitor,
                x,
                y,
                width: _,
                height: _,
                edge: _,
                mode: _,
            } => self.place_panel(&monitor, x, y),
            Action::PlacePopup {
                title,
                monitor,
                x,
                y,
                width: _,
                height: _,
            } => self.place_window(&title, &monitor, x, y),
            Action::Quit => Ok(()),
        };
        self.telemetry.emit(
            "compositor_action",
            [
                (
                    "duration_us",
                    serde_json::json!(started.elapsed().as_micros()),
                ),
                ("ok", serde_json::json!(result.is_ok())),
                (
                    "error",
                    result
                        .as_ref()
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_default()
                        .into(),
                ),
            ],
        );
        if result.is_ok() {
            *panel_ack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = panel_target;
        }
    }

    fn request(&self, command: &str) -> std::io::Result<Vec<u8>> {
        let mut stream = UnixStream::connect(self.directory.join(".socket.sock"))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        stream.write_all(command.as_bytes())?;
        stream.shutdown(std::net::Shutdown::Write)?;
        let mut reply = Vec::new();
        stream
            .take((MAX_REPLY + 1) as u64)
            .read_to_end(&mut reply)?;
        if reply.len() > MAX_REPLY {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Hyprland IPC response exceeded 8 MiB",
            ));
        }
        Ok(reply)
    }

    fn json<T: for<'de> Deserialize<'de>>(&self, command: &str) -> Result<T, WorkerError> {
        let reply = self.request(command)?;
        Ok(serde_json::from_slice(&reply)?)
    }

    fn dispatch(&self, expression: String) -> Result<(), WorkerError> {
        let reply = self.request(&format!("dispatch {expression}"))?;
        if String::from_utf8_lossy(&reply).trim() != "ok" {
            return Err(WorkerError::Message(format!(
                "Hyprland rejected action: {}",
                String::from_utf8_lossy(&reply).trim()
            )));
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<CompositorState, WorkerError> {
        let raw_monitors: Vec<HyprMonitor> = self.json("j/monitors")?;
        let raw_workspaces: Vec<HyprWorkspace> = self.json("j/workspaces")?;
        let active: Option<HyprWindow> = self.json("j/activewindow").ok();
        let devices: HyprDevices = self.json("j/devices")?;
        let layouts: Option<HyprOption> = self.json("j/getoption input:kb_layout").ok();
        let variants: Option<HyprOption> = self.json("j/getoption input:kb_variant").ok();

        let monitors = raw_monitors
            .into_iter()
            .map(|monitor| Monitor {
                id: monitor.id.to_string(),
                connector: monitor.name,
                description: monitor.description.unwrap_or_default(),
                x: monitor.x,
                y: monitor.y,
                width: monitor.width,
                height: monitor.height,
                scale: monitor.scale,
                focused: monitor.focused,
                active_workspace: monitor
                    .active_workspace
                    .map(|workspace| workspace.id)
                    .unwrap_or(0),
            })
            .collect::<Vec<_>>();
        let visible: HashSet<_> = monitors
            .iter()
            .map(|monitor| monitor.active_workspace)
            .collect();
        let active_workspace = active
            .as_ref()
            .and_then(|window| window.workspace.as_ref())
            .map(|workspace| workspace.id)
            .or_else(|| {
                monitors
                    .iter()
                    .find(|monitor| monitor.focused)
                    .map(|monitor| monitor.active_workspace)
            })
            .unwrap_or(1);
        let active_monitor = active
            .as_ref()
            .map(|window| window.monitor.to_string())
            .and_then(|id| monitors.iter().find(|monitor| monitor.id == id))
            .or_else(|| monitors.iter().find(|monitor| monitor.focused))
            .map(|monitor| monitor.connector.clone())
            .unwrap_or_default();
        let workspaces = raw_workspaces
            .into_iter()
            .filter(|workspace| workspace.id > 0)
            .map(|workspace| Workspace {
                occupied: workspace.windows > 0,
                visible: visible.contains(&workspace.id),
                focused: workspace.id == active_workspace,
                monitor: workspace.monitor.unwrap_or_default(),
                id: workspace.id,
                windows: workspace.windows,
                temporary: false,
            })
            .collect();
        let keyboard = devices
            .keyboards
            .into_iter()
            .find(|keyboard| {
                !keyboard.name.to_ascii_lowercase().contains("power")
                    && !keyboard.name.to_ascii_lowercase().contains("button")
            })
            .map(|keyboard| {
                let layouts = split_option(&layouts.map(|option| option.str).unwrap_or_default());
                let variants = split_option(&variants.map(|option| option.str).unwrap_or_default());
                let index = keyboard.active_layout_index.unwrap_or(0) as usize;
                KeyboardState {
                    visible: layouts.len() > 1 && keyboard.active_keymap.is_some(),
                    code: layout_code(keyboard.active_keymap.as_deref().unwrap_or("")),
                    layout: keyboard.active_keymap.unwrap_or_default(),
                    xkb_layout: layouts
                        .get(index)
                        .or_else(|| layouts.first())
                        .cloned()
                        .unwrap_or_default(),
                    xkb_variant: variants.get(index).cloned().unwrap_or_default(),
                }
            })
            .unwrap_or_default();
        Ok(CompositorState {
            kind: CompositorKind::Hyprland,
            monitors,
            workspaces,
            policy_primary: policy_primary(),
            focused_workspace: active_workspace,
            focused_monitor: active_monitor,
            active_window_title: active.and_then(|window| window.title).unwrap_or_default(),
            keyboard,
            error: String::new(),
        })
    }

    fn select_workspace(&self, workspace: i32) -> Result<(), WorkerError> {
        let state = self.snapshot()?;
        let target = state
            .workspaces
            .iter()
            .find(|candidate| candidate.id == workspace);
        let focused = state.primary_monitor();
        if target.is_some_and(|candidate| candidate.visible)
            && target
                .is_some_and(|candidate| candidate.monitor != focused.map_or("", |m| &m.connector))
        {
            if let Some(other) = focused {
                if let Some(target) = target {
                    return self.dispatch(format!(
                        "hl.dsp.workspace.swap_monitors({{monitor1='{}',monitor2='{}'}})",
                        other.connector, target.monitor
                    ));
                }
            }
        }
        self.dispatch(format!("hl.dsp.focus({{workspace={workspace}}})"))
    }

    fn place_panel(&self, monitor: &str, x: i32, y: i32) -> Result<(), WorkerError> {
        self.place_matching_window(monitor, x, y, |client| {
            client.pid == i64::from(std::process::id())
                && client.class.as_deref() == Some("smpl-bar")
                && client.title.as_deref().is_some_and(|title| {
                    title == "smpl-bar" || title.starts_with("smpl-bar:panel-")
                })
        })
    }

    fn place_window(&self, title: &str, monitor: &str, x: i32, y: i32) -> Result<(), WorkerError> {
        self.place_matching_window(monitor, x, y, |client| {
            client.pid == i64::from(std::process::id())
                && client.class.as_deref() == Some("smpl-bar")
                && client.title.as_deref() == Some(title)
        })
    }

    fn place_matching_window(
        &self,
        monitor: &str,
        x: i32,
        y: i32,
        matches: impl Fn(&HyprPanelClient) -> bool,
    ) -> Result<(), WorkerError> {
        let mut client = None;
        for _ in 0..4 {
            let clients: Vec<HyprPanelClient> = self.json("j/clients")?;
            client = clients.into_iter().find(|candidate| matches(candidate));
            if client.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let client = client.ok_or_else(|| {
            WorkerError::Message("native window is not mapped for placement".to_owned())
        })?;
        let target_monitor = self
            .json::<Vec<HyprMonitor>>("j/monitors")?
            .into_iter()
            .find(|candidate| candidate.name == monitor);
        if let Some(target) = target_monitor {
            if client.monitor != Some(target.id) {
                self.dispatch(format!(
                    "hl.dsp.window.move({{window=hl.get_window('address:{}'),monitor={}}})",
                    client.address, target.id
                ))?;
            }
        }
        let reply = self.request(&format!(
            "dispatch movewindowpixel exact {x} {y},address:{}",
            client.address
        ))?;
        if String::from_utf8_lossy(&reply).trim() != "ok" {
            return Err(WorkerError::Message(format!(
                "Hyprland rejected native window placement: {}",
                String::from_utf8_lossy(&reply).trim()
            )));
        }
        Ok(())
    }
}

fn event_needs_snapshot(event: &str) -> bool {
    let name = event.split_once(">>").map_or(event, |(name, _)| name);
    matches!(
        name,
        "workspace"
            | "workspacev2"
            | "focusedmon"
            | "activewindow"
            | "activewindowv2"
            | "openwindow"
            | "closewindow"
            | "movewindow"
            | "movewindowv2"
            | "moveworkspace"
            | "moveworkspacev2"
            | "monitoradded"
            | "monitoraddedv2"
            | "monitorremoved"
            | "monitorremovedv2"
            | "configreloaded"
            | "fullscreen"
            | "activelayout"
    ) || name.starts_with("createworkspace")
        || name.starts_with("destroyworkspace")
}

struct NiriWorker {
    socket: PathBuf,
    telemetry: Telemetry,
}

impl NiriWorker {
    fn new(telemetry: Telemetry) -> Self {
        Self {
            socket: std::env::var_os("NIRI_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_default(),
            telemetry,
        }
    }

    fn run(
        self,
        state_tx: SyncSender<CompositorState>,
        actions: Receiver<Action>,
        panel_ack: Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) {
        self.telemetry
            .emit("compositor_detected", [("kind", serde_json::json!("niri"))]);
        let mut outputs = self.outputs().unwrap_or_default();
        let mut live = NiriLiveState::default();
        loop {
            let mut stream = match UnixStream::connect(&self.socket) {
                Ok(stream) => stream,
                Err(error) => {
                    publish(
                        &state_tx,
                        error_state(
                            CompositorKind::Niri,
                            format!("Niri IPC unavailable: {error}"),
                        ),
                    );
                    self.telemetry.emit(
                        "compositor_subscribe",
                        [
                            ("kind", serde_json::json!("niri")),
                            ("transport", serde_json::json!("niri-socket")),
                            ("ok", serde_json::json!(false)),
                        ],
                    );
                    if self.wait_for_action(&actions, &mut live, &outputs, &panel_ack) {
                        return;
                    }
                    continue;
                }
            };
            let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            if stream.write_all(b"\"EventStream\"\n").is_err() {
                if self.wait_for_action(&actions, &mut live, &outputs, &panel_ack) {
                    return;
                }
                continue;
            }
            self.telemetry.emit(
                "compositor_subscribe",
                [
                    ("kind", serde_json::json!("niri")),
                    ("transport", serde_json::json!("niri-socket")),
                    ("ok", serde_json::json!(true)),
                ],
            );
            let mut reader = BufReader::new(stream);
            loop {
                if self.drain_actions(&actions, &mut live, &outputs, &panel_ack) {
                    return;
                }
                let mut line = Vec::new();
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        let Ok(event) = serde_json::from_slice::<serde_json::Value>(&line) else {
                            continue;
                        };
                        if event.get("Ok").is_some() || event.get("Err").is_some() {
                            continue;
                        }
                        let applied = live.apply_event(&event);
                        if applied.refresh_outputs {
                            outputs = self.outputs().unwrap_or(outputs);
                        }
                        if applied.changed {
                            publish(&state_tx, live.snapshot(outputs.clone()));
                            self.telemetry.emit(
                                "compositor_snapshot",
                                [("source", serde_json::json!("niri-socket"))],
                            );
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(_) => break,
                }
            }
        }
    }

    fn wait_for_action(
        &self,
        actions: &Receiver<Action>,
        live: &mut NiriLiveState,
        outputs: &[NiriOutput],
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) -> bool {
        match actions.recv_timeout(Duration::from_millis(250)) {
            Ok(Action::Quit) => true,
            Ok(action) => {
                let _ = self.handle_action(action, live, outputs, panel_ack);
                false
            }
            Err(_) => false,
        }
    }

    fn drain_actions(
        &self,
        actions: &Receiver<Action>,
        live: &mut NiriLiveState,
        outputs: &[NiriOutput],
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) -> bool {
        loop {
            match actions.try_recv() {
                Ok(Action::Quit) => return true,
                Ok(action) => {
                    let _ = self.handle_action(action, live, outputs, panel_ack);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn handle_action(
        &self,
        action: Action,
        live: &NiriLiveState,
        outputs: &[NiriOutput],
        panel_ack: &Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) -> Result<(), WorkerError> {
        let panel_target = PanelPlacementTarget::from_action(&action);
        let result = match action {
            Action::SelectWorkspace(index) => self.request(serde_json::json!({
                "Action": {
                    "FocusWorkspace": {
                        "reference": {"Index": index}
                    }
                }
            })),
            Action::FocusMonitor(output) => self.request(serde_json::json!({
                "Action": {"FocusMonitor": {"output": output}}
            })),
            Action::NextKeyboardLayout => self.request(serde_json::json!({
                "Action": {"SwitchLayout": {"layout": "Next"}}
            })),
            Action::PlacePanel {
                monitor,
                x,
                y,
                width: _,
                height,
                edge: _,
                mode: _,
            } => self.place_panel(
                live,
                outputs,
                NiriPlacement {
                    monitor: &monitor,
                    x,
                    y,
                    height,
                },
            ),
            Action::PlacePopup {
                title,
                monitor,
                x,
                y,
                width: _,
                height,
            } => self.place_popup(
                live,
                outputs,
                &title,
                NiriPlacement {
                    monitor: &monitor,
                    x,
                    y,
                    height,
                },
            ),
            Action::Quit => Ok(serde_json::Value::Null),
        };
        let result = result.map(|_| ());
        self.telemetry.emit(
            "compositor_action",
            [
                ("kind", serde_json::json!("niri")),
                ("ok", serde_json::json!(result.is_ok())),
            ],
        );
        if result.is_ok() {
            *panel_ack
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = panel_target;
        }
        result
    }

    fn outputs(&self) -> Result<Vec<NiriOutput>, WorkerError> {
        let response = self.request(serde_json::Value::String("Outputs".to_owned()))?;
        let outputs = response
            .get("Ok")
            .and_then(|ok| ok.get("Outputs"))
            .cloned()
            .ok_or_else(|| WorkerError::Message("Niri Outputs reply was malformed".to_owned()))?;
        Ok(serde_json::from_value(outputs)?)
    }

    fn request(&self, request: serde_json::Value) -> Result<serde_json::Value, WorkerError> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        serde_json::to_writer(&mut stream, &request)?;
        stream.write_all(b"\n")?;
        let mut reply = Vec::new();
        BufReader::new(&stream)
            .take((MAX_REPLY + 1) as u64)
            .read_until(b'\n', &mut reply)?;
        if reply.len() > MAX_REPLY {
            return Err(WorkerError::Message(
                "Niri IPC response exceeded 8 MiB".to_owned(),
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&reply)?;
        if let Some(error) = value.get("Err") {
            return Err(WorkerError::Message(format!(
                "Niri rejected action: {error}"
            )));
        }
        Ok(value)
    }

    fn place_panel(
        &self,
        live: &NiriLiveState,
        outputs: &[NiriOutput],
        placement: NiriPlacement<'_>,
    ) -> Result<serde_json::Value, WorkerError> {
        self.place_matching_window(live, outputs, placement, |window| {
            window.pid == Some(std::process::id() as i32)
                && window.app_id.as_deref() == Some("smpl-bar")
                && window.title.as_deref().is_some_and(|title| {
                    title == "smpl-bar" || title.starts_with("smpl-bar:panel-")
                })
        })
    }

    fn place_popup(
        &self,
        live: &NiriLiveState,
        outputs: &[NiriOutput],
        title: &str,
        placement: NiriPlacement<'_>,
    ) -> Result<serde_json::Value, WorkerError> {
        self.place_matching_window(live, outputs, placement, |window| {
            window.pid == Some(std::process::id() as i32)
                && window.app_id.as_deref() == Some("smpl-bar")
                && window.title.as_deref() == Some(title)
        })
    }

    fn place_matching_window(
        &self,
        live: &NiriLiveState,
        outputs: &[NiriOutput],
        placement: NiriPlacement<'_>,
        matches: impl Fn(&NiriWindow) -> bool,
    ) -> Result<serde_json::Value, WorkerError> {
        let mut windows = self.windows().unwrap_or_else(|_| live.windows.clone());
        let mut window = windows.into_iter().find(|window| matches(window));
        if window.is_none() {
            std::thread::sleep(Duration::from_millis(75));
            windows = self.windows().unwrap_or_else(|_| live.windows.clone());
            window = windows.into_iter().find(|window| matches(window));
        }
        let window = window.ok_or_else(|| {
            WorkerError::Message("Niri native window is not mapped for placement".to_owned())
        })?;
        let output = outputs
            .iter()
            .find(|output| output.name == placement.monitor)
            .and_then(|output| output.logical.as_ref());
        self.request(serde_json::json!({
            "Action": {"MoveWindowToMonitor": {"id": window.id, "output": placement.monitor}}
        }))?;
        self.request(serde_json::json!({
            "Action": {"MoveWindowToFloating": {"id": window.id}}
        }))?;
        let (local_x, local_y) = if let Some(output) = output {
            (
                placement.x.saturating_sub(output.x),
                placement
                    .y
                    .saturating_sub(output.y)
                    .clamp(0, output.height.saturating_sub(placement.height) as i32),
            )
        } else {
            (placement.x.max(0), placement.y.max(0))
        };
        self.request(serde_json::json!({
            "Action": {
                "MoveFloatingWindow": {
                    "id": window.id,
                    "x": {"SetFixed": local_x as f64},
                    "y": {"SetFixed": local_y as f64}
                }
            }
        }))
    }

    fn windows(&self) -> Result<Vec<NiriWindow>, WorkerError> {
        let response = self.request(serde_json::Value::String("Windows".to_owned()))?;
        let windows = response
            .get("Ok")
            .and_then(|ok| ok.get("Windows"))
            .cloned()
            .ok_or_else(|| WorkerError::Message("Niri Windows reply was malformed".to_owned()))?;
        Ok(serde_json::from_value(windows)?)
    }
}

#[derive(Clone, Copy)]
struct NiriPlacement<'a> {
    monitor: &'a str,
    x: i32,
    y: i32,
    height: u32,
}

#[derive(Default)]
struct NiriLiveState {
    workspaces: Vec<NiriWorkspace>,
    windows: Vec<NiriWindow>,
    layouts: NiriKeyboardLayouts,
}

struct NiriEventUpdate {
    changed: bool,
    refresh_outputs: bool,
}

impl NiriLiveState {
    fn apply_event(&mut self, event: &serde_json::Value) -> NiriEventUpdate {
        let mut changed = false;
        let mut refresh_outputs = false;
        if let Some(event) = event.get("WorkspacesChanged") {
            if let Some(workspaces) = event.get("workspaces") {
                if let Ok(workspaces) = serde_json::from_value(workspaces.clone()) {
                    self.workspaces = workspaces;
                    changed = true;
                    refresh_outputs = true;
                }
            }
        } else if let Some(event) = event.get("WindowsChanged") {
            if let Some(windows) = event.get("windows") {
                if let Ok(windows) = serde_json::from_value(windows.clone()) {
                    self.windows = windows;
                    changed = true;
                }
            }
        } else if let Some(event) = event.get("WindowOpenedOrChanged") {
            if let Some(window) = event.get("window") {
                if let Ok(window) = serde_json::from_value::<NiriWindow>(window.clone()) {
                    if let Some(existing) = self
                        .windows
                        .iter_mut()
                        .find(|candidate| candidate.id == window.id)
                    {
                        *existing = window;
                    } else {
                        self.windows.push(window);
                    }
                    changed = true;
                }
            }
        } else if let Some(event) = event.get("WindowClosed") {
            if let Some(id) = event.get("id").and_then(serde_json::Value::as_u64) {
                self.windows.retain(|window| window.id != id);
                changed = true;
            }
        } else if let Some(event) = event.get("WindowFocusChanged") {
            let focused = event.get("id").and_then(serde_json::Value::as_u64);
            for window in &mut self.windows {
                window.is_focused = Some(window.id) == focused;
            }
            changed = true;
        } else if let Some(event) = event.get("WorkspaceActivated") {
            let id = event.get("id").and_then(serde_json::Value::as_u64);
            if let Some(output) = self
                .workspaces
                .iter()
                .find(|workspace| Some(workspace.id) == id)
                .and_then(|workspace| workspace.output.clone())
            {
                for workspace in &mut self.workspaces {
                    if workspace.output == Some(output.clone()) {
                        workspace.is_active = Some(workspace.id) == id;
                    }
                    if event.get("focused").and_then(serde_json::Value::as_bool) == Some(true) {
                        workspace.is_focused = Some(workspace.id) == id;
                    }
                }
                changed = true;
            }
        } else if let Some(event) = event.get("WorkspaceActiveWindowChanged") {
            if let (Some(workspace_id), Some(active)) = (
                event
                    .get("workspace_id")
                    .and_then(serde_json::Value::as_u64),
                event.get("active_window_id"),
            ) {
                if let Some(workspace) = self
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == workspace_id)
                {
                    workspace.active_window_id = active.as_u64();
                    changed = true;
                }
            }
        } else if let Some(event) = event.get("KeyboardLayoutsChanged") {
            if let Some(layouts) = event.get("keyboard_layouts") {
                if let Ok(layouts) = serde_json::from_value(layouts.clone()) {
                    self.layouts = layouts;
                    changed = true;
                }
            }
        } else if let Some(event) = event.get("KeyboardLayoutSwitched") {
            if let Some(index) = event.get("idx").and_then(serde_json::Value::as_u64) {
                self.layouts.current_idx = index as usize;
                changed = true;
            }
        } else if event.get("ConfigLoaded").is_some() {
            refresh_outputs = true;
        }
        NiriEventUpdate {
            changed,
            refresh_outputs,
        }
    }

    fn snapshot(&self, outputs: Vec<NiriOutput>) -> CompositorState {
        let focused_workspace = self
            .workspaces
            .iter()
            .find(|workspace| workspace.is_focused);
        let focused_monitor = focused_workspace
            .and_then(|workspace| workspace.output.clone())
            .unwrap_or_default();
        let active_window_title = self
            .windows
            .iter()
            .find(|window| window.is_focused)
            .and_then(|window| window.title.clone())
            .unwrap_or_default();
        let monitors = outputs
            .into_iter()
            .filter_map(|output| {
                let logical = output.logical?;
                let active_workspace = self
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.output.as_deref() == Some(&output.name) && workspace.is_active
                    })
                    .map(|workspace| i32::from(workspace.idx))
                    .unwrap_or(0);
                Some(Monitor {
                    id: output.name.clone(),
                    connector: output.name.clone(),
                    description: format!("{} {}", output.make, output.model)
                        .trim()
                        .to_owned(),
                    x: logical.x,
                    y: logical.y,
                    width: (logical.width as f64 * logical.scale.max(0.5)) as u32,
                    height: (logical.height as f64 * logical.scale.max(0.5)) as u32,
                    scale: logical.scale,
                    focused: output.name == focused_monitor,
                    active_workspace,
                })
            })
            .collect::<Vec<_>>();
        let workspaces = self
            .workspaces
            .iter()
            .filter(|workspace| workspace.name.as_deref() != Some("scratchpad"))
            .map(|workspace| Workspace {
                id: i32::from(workspace.idx),
                occupied: workspace.active_window_id.is_some(),
                focused: workspace.is_focused,
                visible: workspace.is_active,
                monitor: workspace.output.clone().unwrap_or_default(),
                windows: u32::from(workspace.active_window_id.is_some()),
                temporary: false,
            })
            .collect();
        let current_layout = self
            .layouts
            .names
            .get(self.layouts.current_idx)
            .cloned()
            .unwrap_or_default();
        CompositorState {
            kind: CompositorKind::Niri,
            monitors,
            workspaces,
            policy_primary: policy_primary(),
            focused_workspace: focused_workspace
                .map(|workspace| i32::from(workspace.idx))
                .unwrap_or(1),
            focused_monitor,
            active_window_title,
            keyboard: KeyboardState {
                visible: self.layouts.names.len() > 1 && !current_layout.is_empty(),
                code: layout_code(&current_layout),
                layout: current_layout.clone(),
                xkb_layout: current_layout,
                xkb_variant: String::new(),
            },
            error: String::new(),
        }
    }
}

struct X11Worker {
    telemetry: Telemetry,
}

impl X11Worker {
    fn new(telemetry: Telemetry) -> Self {
        Self { telemetry }
    }

    fn run(
        self,
        state_tx: SyncSender<CompositorState>,
        actions: Receiver<Action>,
        panel_ack: Arc<Mutex<Option<PanelPlacementTarget>>>,
    ) {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, EventMask};
        use x11rb::rust_connection::RustConnection;

        self.telemetry
            .emit("compositor_detected", [("kind", serde_json::json!("x11"))]);
        let connection = match RustConnection::connect(None) {
            Ok((connection, screen_number)) => {
                let root = connection.setup().roots[screen_number].root;
                let _ = connection.change_window_attributes(
                    root,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                );
                let _ = connection.flush();
                X11Context { connection, root }
            }
            Err(error) => {
                publish(
                    &state_tx,
                    error_state(CompositorKind::X11, format!("X11 connect failed: {error}")),
                );
                while actions.recv().is_ok() {}
                return;
            }
        };
        self.telemetry.emit(
            "compositor_subscribe",
            [
                ("kind", serde_json::json!("x11")),
                ("transport", serde_json::json!("x11-property-notify")),
                ("ok", serde_json::json!(true)),
            ],
        );
        loop {
            match connection.snapshot() {
                Ok(state) => publish(&state_tx, state),
                Err(error) => publish(&state_tx, error_state(CompositorKind::X11, error)),
            }
            let mut last_snapshot = std::time::Instant::now();
            loop {
                match connection.connection.poll_for_event() {
                    Ok(Some(_event)) => break,
                    Ok(None) => {
                        if x11_snapshot_due(last_snapshot, std::time::Instant::now()) {
                            match connection.snapshot() {
                                Ok(state) => publish(&state_tx, state),
                                Err(error) => {
                                    publish(&state_tx, error_state(CompositorKind::X11, error))
                                }
                            }
                            last_snapshot = std::time::Instant::now();
                        }
                        match actions.recv_timeout(Duration::from_millis(50)) {
                            Ok(Action::Quit) => return,
                            Ok(Action::SelectWorkspace(workspace)) => {
                                let _ = connection.select_workspace(workspace);
                            }
                            Ok(Action::FocusMonitor(_)) => {}
                            Ok(Action::NextKeyboardLayout) => {
                                let _ = launch::command("xkb-switch").arg("-n").status();
                            }
                            Ok(action @ Action::PlacePanel { .. }) => {
                                if let Some(target) = PanelPlacementTarget::from_action(&action) {
                                    *panel_ack
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                        Some(target);
                                }
                            }
                            Ok(Action::PlacePopup { .. }) => {}
                            Err(_) => {}
                        }
                    }
                    Err(error) => {
                        publish(&state_tx, error_state(CompositorKind::X11, error));
                        std::thread::sleep(Duration::from_millis(250));
                        break;
                    }
                }
            }
        }
    }
}

fn x11_snapshot_due(last: std::time::Instant, now: std::time::Instant) -> bool {
    now.saturating_duration_since(last) >= Duration::from_millis(500)
}

struct X11Context {
    connection: x11rb::rust_connection::RustConnection,
    root: u32,
}

impl X11Context {
    fn atom(&self, name: &[u8]) -> Result<u32, WorkerError> {
        use x11rb::protocol::xproto::ConnectionExt;
        Ok(self.connection.intern_atom(false, name)?.reply()?.atom)
    }

    fn property(&self, window: u32, atom: u32) -> Result<Vec<u8>, WorkerError> {
        use x11rb::protocol::xproto::ConnectionExt;
        let reply = self
            .connection
            .get_property(
                false,
                window,
                atom,
                x11rb::protocol::xproto::AtomEnum::ANY,
                0,
                1 << 20,
            )?
            .reply()?;
        Ok(reply.value)
    }

    fn snapshot(&self) -> Result<CompositorState, WorkerError> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;

        let current_atom = self.atom(b"_NET_CURRENT_DESKTOP")?;
        let count_atom = self.atom(b"_NET_NUMBER_OF_DESKTOPS")?;
        let clients_atom = self.atom(b"_NET_CLIENT_LIST")?;
        let active_atom = self.atom(b"_NET_ACTIVE_WINDOW")?;
        let name_atom = self.atom(b"_NET_WM_NAME")?;
        let desktop_atom = self.atom(b"_NET_WM_DESKTOP")?;
        let utf8_atom = self.atom(b"UTF8_STRING")?;
        let screen = self
            .connection
            .setup()
            .roots
            .iter()
            .find(|screen| screen.root == self.root)
            .ok_or_else(|| WorkerError::Message("X11 root screen disappeared".to_owned()))?;
        let current =
            u32_property(&self.property(self.root, current_atom)?).unwrap_or(0) as i32 + 1;
        let count = u32_property(&self.property(self.root, count_atom)?)
            .unwrap_or(1)
            .max(1) as usize;
        let clients_property = self.property(self.root, clients_atom)?;
        let clients = u32_list(&clients_property);
        let active_window = u32_property(&self.property(self.root, active_atom)?).unwrap_or(0);
        let mut occupied = HashSet::new();
        for client in clients {
            if let Ok(value) = self.property(client, desktop_atom) {
                if let Some(desktop) = u32_property(&value) {
                    occupied.insert((desktop as i32) + 1);
                }
            }
        }
        let title = if active_window != 0 {
            let property = self
                .connection
                .get_property(false, active_window, name_atom, utf8_atom, 0, 4096)?
                .reply()?;
            String::from_utf8_lossy(&property.value).into_owned()
        } else {
            String::new()
        };
        let workspaces = (1..=count)
            .map(|id| Workspace {
                id: id as i32,
                occupied: occupied.contains(&(id as i32)),
                visible: current == id as i32,
                focused: current == id as i32,
                monitor: "X11".to_owned(),
                windows: u32::from(occupied.contains(&(id as i32))),
                temporary: false,
            })
            .collect();
        Ok(CompositorState {
            kind: CompositorKind::X11,
            monitors: vec![Monitor {
                id: "0".to_owned(),
                connector: "X11".to_owned(),
                description: "X11 screen".to_owned(),
                x: 0,
                y: 0,
                width: u32::from(screen.width_in_pixels),
                height: u32::from(screen.height_in_pixels),
                scale: 1.0,
                focused: true,
                active_workspace: current,
            }],
            workspaces,
            policy_primary: String::new(),
            focused_workspace: current,
            focused_monitor: "X11".to_owned(),
            active_window_title: title,
            keyboard: KeyboardState::default(),
            error: String::new(),
        })
    }

    fn select_workspace(&self, workspace: i32) -> Result<(), WorkerError> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{
            ClientMessageData, ClientMessageEvent, ConnectionExt, EventMask,
        };

        let atom = self.atom(b"_NET_CURRENT_DESKTOP")?;
        let event = ClientMessageEvent::new(
            32,
            self.root,
            atom,
            ClientMessageData::from([workspace.saturating_sub(1).max(0) as u32, 0, 0, 0, 0]),
        );
        self.connection.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        )?;
        self.connection.flush()?;
        Ok(())
    }
}

fn u32_property(value: &[u8]) -> Option<u32> {
    value
        .get(..4)
        .map(|bytes| u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn u32_list(value: &[u8]) -> impl Iterator<Item = u32> + '_ {
    let (chunks, _) = value.as_chunks::<4>();
    chunks.iter().map(|bytes| u32::from_ne_bytes(*bytes))
}

#[derive(Debug, Deserialize)]
struct HyprWorkspace {
    id: i32,
    windows: u32,
    monitor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HyprMonitor {
    id: i64,
    name: String,
    description: Option<String>,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
    scale: f64,
    focused: bool,
    #[serde(rename = "activeWorkspace")]
    active_workspace: Option<HyprWorkspaceRef>,
}

#[derive(Debug, Deserialize)]
struct HyprWorkspaceRef {
    id: i32,
}

#[derive(Debug, Deserialize)]
struct HyprWindow {
    title: Option<String>,
    monitor: i64,
    workspace: Option<HyprWorkspaceRef>,
}

#[derive(Debug, Deserialize)]
struct HyprPanelClient {
    address: String,
    pid: i64,
    class: Option<String>,
    title: Option<String>,
    monitor: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct HyprDevices {
    #[serde(default)]
    keyboards: Vec<HyprKeyboard>,
}

#[derive(Debug, Deserialize)]
struct HyprKeyboard {
    name: String,
    #[serde(rename = "active_keymap")]
    active_keymap: Option<String>,
    #[serde(rename = "active_layout_index")]
    active_layout_index: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct HyprOption {
    #[serde(default, rename = "str")]
    str: String,
}

fn split_option(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct NiriWorkspace {
    id: u64,
    idx: u8,
    name: Option<String>,
    output: Option<String>,
    is_focused: bool,
    is_active: bool,
    active_window_id: Option<u64>,
}

impl Default for NiriWorkspace {
    fn default() -> Self {
        Self {
            id: 0,
            idx: 1,
            name: None,
            output: None,
            is_focused: false,
            is_active: false,
            active_window_id: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct NiriWindow {
    id: u64,
    title: Option<String>,
    app_id: Option<String>,
    pid: Option<i32>,
    is_focused: bool,
    workspace_id: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct NiriKeyboardLayouts {
    names: Vec<String>,
    current_idx: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
struct NiriOutput {
    name: String,
    make: String,
    model: String,
    logical: Option<NiriLogicalOutput>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
struct NiriLogicalOutput {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
}

impl Default for NiriLogicalOutput {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
            scale: 1.0,
        }
    }
}

#[derive(Debug)]
enum WorkerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    X11(x11rb::errors::ReplyError),
    Message(String),
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Json(error) => error.fmt(formatter),
            Self::X11(error) => error.fmt(formatter),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

impl From<std::io::Error> for WorkerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for WorkerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<x11rb::errors::ReplyError> for WorkerError {
    fn from(error: x11rb::errors::ReplyError) -> Self {
        Self::X11(error)
    }
}

impl From<x11rb::errors::ConnectError> for WorkerError {
    fn from(error: x11rb::errors::ConnectError) -> Self {
        Self::Message(error.to_string())
    }
}

impl From<x11rb::errors::ConnectionError> for WorkerError {
    fn from(error: x11rb::errors::ConnectionError) -> Self {
        Self::Message(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relevant_hyprland_events_are_bounded_and_explicit() {
        for event in [
            "workspacev2>>1,2",
            "activewindow>>0x1,title",
            "monitoraddedv2>>1,DP-1",
            "activelayout>>keyboard,English",
        ] {
            assert!(event_needs_snapshot(event), "{event}");
        }
        assert!(!event_needs_snapshot("bell>>x"));
    }

    #[test]
    fn option_lists_preserve_keyboard_layout_indices() {
        assert_eq!(split_option(" us, ru, "), ["us", "ru"]);
        assert_eq!(split_option(""), Vec::<String>::new());
    }

    #[test]
    fn properties_are_native_endian_u32_values() {
        let value = 3_u32.to_ne_bytes();
        assert_eq!(u32_property(&value), Some(3));
        assert_eq!(
            u32_list(&[value, 4_u32.to_ne_bytes()].concat()).collect::<Vec<_>>(),
            [3, 4]
        );
    }

    #[test]
    fn unavailable_session_has_a_truthful_error_state() {
        let state = error_state(CompositorKind::Unavailable, "no session");
        assert_eq!(state.kind, CompositorKind::Unavailable);
        assert!(state.error.contains("no session"));
    }

    #[test]
    fn niri_socket_events_replace_full_state_without_helper_processes() {
        let mut live = NiriLiveState::default();
        let update = live.apply_event(&serde_json::json!({
            "WorkspacesChanged": {
                "workspaces": [{
                    "id": 91,
                    "idx": 1,
                    "output": "DP-1",
                    "is_active": true,
                    "is_focused": true,
                    "active_window_id": 7
                }]
            }
        }));
        assert!(update.changed);
        let state = live.snapshot(vec![NiriOutput {
            name: "DP-1".to_owned(),
            make: "smplOS".to_owned(),
            model: "Display".to_owned(),
            logical: Some(NiriLogicalOutput {
                x: 10,
                y: 20,
                width: 1920,
                height: 1080,
                scale: 1.5,
            }),
        }]);
        assert_eq!(state.monitors[0].connector, "DP-1");
        assert_eq!(state.workspaces[0].id, 1);
        assert_eq!(state.monitors[0].width, 2880);
    }

    #[test]
    fn x11_poll_catches_title_only_changes() {
        let now = std::time::Instant::now();
        assert!(!x11_snapshot_due(now, now + Duration::from_millis(499)));
        assert!(x11_snapshot_due(now, now + Duration::from_millis(500)));
    }
}
