use futures::StreamExt;
use serde::{Deserialize, Serialize};
use smpl_shell::icons::IconResolver;
use smpl_shell::telemetry::Telemetry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use zbus::message::{Header, Type as MessageType};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Type};

const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const DBUS_MENU: &str = "com.canonical.dbusmenu";
const DEFAULT_ITEM_PATH: &str = "/StatusNotifierItem";
const MAX_ITEMS: usize = 32;
const MAX_MENU_ITEMS: usize = 64;
const MAX_ICON_PIXELS: usize = 128 * 128;
const MAX_ICON_BYTES: usize = MAX_ICON_PIXELS * 4;
const MAX_REFRESH_CYCLE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TrayIcon {
    #[default]
    Empty,
    File(PathBuf),
    Pixmap {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayItem {
    pub key: String,
    pub title: String,
    pub id: String,
    pub category: String,
    pub status: String,
    pub item_is_menu: bool,
    pub icon: TrayIcon,
    pub overlay: TrayIcon,
}

impl TrayItem {
    pub fn needs_attention(&self) -> bool {
        self.status == "NeedsAttention"
    }

    pub fn visible(&self) -> bool {
        self.status != "Passive"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenuEntry {
    pub item_id: i32,
    pub label: String,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub level: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayMenu {
    pub item_key: String,
    pub title: String,
    pub entries: Vec<TrayMenuEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SniStatus {
    pub watcher_owned: bool,
    pub host_registered: bool,
    pub item_count: usize,
    pub error: String,
}

impl Default for SniStatus {
    fn default() -> Self {
        Self {
            watcher_owned: false,
            host_registered: false,
            item_count: 0,
            error: "not connected".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SniSnapshot {
    pub items: Vec<TrayItem>,
    pub menu: Option<TrayMenu>,
    pub status: SniStatus,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SniAction {
    Activate {
        key: String,
        x: i32,
        y: i32,
    },
    Secondary {
        key: String,
        x: i32,
        y: i32,
    },
    Scroll {
        key: String,
        delta: i32,
        horizontal: bool,
    },
    MenuEvent {
        key: String,
        item_id: i32,
    },
}

pub struct SniHandle {
    state: Arc<Mutex<SniSnapshot>>,
    actions: mpsc::Sender<SniAction>,
}

impl SniHandle {
    pub fn spawn(telemetry: Telemetry) -> Self {
        let state = Arc::new(Mutex::new(SniSnapshot::default()));
        let (actions, action_rx) = mpsc::channel(64);
        let worker_state = state.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("sni-host".to_owned())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    let mut snapshot = worker_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    snapshot.status.error = "cannot start SNI runtime".to_owned();
                    snapshot.revision = snapshot.revision.wrapping_add(1);
                    return;
                };
                runtime.block_on(run(worker_state, action_rx, telemetry));
            })
        {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.status.error = format!("cannot start SNI host: {error}");
            state.revision = state.revision.wrapping_add(1);
        }
        Self { state, actions }
    }

    pub fn changed_since(&self, revision: u64) -> Option<SniSnapshot> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.revision == revision {
            return None;
        }
        let snapshot = state.clone();
        state.menu = None;
        Some(snapshot)
    }

    pub fn send(&self, action: SniAction) {
        if let Err(error) = self.actions.try_send(action) {
            eprintln!("smpl-bar: SNI action queue rejected: {error}");
        }
    }
}

#[derive(Clone, Debug, Default)]
struct WatcherState {
    items: BTreeMap<String, String>,
    hosts: HashSet<String>,
}

enum Registration {
    ItemAdded(String),
    ItemRemoved(String),
    HostAdded(String),
}

struct StatusNotifierWatcher {
    state: Arc<Mutex<WatcherState>>,
    registrations: mpsc::UnboundedSender<Registration>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl StatusNotifierWatcher {
    async fn register_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(ToString::to_string);
        let registration = canonical_registration(service, sender.as_deref());
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.items.len() >= MAX_ITEMS && !state.items.contains_key(&registration) {
                return Err(zbus::fdo::Error::LimitsExceeded(
                    "StatusNotifierItem limit reached".to_owned(),
                ));
            }
            state
                .items
                .insert(registration.clone(), registration.clone());
        }
        let _ = self
            .registrations
            .send(Registration::ItemAdded(registration.clone()));
        emitter
            .status_notifier_item_registered(&registration)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn unregister_status_notifier_item(
        &self,
        service: &str,
        #[zbus(header)] header: Header<'_>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let sender = header.sender().map(ToString::to_string);
        let registration = canonical_registration(service, sender.as_deref());
        {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .items
                .remove(&registration);
        }
        let _ = self
            .registrations
            .send(Registration::ItemRemoved(registration.clone()));
        emitter
            .status_notifier_item_unregistered(&registration)
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn register_status_notifier_host(
        &self,
        service: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let inserted = {
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .hosts
                .insert(service.to_owned())
        };
        if inserted {
            let _ = self
                .registrations
                .send(Registration::HostAdded(service.to_owned()));
        }
        emitter
            .status_notifier_host_registered()
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .items
            .values()
            .cloned()
            .collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .hosts
            .is_empty()
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &SignalEmitter<'_>,
        service: &str,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_host_registered(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

#[derive(Clone, Debug)]
struct RegisteredItem {
    registration: String,
    destination: String,
    path: String,
    owner: String,
    failures: u8,
}

struct SniWorker {
    connection: zbus::Connection,
    watcher_owned: bool,
    host_name: String,
    host_registered: bool,
    registrations: HashMap<String, RegisteredItem>,
    icons: IconResolver,
    state: Arc<Mutex<SniSnapshot>>,
    telemetry: Telemetry,
    last_error: String,
    watcher_state: Arc<Mutex<WatcherState>>,
    registration_tx: mpsc::UnboundedSender<Registration>,
}

impl SniWorker {
    async fn new(
        state: Arc<Mutex<SniSnapshot>>,
        telemetry: Telemetry,
        watcher_state: Arc<Mutex<WatcherState>>,
        registration_tx: mpsc::UnboundedSender<Registration>,
    ) -> Result<Self, String> {
        let probe = zbus::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .method_timeout(Duration::from_millis(500))
            .build()
            .await
            .map_err(|error| error.to_string())?;
        let watcher_exists = name_has_owner(&probe, WATCHER).await;
        let (connection, watcher_owned) = if watcher_exists {
            (probe, false)
        } else {
            let watcher = StatusNotifierWatcher {
                    state: watcher_state.clone(),
                registrations: registration_tx.clone(),
            };
            match zbus::connection::Builder::session()
                .map_err(|error| error.to_string())?
                .method_timeout(Duration::from_millis(500))
                .serve_at(WATCHER_PATH, watcher)
                .map_err(|error| error.to_string())?
                .name(WATCHER)
                .map_err(|error| error.to_string())?
                .build()
                .await
            {
                Ok(connection) => (connection, true),
                Err(_) => (probe, false),
            }
        };
        let host_name = format!("org.kde.StatusNotifierHost-{}", std::process::id());
        let host_registered = connection.request_name(host_name.as_str()).await.is_ok()
            && call_host_register(&connection, &host_name).await.is_ok();
        let worker = Self {
            connection,
            watcher_owned,
            host_name,
            host_registered,
            registrations: HashMap::new(),
            icons: IconResolver::from_env(),
            state,
            telemetry,
            last_error: String::new(),
            watcher_state,
            registration_tx,
        };
        Ok(worker)
    }

    async fn refresh_registry(&mut self) {
        let registered: Vec<String> = self
            .property::<Vec<String>>(
                WATCHER,
                WATCHER_PATH,
                WATCHER,
                "RegisteredStatusNotifierItems",
            )
            .await
            .unwrap_or_default();
        let keep: HashSet<_> = registered.iter().cloned().collect();
        self.registrations.retain(|key, _| keep.contains(key));
        for registration in registered.into_iter().take(MAX_ITEMS) {
            self.register(registration).await;
        }
        self.refresh_items().await;
        self.publish().await;
    }

    async fn register(&mut self, registration: String) {
        if self.registrations.len() >= MAX_ITEMS && !self.registrations.contains_key(&registration)
        {
            self.last_error = "SNI item limit reached".to_owned();
            return;
        }
        let (destination, path) = split_registration(&registration);
        let owner = name_owner(&self.connection, &destination)
            .await
            .unwrap_or_else(|| destination.clone());
        self.registrations
            .entry(registration.clone())
            .and_modify(|item| {
                item.destination = destination.clone();
                item.path = path.clone();
                item.owner = owner.clone();
            })
            .or_insert(RegisteredItem {
                registration,
                destination,
                path,
                owner,
                failures: 0,
            });
    }

    async fn unregister(&mut self, registration: String) {
        self.registrations.remove(&registration);
        self.publish().await;
    }

    async fn owner_changed(&mut self, name: String, old: String, new: String) {
        if name == WATCHER && new.is_empty() {
            self.watcher_owned = false;
            self.host_registered = false;
            self.last_error = "StatusNotifierWatcher disappeared; reacquiring".to_owned();
            if self.ensure_watcher().await {
                self.host_registered =
                    call_host_register(&self.connection, &self.host_name).await.is_ok();
                self.last_error.clear();
                self.refresh_registry().await;
            }
        } else if name == WATCHER {
            self.watcher_owned = false;
            self.host_registered =
                call_host_register(&self.connection, &self.host_name).await.is_ok();
            self.refresh_registry().await;
        }
        for item in self.registrations.values_mut() {
            if item.owner == old || item.destination == name {
                item.owner = new.clone();
                item.failures = 0;
            }
        }
        if new.is_empty() {
            self.registrations
                .retain(|_, item| item.destination != name || !item.owner.is_empty());
        } else if old != new {
            self.refresh_items().await;
        }
        self.publish_status();
    }

    async fn ensure_watcher(&mut self) -> bool {
        if name_has_owner(&self.connection, WATCHER).await {
            return true;
        }
        if self
            .connection
            .request_name(WATCHER)
            .await
            .is_err()
        {
            return false;
        }
        let watcher = StatusNotifierWatcher {
            state: self.watcher_state.clone(),
            registrations: self.registration_tx.clone(),
        };
        match self
            .connection
            .object_server()
            .at(WATCHER_PATH, watcher)
            .await
        {
            Ok(true) | Ok(false) => {
                self.watcher_owned = true;
                true
            }
            Err(error) => {
                self.last_error = format!("cannot restore StatusNotifierWatcher: {error}");
                false
            }
        }
    }

    async fn refresh_changed(&mut self, sender: Option<String>) {
        if let Some(sender) = sender {
            if self
                .registrations
                .values()
                .any(|item| item.owner == sender || item.destination == sender)
            {
                self.refresh_items().await;
            }
        }
    }

    async fn refresh_items(&mut self) {
        let mut items = Vec::new();
        let keys: Vec<_> = self.registrations.keys().cloned().collect();
        let deadline = Instant::now() + MAX_REFRESH_CYCLE;
        for key in keys {
            if Instant::now() >= deadline {
                self.last_error =
                    "SNI refresh budget reached; remaining items retry next cycle".to_owned();
                break;
            }
            let Some(registered) = self.registrations.get(&key).cloned() else {
                continue;
            };
            match self.read_item(&registered).await {
                Ok(item) => {
                    if let Some(current) = self.registrations.get_mut(&key) {
                        current.failures = 0;
                    }
                    if item.visible() {
                        items.push(item);
                    }
                }
                Err(error) => {
                    if let Some(current) = self.registrations.get_mut(&key) {
                        current.failures = current.failures.saturating_add(1);
                        if current.failures < 3 {
                            self.last_error = format!("{}: {error}", registered.registration);
                        } else {
                            self.last_error.clear();
                        }
                    }
                }
            }
        }
        items.sort_by(|left, right| left.title.cmp(&right.title).then(left.key.cmp(&right.key)));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.items = items;
        state.revision = state.revision.wrapping_add(1);
        self.publish_status_locked(&mut state);
    }

    async fn read_item(&self, registered: &RegisteredItem) -> Result<TrayItem, String> {
        let proxy = zbus::Proxy::new_owned(
            self.connection.clone(),
            registered.destination.clone(),
            registered.path.clone(),
            ITEM_INTERFACE.to_owned(),
        )
        .await
        .map_err(|error| error.to_string())?;
        let title = proxy
            .get_property::<String>("Title")
            .await
            .unwrap_or_default();
        let id = proxy.get_property::<String>("Id").await.unwrap_or_default();
        let category = proxy
            .get_property::<String>("Category")
            .await
            .unwrap_or_else(|_| "ApplicationStatus".to_owned());
        let status = proxy
            .get_property::<String>("Status")
            .await
            .unwrap_or_else(|_| "Active".to_owned());
        let attention = status == "NeedsAttention";
        let theme_path = proxy
            .get_property::<String>("IconThemePath")
            .await
            .ok()
            .filter(|path| Path::new(path).is_absolute())
            .map(PathBuf::from);
        let name_property = if attention {
            "AttentionIconName"
        } else {
            "IconName"
        };
        let pixmap_property = if attention {
            "AttentionIconPixmap"
        } else {
            "IconPixmap"
        };
        let icon = self
            .read_icon(
                &proxy,
                name_property,
                pixmap_property,
                theme_path.as_deref(),
            )
            .await;
        let overlay = self
            .read_icon(
                &proxy,
                "OverlayIconName",
                "OverlayIconPixmap",
                theme_path.as_deref(),
            )
            .await;
        let item_is_menu = proxy
            .get_property::<bool>("ItemIsMenu")
            .await
            .unwrap_or(false);
        let title = if title.trim().is_empty() {
            id.clone()
        } else {
            title
        };
        Ok(TrayItem {
            key: registered.registration.clone(),
            title,
            id,
            category,
            status,
            item_is_menu,
            icon,
            overlay,
        })
    }

    async fn read_icon(
        &self,
        proxy: &zbus::Proxy<'_>,
        name_property: &str,
        pixmap_property: &str,
        theme_path: Option<&Path>,
    ) -> TrayIcon {
        if let Ok(name) = proxy.get_property::<String>(name_property).await {
            if let Some(path) = theme_path.and_then(|root| resolve_in_theme(root, &name)) {
                return TrayIcon::File(path);
            }
            if let Some(path) = self.icons.resolve(&name) {
                return TrayIcon::File(path);
            }
        }
        let Ok(pixmaps) = proxy
            .get_property::<Vec<(i32, i32, Vec<u8>)>>(pixmap_property)
            .await
        else {
            return TrayIcon::Empty;
        };
        pixmaps
            .into_iter()
            .filter_map(|(width, height, data)| {
                decode_pixmap(width, height, data).map(|icon| (width.abs_diff(height), icon))
            })
            .max_by_key(|(shape, icon)| match icon {
                TrayIcon::Pixmap { width, height, .. } => (*width * *height, -i64::from(*shape)),
                _ => (0, 0),
            })
            .map(|(_, icon)| icon)
            .unwrap_or_default()
    }

    async fn action(&mut self, action: SniAction) {
        if matches!(
            &action,
            SniAction::Activate { .. } | SniAction::Secondary { .. }
        ) {
            if let Err(error) = smpl_shell::launch::close_popups() {
                self.last_error = error.to_string();
                self.publish_status();
                return;
            }
        }
        let key = match &action {
            SniAction::Activate { key, .. }
            | SniAction::Secondary { key, .. }
            | SniAction::Scroll { key, .. }
            | SniAction::MenuEvent { key, .. } => key.clone(),
        };
        let Some(item) = self.registrations.get(&key).cloned() else {
            self.last_error = "SNI item disappeared before activation".to_owned();
            self.publish_status();
            return;
        };
        let result = match action {
            SniAction::Activate { x, y, .. } => {
                if self.item_is_menu(&item).await.unwrap_or(false) {
                    self.open_menu(&item).await
                } else {
                    self.call_item(&item, "Activate", &(x, y)).await.map(|_| ())
                }
            }
            SniAction::Secondary { x, y, .. } => self.context_or_menu(&item, x, y).await,
            SniAction::Scroll {
                delta, horizontal, ..
            } => self
                .call_item(
                    &item,
                    "Scroll",
                    &(delta, if horizontal { "horizontal" } else { "vertical" }),
                )
                .await
                .map(|_| ()),
            SniAction::MenuEvent { item_id, .. } => self.menu_event(&item, item_id).await,
        };
        if let Err(error) = result {
            self.last_error = error;
            self.telemetry.emit(
                "sni_action_failed",
                [("error", serde_json::json!(self.last_error))],
            );
        } else {
            self.last_error.clear();
        }
        self.publish_status();
    }

    async fn call_item<B>(
        &self,
        item: &RegisteredItem,
        method: &str,
        body: &B,
    ) -> Result<zbus::message::Message, String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.connection
            .call_method(
                Some(item.destination.as_str()),
                item.path.as_str(),
                Some(ITEM_INTERFACE),
                method,
                body,
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn context_or_menu(
        &mut self,
        item: &RegisteredItem,
        x: i32,
        y: i32,
    ) -> Result<(), String> {
        if self.menu_path(item).await.is_some() {
            self.open_menu(item).await
        } else {
            self.call_item(item, "ContextMenu", &(x, y))
                .await
                .map(|_| ())
        }
    }

    async fn menu_path(&self, item: &RegisteredItem) -> Option<String> {
        let proxy = zbus::Proxy::new_owned(
            self.connection.clone(),
            item.destination.clone(),
            item.path.clone(),
            ITEM_INTERFACE.to_owned(),
        )
        .await
        .ok()?;
        proxy
            .get_property::<OwnedObjectPath>("Menu")
            .await
            .ok()
            .map(|path| path.to_string())
    }

    async fn item_is_menu(&self, item: &RegisteredItem) -> Result<bool, String> {
        let proxy = zbus::Proxy::new_owned(
            self.connection.clone(),
            item.destination.clone(),
            item.path.clone(),
            ITEM_INTERFACE.to_owned(),
        )
        .await
        .map_err(|error| error.to_string())?;
        proxy
            .get_property("ItemIsMenu")
            .await
            .map_err(|error| error.to_string())
    }

    async fn open_menu(&mut self, item: &RegisteredItem) -> Result<(), String> {
        let menu_path = self
            .menu_path(item)
            .await
            .ok_or_else(|| "item has no DBusMenu object".to_owned())?;
        let proxy = zbus::Proxy::new_owned(
            self.connection.clone(),
            item.destination.clone(),
            menu_path.clone(),
            DBUS_MENU.to_owned(),
        )
        .await
        .map_err(|error| error.to_string())?;
        let _: bool = proxy
            .call("AboutToShow", &(0_i32,))
            .await
            .map_err(|error| error.to_string())?;
        let (_, layout): (u32, MenuLayout) = proxy
            .call("GetLayout", &(0_i32, 3_i32, Vec::<String>::new()))
            .await
            .map_err(|error| error.to_string())?;
        let mut entries = Vec::new();
        flatten_menu(&layout, 0, &mut entries);
        let title = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .items
            .iter()
            .find(|candidate| candidate.key == item.registration)
            .map(|candidate| candidate.title.clone())
            .unwrap_or_else(|| item.registration.clone());
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.menu = Some(TrayMenu {
            item_key: item.registration.clone(),
            title,
            entries,
        });
        state.revision = state.revision.wrapping_add(1);
        Ok(())
    }

    async fn menu_event(&self, item: &RegisteredItem, item_id: i32) -> Result<(), String> {
        let menu_path = self
            .menu_path(item)
            .await
            .ok_or_else(|| "item has no DBusMenu object".to_owned())?;
        let proxy = zbus::Proxy::new_owned(
            self.connection.clone(),
            item.destination.clone(),
            menu_path,
            DBUS_MENU.to_owned(),
        )
        .await
        .map_err(|error| error.to_string())?;
        let _: () = proxy
            .call(
                "Event",
                &(
                    item_id,
                    "clicked",
                    zvariant::Value::from(0_i32),
                    unix_millis() as u32,
                ),
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn property<T>(
        &self,
        destination: &str,
        path: &str,
        interface: &str,
        property: &str,
    ) -> Result<T, zbus::Error>
    where
        T: TryFrom<OwnedValue>,
        T::Error: Into<zbus::Error>,
    {
        let proxy = zbus::Proxy::new(
            &self.connection,
            destination.to_owned(),
            path.to_owned(),
            interface.to_owned(),
        )
        .await?;
        proxy.get_property(property).await
    }

    async fn publish(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.revision = state.revision.wrapping_add(1);
        self.publish_status_locked(&mut state);
        self.telemetry.emit(
            "sni_items",
            [
                ("count", serde_json::json!(state.items.len())),
                ("host_registered", serde_json::json!(self.host_registered)),
            ],
        );
    }

    fn publish_status(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.publish_status_locked(&mut state);
        state.revision = state.revision.wrapping_add(1);
    }

    fn publish_status_locked(&self, state: &mut SniSnapshot) {
        state.status = SniStatus {
            watcher_owned: self.watcher_owned,
            host_registered: self.host_registered,
            item_count: self.registrations.len(),
            error: self.last_error.clone(),
        };
    }
}

async fn run(
    state: Arc<Mutex<SniSnapshot>>,
    mut actions: mpsc::Receiver<SniAction>,
    telemetry: Telemetry,
) {
    let (registration_tx, mut registrations) = mpsc::unbounded_channel();
    let watcher_state = Arc::new(Mutex::new(WatcherState::default()));
    let mut worker = match SniWorker::new(
        state.clone(),
        telemetry.clone(),
        watcher_state,
        registration_tx,
    )
    .await
    {
        Ok(worker) => worker,
        Err(error) => {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.status.error = error.clone();
            telemetry.emit("sni_unavailable", [("error", serde_json::json!(error))]);
            return;
        }
    };
    let rule = zbus::MatchRule::builder()
        .msg_type(MessageType::Signal)
        .build();
    let mut messages =
        match zbus::MessageStream::for_match_rule(rule, &worker.connection, Some(128)).await {
            Ok(messages) => messages,
            Err(error) => {
                worker.last_error = error.to_string();
                worker.publish_status();
                return;
            }
        };
    let mut refresh = tokio::time::interval(Duration::from_secs(30));
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    worker.refresh_registry().await;
    loop {
        tokio::select! {
            Some(action) = actions.recv() => worker.action(action).await,
            Some(registration) = registrations.recv() => {
                match registration {
                    Registration::ItemAdded(service) => {
                        worker.register(service).await;
                        worker.refresh_items().await;
                    }
                    Registration::ItemRemoved(service) => worker.unregister(service).await,
                    Registration::HostAdded(service) => {
                        if service == worker.host_name {
                            worker.host_registered = true;
                            worker.publish_status();
                        }
                    }
                }
            }
            Some(message) = messages.next() => {
                if let Ok(message) = message {
                    let interface = message.header().interface().map(ToString::to_string);
                    let member = message.header().member().map(ToString::to_string);
                    let sender = message.header().sender().map(ToString::to_string);
                    match (interface.as_deref(), member.as_deref()) {
                        (Some("org.freedesktop.DBus"), Some("NameOwnerChanged")) => {
                            if let Ok((name, old, new)) =
                                message.body().deserialize::<(String, String, String)>()
                            {
                                worker.owner_changed(name, old, new).await;
                            }
                        }
                        (Some(WATCHER), Some("StatusNotifierItemRegistered")) => {
                            if let Ok(service) = message.body().deserialize::<String>() {
                                worker.register(service).await;
                                worker.refresh_items().await;
                            }
                        }
                        (Some(WATCHER), Some("StatusNotifierItemUnregistered")) => {
                            if let Ok(service) = message.body().deserialize::<String>() {
                                worker.unregister(service).await;
                            }
                        }
                        (Some(ITEM_INTERFACE), Some(member))
                            if member.starts_with("New") || member == "NewToolTip" =>
                        {
                            worker.refresh_changed(sender).await;
                        }
                        _ => {}
                    }
                }
            }
            _ = refresh.tick() => {
                if !name_has_owner(&worker.connection, WATCHER).await {
                    worker.host_registered = false;
                    let _ = worker.ensure_watcher().await;
                }
                if !worker.host_registered {
                    worker.host_registered =
                        call_host_register(&worker.connection, &worker.host_name).await.is_ok();
                }
                worker.refresh_registry().await;
            }
        }
    }
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

async fn name_owner(connection: &zbus::Connection, name: &str) -> Option<String> {
    connection
        .call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetNameOwner",
            &name,
        )
        .await
        .ok()
        .and_then(|message| message.body().deserialize::<String>().ok())
}

async fn call_host_register(connection: &zbus::Connection, host: &str) -> zbus::Result<()> {
    connection
        .call_method(
            Some(WATCHER),
            WATCHER_PATH,
            Some(WATCHER),
            "RegisterStatusNotifierHost",
            &host,
        )
        .await
        .map(|_| ())
}

fn canonical_registration(service: &str, sender: Option<&str>) -> String {
    if service.starts_with('/') {
        return format!("{}{}", sender.unwrap_or_default(), service);
    }
    let (destination, path) = split_registration(service);
    format!("{destination}{path}")
}

fn split_registration(service: &str) -> (String, String) {
    if let Some(index) = service.find('/') {
        let destination = service[..index].to_owned();
        let path = service[index..].to_owned();
        (destination, path)
    } else {
        (service.to_owned(), DEFAULT_ITEM_PATH.to_owned())
    }
}

fn decode_pixmap(width: i32, height: i32, data: Vec<u8>) -> Option<TrayIcon> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels > MAX_ICON_PIXELS
        || data.len() != pixels.checked_mul(4)?
        || data.len() > MAX_ICON_BYTES
    {
        return None;
    }
    let mut rgba = Vec::with_capacity(data.len());
    let (chunks, remainder) = data.as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        let alpha = chunk[0];
        rgba.extend_from_slice(&[chunk[1], chunk[2], chunk[3], alpha]);
    }
    Some(TrayIcon::Pixmap {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

fn resolve_in_theme(root: &Path, name: &str) -> Option<PathBuf> {
    if name.contains('/') || matches!(name, "." | "..") {
        return None;
    }
    let explicit = Path::new(name).extension().is_some();
    for extension in if explicit {
        &[""][..]
    } else {
        &["svg", "png"][..]
    } {
        let candidate = if extension.is_empty() {
            root.join(name)
        } else {
            root.join(format!("{name}.{extension}"))
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

#[derive(
    Clone, Debug, Deserialize, Serialize, Type, zbus::zvariant::Value, zbus::zvariant::OwnedValue,
)]
#[zvariant(crate = "zbus::zvariant")]
struct MenuLayout {
    id: i32,
    properties: HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

fn flatten_menu(root: &MenuLayout, level: usize, entries: &mut Vec<TrayMenuEntry>) {
    if level >= 8 {
        return;
    }
    for child in &root.children {
        if entries.len() >= MAX_MENU_ITEMS {
            return;
        }
        let Ok(child) = child.try_clone().and_then(MenuLayout::try_from) else {
            continue;
        };
        let visible = property_bool(&child.properties, "visible").unwrap_or(true);
        if !visible {
            continue;
        }
        let type_name = property_string(&child.properties, "type").unwrap_or_default();
        let separator = type_name == "separator";
        let label = property_string(&child.properties, "label")
            .unwrap_or_default()
            .replace('_', "");
        let enabled = property_bool(&child.properties, "enabled").unwrap_or(true);
        let checked = property_i32(&child.properties, "toggle-state").unwrap_or(0) == 1;
        entries.push(TrayMenuEntry {
            item_id: child.id,
            label,
            enabled,
            checked,
            separator,
            level,
        });
        flatten_menu(&child, level + 1, entries);
    }
}

fn property_string(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<String> {
    let value = properties.get(name)?;
    <&str>::try_from(value).ok().map(str::to_owned)
}

fn property_bool(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<bool> {
    properties.get(name)?.try_clone().ok()?.try_into().ok()
}

fn property_i32(properties: &HashMap<String, OwnedValue>, name: &str) -> Option<i32> {
    properties.get(name)?.try_clone().ok()?.try_into().ok()
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_supports_bus_names_object_paths_and_sender_paths() {
        assert_eq!(
            split_registration("org.example.Tray/Item"),
            ("org.example.Tray".to_owned(), "/Item".to_owned())
        );
        assert_eq!(canonical_registration("/Menu", Some(":1.42")), ":1.42/Menu");
        assert_eq!(
            canonical_registration("org.example.Tray", None),
            "org.example.Tray/StatusNotifierItem"
        );
    }

    #[test]
    fn pixmap_decoder_bounds_allocation_and_converts_argb_to_rgba() {
        assert_eq!(
            decode_pixmap(1, 1, vec![0x80, 0x10, 0x20, 0x30]),
            Some(TrayIcon::Pixmap {
                width: 1,
                height: 1,
                rgba: vec![0x10, 0x20, 0x30, 0x80],
            })
        );
        assert!(decode_pixmap(300, 300, vec![]).is_none());
        assert!(decode_pixmap(-1, 1, vec![]).is_none());
    }

    #[test]
    fn menu_properties_keep_bounded_variants_only() {
        let root = MenuLayout {
            id: 0,
            properties: HashMap::new(),
            children: (0..MAX_MENU_ITEMS + 10)
                .map(|id| {
                    MenuLayout {
                        id: id as i32,
                        properties: HashMap::new(),
                        children: Vec::new(),
                    }
                    .try_into()
                    .unwrap()
                })
                .collect(),
        };
        let mut entries = Vec::new();
        flatten_menu(&root, 0, &mut entries);
        assert_eq!(entries.len(), MAX_MENU_ITEMS);
    }
}
