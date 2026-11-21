use crate::config::BarConfigFile;
use crate::launch;
use crate::monitor::{
    layout_code, CompositorKind, CompositorState, KeyboardState, Monitor, Workspace,
};
use crate::telemetry::Telemetry;
use serde::Deserialize;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Stdio};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::time::Duration;

const MAX_REPLY: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    SelectWorkspace(i32),
    FocusMonitor(String),
    NextKeyboardLayout,
    Quit,
}

pub struct CompositorHandle {
    pub kind: CompositorKind,
    pub states: Receiver<CompositorState>,
    actions: SyncSender<Action>,
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

pub fn spawn(_config: BarConfigFile, telemetry: Telemetry) -> CompositorHandle {
    let kind = detect();
    let (state_tx, states) = mpsc::sync_channel(4);
    let (actions, action_rx) = mpsc::sync_channel(32);
    let thread_telemetry = telemetry.clone();
    let thread_kind = kind;
    let _ = std::thread::Builder::new()
        .name("compositor".to_owned())
        .spawn(move || match thread_kind {
            CompositorKind::Hyprland => {
                HyprlandWorker::new(thread_telemetry).run(state_tx, action_rx)
            }
            CompositorKind::Niri => NiriWorker::new(thread_telemetry).run(state_tx, action_rx),
            CompositorKind::X11 => X11Worker::new(thread_telemetry).run(state_tx, action_rx),
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

    fn run(self, state_tx: SyncSender<CompositorState>, actions: Receiver<Action>) {
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
                    if self.wait_for_action(&actions) {
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
                if self.drain_actions(&actions) {
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

    fn wait_for_action(&self, actions: &Receiver<Action>) -> bool {
        match actions.recv_timeout(Duration::from_millis(500)) {
            Ok(Action::Quit) => true,
            Ok(action) => {
                self.handle_action(action);
                false
            }
            Err(_) => false,
        }
    }

    fn drain_actions(&self, actions: &Receiver<Action>) -> bool {
        loop {
            match actions.try_recv() {
                Ok(Action::Quit) => return true,
                Ok(action) => self.handle_action(action),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn handle_action(&self, action: Action) {
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
                        .err()
                        .map(|error| error.to_string())
                        .unwrap_or_default()
                        .into(),
                ),
            ],
        );
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
    telemetry: Telemetry,
}

impl NiriWorker {
    fn new(telemetry: Telemetry) -> Self {
        Self { telemetry }
    }

    fn run(self, state_tx: SyncSender<CompositorState>, actions: Receiver<Action>) {
        self.telemetry
            .emit("compositor_detected", [("kind", serde_json::json!("niri"))]);
        publish(&state_tx, self.snapshot());
        let mut event_stream = match NiriEventStream::spawn() {
            Ok(stream) => {
                self.telemetry.emit(
                    "compositor_subscribe",
                    [
                        ("kind", serde_json::json!("niri")),
                        ("transport", serde_json::json!("niri-msg-process-gate")),
                        ("ok", serde_json::json!(true)),
                    ],
                );
                Some(stream)
            }
            Err(error) => {
                let mut state = error_state(CompositorKind::Niri, error);
                publish(&state_tx, state.clone());
                self.telemetry.emit(
                    "compositor_subscribe",
                    [
                        ("kind", serde_json::json!("niri")),
                        ("ok", serde_json::json!(false)),
                    ],
                );
                state
                    .error
                    .push_str("; falling back to a one-second native process snapshot");
                publish(&state_tx, state);
                None
            }
        };
        let mut last_snapshot = std::time::Instant::now();
        loop {
            if let Ok(action) = actions.recv_timeout(Duration::from_millis(100)) {
                match action {
                    Action::Quit => break,
                    Action::SelectWorkspace(workspace) => {
                        self.command(["msg", "action", "focus-workspace", &workspace.to_string()])
                    }
                    Action::FocusMonitor(_monitor) => {}
                    Action::NextKeyboardLayout => {
                        self.command(["msg", "action", "switch-xkb-layout", "next"]);
                    }
                }
                publish(&state_tx, self.snapshot());
                continue;
            }
            if event_stream.is_none() && last_snapshot.elapsed() >= Duration::from_secs(1) {
                publish(&state_tx, self.snapshot());
                last_snapshot = std::time::Instant::now();
            } else if let Some(stream) = event_stream.as_mut() {
                match stream.poll_line() {
                    Ok(true) => {
                        publish(&state_tx, self.snapshot());
                        last_snapshot = std::time::Instant::now();
                    }
                    Ok(false) => {}
                    Err(_) => event_stream = None,
                }
            }
        }
    }

    fn command<const N: usize>(&self, args: [&str; N]) {
        if let Err(error) = launch::command("niri").args(args).status() {
            eprintln!("smpl-bar: niri action failed: {error}");
        }
    }

    fn output(&self, args: &[&str]) -> Result<Vec<u8>, WorkerError> {
        let output = launch::command("niri")
            .args(args)
            .output()
            .map_err(WorkerError::from)?;
        if !output.status.success() {
            return Err(WorkerError::Message(format!(
                "niri msg failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output.stdout)
    }

    fn snapshot(&self) -> CompositorState {
        let workspaces = self
            .output(&["msg", "--json", "workspaces"])
            .ok()
            .and_then(|reply| serde_json::from_slice::<Vec<NiriWorkspace>>(&reply).ok())
            .unwrap_or_default();
        let focused_window = self
            .output(&["msg", "--json", "focused-window"])
            .ok()
            .and_then(|reply| serde_json::from_slice::<NiriWindow>(&reply).ok());
        let mut sorted: Vec<_> = workspaces
            .into_iter()
            .filter(|workspace| workspace.name.as_deref() != Some("scratchpad"))
            .collect();
        sorted.sort_by_key(|workspace| workspace.idx);
        let focused_index = sorted
            .iter()
            .position(|workspace| workspace.is_focused)
            .map(|index| index + 1)
            .unwrap_or(1);
        let result = sorted
            .iter()
            .enumerate()
            .map(|(index, workspace)| Workspace {
                id: (index + 1) as i32,
                occupied: workspace.active_window_id.is_some(),
                focused: workspace.is_focused,
                visible: workspace.is_focused,
                monitor: workspace.output.clone().unwrap_or_default(),
                windows: u32::from(workspace.active_window_id.is_some()),
                temporary: false,
            })
            .collect();
        CompositorState {
            kind: CompositorKind::Niri,
            monitors: Vec::new(),
            workspaces: result,
            focused_workspace: focused_index as i32,
            focused_monitor: sorted
                .iter()
                .find(|workspace| workspace.is_focused)
                .and_then(|workspace| workspace.output.clone())
                .unwrap_or_default(),
            active_window_title: focused_window
                .and_then(|window| window.title)
                .unwrap_or_default(),
            keyboard: KeyboardState::default(),
            error: String::new(),
        }
    }
}

struct NiriEventStream {
    child: Child,
    reader: BufReader<ChildStdout>,
}

impl NiriEventStream {
    fn spawn() -> std::io::Result<Self> {
        let mut child = launch::command("niri")
            .args(["msg", "event-stream"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "niri event stream has no stdout",
            )
        })?;
        let flags = unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            let _ = child.kill();
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            child,
            reader: BufReader::new(stdout),
        })
    }

    fn poll_line(&mut self) -> std::io::Result<bool> {
        let mut line = Vec::new();
        match self.reader.read_until(b'\n', &mut line) {
            Ok(0) => match self.child.try_wait()? {
                Some(_) => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "niri event stream exited",
                )),
                None => Ok(false),
            },
            Ok(_) => Ok(true),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
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

    fn run(self, state_tx: SyncSender<CompositorState>, actions: Receiver<Action>) {
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
            loop {
                match connection.connection.poll_for_event() {
                    Ok(Some(_event)) => break,
                    Ok(None) => match actions.recv_timeout(Duration::from_millis(50)) {
                        Ok(Action::Quit) => return,
                        Ok(Action::SelectWorkspace(workspace)) => {
                            let _ = connection.select_workspace(workspace);
                        }
                        Ok(Action::FocusMonitor(_)) => {}
                        Ok(Action::NextKeyboardLayout) => {
                            let _ = launch::command("xkb-switch").arg("-n").status();
                        }
                        Err(_) => {}
                    },
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

#[derive(Debug, Deserialize)]
struct NiriWorkspace {
    idx: i32,
    name: Option<String>,
    output: Option<String>,
    #[serde(rename = "is_focused")]
    is_focused: bool,
    #[serde(rename = "active_window_id")]
    active_window_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct NiriWindow {
    title: Option<String>,
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
}
