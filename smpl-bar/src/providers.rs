use futures::StreamExt;
use smpl_shell::telemetry::Telemetry;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout};
use tokio::sync::mpsc;
use zbus::message::Type;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

const CONTROL_SERVICE: &str = "org.smplos.ControlSurface";
const CONTROL_PATH: &str = "/org/smplos/ControlSurface";
const CONTROL_INTERFACE: &str = "org.smplos.ControlSurface1";
const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const NETWORK_CONNECTION: &str = "org.freedesktop.NetworkManager.Connection.Active";
const BLUEZ_INTERFACE: &str = "org.bluez.Adapter1";
const SYNC_SERVICE: &str = "org.smpl.SyncCenter";
const SYNC_PATH: &str = "/org/smpl/SyncCenter";
const SYNC_INTERFACE: &str = "org.smpl.SyncCenter";
const SAMPLE_TIMEOUT: Duration = Duration::from_millis(450);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProviderState {
    pub wifi_available: Option<bool>,
    pub wifi_on: Option<bool>,
    pub wired_available: Option<bool>,
    pub wired: Option<bool>,
    pub online: Option<bool>,
    pub vpn: Option<bool>,
    pub ssid: Option<String>,
    pub bluetooth_available: Option<bool>,
    pub bluetooth_on: Option<bool>,
    pub volume: Option<u8>,
    pub volume_muted: Option<bool>,
    pub notification_count: Option<u32>,
    pub notifications_paused: Option<bool>,
    pub sync_active: Option<bool>,
    pub keypad_present: Option<bool>,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderSnapshot {
    pub state: ProviderState,
    pub keypad_sheet: Option<serde_json::Value>,
    pub error: String,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAction {
    ToggleCheatsheet,
    Quit,
}

pub struct ProviderHandle {
    snapshot: Arc<Mutex<ProviderSnapshot>>,
    actions: mpsc::Sender<ProviderAction>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl ProviderHandle {
    pub fn spawn(telemetry: Telemetry) -> Self {
        let snapshot = Arc::new(Mutex::new(ProviderSnapshot::default()));
        let (actions, action_rx) = mpsc::channel(16);
        let worker_snapshot = snapshot.clone();
        let worker = match std::thread::Builder::new()
            .name("desktop-providers".to_owned())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    let mut snapshot = worker_snapshot
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    snapshot.error = "cannot start provider runtime".to_owned();
                    snapshot.revision = snapshot.revision.wrapping_add(1);
                    return;
                };
                runtime.block_on(run(worker_snapshot, action_rx, telemetry));
            })
        {
            Ok(worker) => Some(worker),
            Err(error) => {
                let mut state = snapshot
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.error = format!("cannot start desktop providers: {error}");
                state.revision = state.revision.wrapping_add(1);
                None
            }
        };
        Self {
            snapshot,
            actions,
            worker,
        }
    }

    pub fn changed_since(&self, revision: u64) -> Option<ProviderSnapshot> {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if snapshot.revision == revision {
            return None;
        }
        let update = snapshot.clone();
        snapshot.keypad_sheet = None;
        Some(update)
    }

    pub fn send(&self, action: ProviderAction) {
        if let Err(error) = self.actions.try_send(action) {
            eprintln!("smpl-bar: provider action queue rejected: {error}");
        }
    }
}

impl Drop for ProviderHandle {
    fn drop(&mut self) {
        let _ = self.actions.try_send(ProviderAction::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct ProviderWorker {
    session: zbus::Connection,
    system: Option<zbus::Connection>,
    state: Arc<Mutex<ProviderSnapshot>>,
    telemetry: Telemetry,
}

struct AudioSubscriber {
    child: Child,
    lines: Lines<BufReader<ChildStdout>>,
}

impl Drop for AudioSubscriber {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

impl AudioSubscriber {
    async fn shutdown(mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

impl ProviderWorker {
    async fn connect(
        state: Arc<Mutex<ProviderSnapshot>>,
        telemetry: Telemetry,
    ) -> Result<Self, String> {
        let session = zbus::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .method_timeout(SAMPLE_TIMEOUT)
            .build()
            .await
            .map_err(|error| error.to_string())?;
        let system = if std::env::var_os("SMPL_BAR_PRIVATE").is_some() {
            None
        } else {
            match zbus::connection::Builder::system() {
                Ok(builder) => builder.method_timeout(SAMPLE_TIMEOUT).build().await.ok(),
                Err(_) => None,
            }
        };
        let worker = Self {
            session,
            system,
            state,
            telemetry,
        };
        Ok(worker)
    }

    async fn refresh_all(&mut self) {
        self.refresh_network().await;
        self.refresh_bluetooth().await;
        self.refresh_control_surface().await;
        self.refresh_sync().await;
        self.refresh_audio().await;
        self.refresh_notifications().await;
    }

    async fn refresh_network(&self) {
        let Some(system) = &self.system else {
            return;
        };
        let wireless_enabled = property::<bool>(
            system,
            NETWORK_MANAGER,
            NETWORK_MANAGER_PATH,
            NETWORK_MANAGER,
            "WirelessEnabled",
        )
        .await
        .ok();
        let state = property::<u32>(
            system,
            NETWORK_MANAGER,
            NETWORK_MANAGER_PATH,
            NETWORK_MANAGER,
            "State",
        )
        .await
        .ok();
        let connectivity = property::<u32>(
            system,
            NETWORK_MANAGER,
            NETWORK_MANAGER_PATH,
            NETWORK_MANAGER,
            "Connectivity",
        )
        .await
        .ok();
        let active = property::<Vec<OwnedObjectPath>>(
            system,
            NETWORK_MANAGER,
            NETWORK_MANAGER_PATH,
            NETWORK_MANAGER,
            "ActiveConnections",
        )
        .await
        .unwrap_or_default();
        let mut wired = false;
        let mut wireless = false;
        let mut vpn = false;
        let mut ssid = String::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        for connection in active.into_iter().take(16) {
            if Instant::now() >= deadline {
                break;
            }
            let path = connection.to_string();
            let kind =
                property::<String>(system, NETWORK_MANAGER, &path, NETWORK_CONNECTION, "Type")
                    .await
                    .unwrap_or_default();
            let active_state =
                property::<u32>(system, NETWORK_MANAGER, &path, NETWORK_CONNECTION, "State")
                    .await
                    .unwrap_or(0);
            if active_state != 2 {
                continue;
            }
            match kind.as_str() {
                "802-11-wireless" => {
                    wireless = true;
                    if ssid.is_empty() {
                        if let Ok(access_point) = property::<OwnedObjectPath>(
                            system,
                            NETWORK_MANAGER,
                            &path,
                            NETWORK_CONNECTION,
                            "SpecificObject",
                        )
                        .await
                        {
                            let bytes = property::<Vec<u8>>(
                                system,
                                NETWORK_MANAGER,
                                &access_point.to_string(),
                                "org.freedesktop.NetworkManager.AccessPoint",
                                "Ssid",
                            )
                            .await
                            .unwrap_or_default();
                            ssid = String::from_utf8_lossy(&bytes)
                                .trim_matches('\0')
                                .chars()
                                .take(96)
                                .collect();
                        }
                    }
                }
                "802-3-ethernet" => wired = true,
                "vpn" | "wireguard" | "tun" => vpn = true,
                value if value.contains("vpn") => vpn = true,
                _ => {}
            }
        }
        self.update(ProviderState {
            wifi_available: wireless_enabled.map(|_| true),
            wifi_on: wireless_enabled.map(|enabled| enabled && wireless),
            wired_available: wired.then_some(true),
            wired: Some(wired),
            online: connectivity
                .map(|value| value == 4)
                .or_else(|| state.map(|value| value >= 50)),
            vpn: Some(vpn),
            ssid: Some(ssid),
            ..ProviderState::default()
        });
    }

    async fn refresh_bluetooth(&self) {
        let Some(system) = &self.system else {
            return;
        };
        let message = system
            .call_method(
                Some("org.bluez"),
                "/",
                Some("org.freedesktop.DBus.ObjectManager"),
                "GetManagedObjects",
                &(),
            )
            .await;
        let Ok(message) = message else {
            return;
        };
        type Managed = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
        let Ok(objects) = message.body().deserialize::<Managed>() else {
            return;
        };
        let mut available = false;
        let mut powered = false;
        for interfaces in objects.values() {
            let Some(adapter) = interfaces.get(BLUEZ_INTERFACE) else {
                continue;
            };
            available = true;
            powered |= adapter
                .get("Powered")
                .and_then(|value| bool::try_from(value.try_clone().ok()?).ok())
                .unwrap_or(false);
        }
        self.update(ProviderState {
            bluetooth_available: Some(available),
            bluetooth_on: Some(powered),
            ..ProviderState::default()
        });
    }

    async fn refresh_control_surface(&self) {
        if !name_has_owner(&self.session, CONTROL_SERVICE).await {
            return;
        }
        let keypad_present = property::<bool>(
            &self.session,
            CONTROL_SERVICE,
            CONTROL_PATH,
            CONTROL_INTERFACE,
            "DevicePresent",
        )
        .await
        .ok();
        let visible = property::<bool>(
            &self.session,
            CONTROL_SERVICE,
            CONTROL_PATH,
            CONTROL_INTERFACE,
            "CheatsheetVisible",
        )
        .await
        .unwrap_or(false);
        self.update(ProviderState {
            keypad_present,
            ..ProviderState::default()
        });
        if visible {
            self.refresh_cheatsheet().await;
        } else {
            self.set_cheatsheet(serde_json::json!({"visible": false}));
        }
    }

    async fn refresh_cheatsheet(&self) {
        let message = self
            .session
            .call_method(
                Some(CONTROL_SERVICE),
                CONTROL_PATH,
                Some(CONTROL_INTERFACE),
                "GetCheatsheet",
                &(),
            )
            .await;
        let Some(raw) = message
            .ok()
            .and_then(|message| message.body().deserialize::<String>().ok())
        else {
            return;
        };
        if let Ok(value) = serde_json::from_str(&raw) {
            self.set_cheatsheet(value);
        }
    }

    async fn refresh_sync(&self) {
        let active = property::<bool>(
            &self.session,
            SYNC_SERVICE,
            SYNC_PATH,
            SYNC_INTERFACE,
            "IsActive",
        )
        .await
        .ok();
        self.update(ProviderState {
            sync_active: active,
            ..ProviderState::default()
        });
    }

    async fn refresh_audio(&self) {
        if let Some(volume) = bounded_output(
            ["pactl", "get-sink-volume", "@DEFAULT_SINK@"],
            SAMPLE_TIMEOUT,
        )
        .await
        {
            let volume = first_percentage(&String::from_utf8_lossy(&volume));
            let muted =
                bounded_output(["pactl", "get-sink-mute", "@DEFAULT_SINK@"], SAMPLE_TIMEOUT)
                    .await
                    .map(|output| String::from_utf8_lossy(&output).contains("yes"));
            self.update(ProviderState {
                volume,
                volume_muted: muted,
                ..ProviderState::default()
            });
        }
    }

    async fn refresh_notifications(&self) {
        let count = bounded_output(["dunstctl", "count", "history"], SAMPLE_TIMEOUT)
            .await
            .and_then(|output| String::from_utf8_lossy(&output).trim().parse::<u32>().ok());
        let paused = bounded_output(["dunstctl", "is-paused"], SAMPLE_TIMEOUT)
            .await
            .map(|output| String::from_utf8_lossy(&output).trim() == "true");
        self.update(ProviderState {
            notification_count: count,
            notifications_paused: paused,
            ..ProviderState::default()
        });
    }

    async fn toggle_cheatsheet(&self) -> Result<(), String> {
        smpl_shell::launch::close_popups().map_err(|error| error.to_string())?;
        self.session
            .call_method(
                Some(CONTROL_SERVICE),
                CONTROL_PATH,
                Some(CONTROL_INTERFACE),
                "ToggleCheatsheet",
                &(),
            )
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn handle_session_signal(&mut self, message: zbus::message::Message) {
        let interface = message.header().interface().map(ToString::to_string);
        let member = message.header().member().map(ToString::to_string);
        match (interface.as_deref(), member.as_deref()) {
            (Some(CONTROL_INTERFACE), Some("DeviceChanged")) => {
                if let Ok((present, _)) = message.body().deserialize::<(bool, String)>() {
                    self.update(ProviderState {
                        keypad_present: Some(present),
                        ..ProviderState::default()
                    });
                }
            }
            (Some(CONTROL_INTERFACE), Some("CheatsheetVisibilityChanged")) => {
                if let Ok(visible) = message.body().deserialize::<bool>() {
                    if visible {
                        self.refresh_cheatsheet().await;
                    } else {
                        self.set_cheatsheet(serde_json::json!({"visible": false}));
                    }
                }
            }
            (Some(CONTROL_INTERFACE), Some("CheatsheetChanged")) => {
                if let Ok(raw) = message.body().deserialize::<String>() {
                    if let Ok(value) = serde_json::from_str(&raw) {
                        self.set_cheatsheet(value);
                    }
                }
            }
            (Some(SYNC_INTERFACE), Some("StatusChanged")) => self.refresh_sync().await,
            (Some("org.freedesktop.Notifications"), Some("NotificationClosed"))
            | (Some("org.freedesktop.Notifications"), Some("ActionInvoked")) => {
                self.refresh_notifications().await;
            }
            (Some("org.freedesktop.DBus"), Some("NameOwnerChanged")) => {
                if let Ok((name, _, new)) = message.body().deserialize::<(String, String, String)>()
                {
                    if matches!(name.as_str(), CONTROL_SERVICE | SYNC_SERVICE) && !new.is_empty() {
                        self.refresh_control_surface().await;
                        self.refresh_sync().await;
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_system_signal(&self, message: zbus::message::Message) {
        let interface = message.header().interface().map(ToString::to_string);
        let member = message.header().member().map(ToString::to_string);
        match (interface.as_deref(), member.as_deref()) {
            (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged")) => {
                let Ok((changed_interface, changed, invalidated)) =
                    message
                        .body()
                        .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                else {
                    return;
                };
                let _ = invalidated;
                if changed_interface == NETWORK_MANAGER || changed_interface == NETWORK_CONNECTION {
                    self.apply_network_properties(&changed).await;
                } else if changed_interface == BLUEZ_INTERFACE {
                    self.apply_bluetooth_properties(&changed);
                }
            }
            (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesAdded"))
            | (Some("org.freedesktop.DBus.ObjectManager"), Some("InterfacesRemoved")) => {
                self.refresh_bluetooth().await;
                self.refresh_network().await;
            }
            _ => {}
        }
    }

    async fn apply_network_properties(&self, values: &HashMap<String, OwnedValue>) {
        let wifi_on = value::<bool>(values, "WirelessEnabled");
        let _ = wifi_on;
        self.refresh_network().await;
    }

    fn apply_bluetooth_properties(&self, values: &HashMap<String, OwnedValue>) {
        if let Some(powered) = value::<bool>(values, "Powered") {
            self.update(ProviderState {
                bluetooth_available: Some(true),
                bluetooth_on: Some(powered),
                ..ProviderState::default()
            });
        }
    }

    fn update(&self, patch: ProviderState) {
        self.telemetry.emit(
            "provider_update",
            [
                (
                    "network",
                    serde_json::json!(patch.online.is_some() || patch.ssid.is_some()),
                ),
                ("bluetooth", serde_json::json!(patch.bluetooth_on.is_some())),
                ("audio", serde_json::json!(patch.volume.is_some())),
                (
                    "notifications",
                    serde_json::json!(patch.notification_count.is_some()),
                ),
            ],
        );
        let mut snapshot = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        merge_provider(&mut snapshot.state, patch);
        snapshot.revision = snapshot.revision.wrapping_add(1);
        snapshot.error.clear();
    }

    fn set_cheatsheet(&self, value: serde_json::Value) {
        let mut snapshot = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        snapshot.keypad_sheet = Some(value);
        snapshot.revision = snapshot.revision.wrapping_add(1);
    }

    fn error(&self, error: impl Into<String>) {
        let error = error.into();
        self.telemetry
            .emit("provider_error", [("error", serde_json::json!(error))]);
        let mut snapshot = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        snapshot.error = error;
        snapshot.revision = snapshot.revision.wrapping_add(1);
    }
}

async fn run(
    state: Arc<Mutex<ProviderSnapshot>>,
    mut actions: mpsc::Receiver<ProviderAction>,
    telemetry: Telemetry,
) {
    let mut worker = match ProviderWorker::connect(state.clone(), telemetry.clone()).await {
        Ok(worker) => worker,
        Err(error) => {
            let mut snapshot = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            snapshot.error = error.clone();
            snapshot.revision = snapshot.revision.wrapping_add(1);
            telemetry.emit(
                "providers_unavailable",
                [("error", serde_json::json!(error))],
            );
            return;
        }
    };
    let session_rule = zbus::MatchRule::builder().msg_type(Type::Signal).build();
    let mut session_signals =
        match zbus::MessageStream::for_match_rule(session_rule, &worker.session, Some(128)).await {
            Ok(stream) => stream,
            Err(error) => {
                worker.error(error.to_string());
                return;
            }
        };
    let mut system_signals = if let Some(system) = &worker.system {
        zbus::MessageStream::for_match_rule(
            zbus::MatchRule::builder().msg_type(Type::Signal).build(),
            system,
            Some(128),
        )
        .await
        .ok()
    } else {
        None
    };
    let mut refresh = tokio::time::interval(Duration::from_secs(15));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    worker.refresh_all().await;
    let mut audio_lines = spawn_audio_subscriber();
    loop {
        tokio::select! {
            Some(action) = actions.recv() => match action {
                ProviderAction::ToggleCheatsheet => {
                    if let Err(error) = worker.toggle_cheatsheet().await {
                        worker.error(error);
                    }
                },
                ProviderAction::Quit => {
                    if let Some(audio) = audio_lines.take() {
                        audio.shutdown().await;
                    }
                    return;
                },
            },
            Some(message) = session_signals.next() => {
                if let Ok(message) = message {
                    worker.handle_session_signal(message).await;
                }
            }
            message = async {
                if let Some(stream) = &mut system_signals {
                    stream.next().await
                } else {
                    std::future::pending::<Option<Result<zbus::Message, zbus::Error>>>().await
                }
            } => if let Some(Ok(message)) = message {
                worker.handle_system_signal(message).await;
            },
            line = async {
                if let Some(audio) = &mut audio_lines {
                    audio.lines.next_line().await.transpose()
                } else {
                    None
                }
            }, if audio_lines.is_some() => {
                match line {
                    Some(Ok(line)) if line.contains("sink") => worker.refresh_audio().await,
                    Some(Err(error)) => {
                        worker.error(format!("audio event stream failed: {error}"));
                        audio_lines = None;
                    }
                    None => {
                        audio_lines = None;
                    }
                    _ => {}
                }
            }
            _ = refresh.tick() => {
                worker.refresh_all().await;
                if audio_lines.is_none() {
                    audio_lines = spawn_audio_subscriber();
                }
            }
        }
    }
}

fn spawn_audio_subscriber() -> Option<AudioSubscriber> {
    let mut command = smpl_shell::launch::command("pactl");
    command
        .arg("subscribe")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut command = tokio::process::Command::from(command);
    let mut child: Child = command.spawn().ok()?;
    let stdout = child.stdout.take()?;
    Some(AudioSubscriber {
        child,
        lines: BufReader::new(stdout).lines(),
    })
}

async fn bounded_output<const N: usize>(command: [&str; N], timeout: Duration) -> Option<Vec<u8>> {
    let output = tokio::process::Command::new(command[0])
        .args(&command[1..])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(timeout, output)
        .await
        .ok()?
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout)
}

async fn property<T>(
    connection: &zbus::Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Result<T, zbus::Error>
where
    T: TryFrom<OwnedValue>,
    T::Error: Into<zbus::Error>,
{
    let proxy = zbus::Proxy::new(
        connection,
        destination.to_owned(),
        path.to_owned(),
        interface.to_owned(),
    )
    .await?;
    proxy.get_property(name).await
}

async fn name_has_owner(connection: &zbus::Connection, name: &str) -> bool {
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "NameHasOwner",
            &name,
        )
        .await
        .ok()
        .and_then(|message| message.body().deserialize::<bool>().ok())
        .unwrap_or(false)
}

fn value<T>(values: &HashMap<String, OwnedValue>, key: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    values.get(key)?.try_clone().ok()?.try_into().ok()
}

fn first_percentage(source: &str) -> Option<u8> {
    source
        .split(|character: char| !character.is_ascii_digit())
        .find_map(|number| (!number.is_empty()).then(|| number.parse().ok()).flatten())
        .map(|value: u8| value.min(100))
}

fn merge_provider(current: &mut ProviderState, patch: ProviderState) {
    macro_rules! take {
        ($field:ident) => {
            if patch.$field.is_some() {
                current.$field = patch.$field;
            }
        };
    }
    take!(wifi_available);
    take!(wifi_on);
    take!(wired_available);
    take!(wired);
    take!(online);
    take!(vpn);
    take!(ssid);
    take!(bluetooth_available);
    take!(bluetooth_on);
    take!(volume);
    take!(volume_muted);
    take!(notification_count);
    take!(notifications_paused);
    take!(sync_active);
    take!(keypad_present);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_patches_preserve_unreported_values() {
        let mut state = ProviderState {
            volume: Some(30),
            ssid: Some("office".to_owned()),
            ..ProviderState::default()
        };
        merge_provider(
            &mut state,
            ProviderState {
                volume: Some(50),
                ..ProviderState::default()
            },
        );
        assert_eq!(state.volume, Some(50));
        assert_eq!(state.ssid.as_deref(), Some("office"));
    }

    #[test]
    fn audio_percentage_does_not_require_channel_balance() {
        assert_eq!(first_percentage("Volume: front-left: 42% / 12%"), Some(42));
        assert_eq!(first_percentage("muted"), None);
    }
}
