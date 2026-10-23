//! One persistent bus connection owns identify/learning until the tab closes.
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetupStatus {
    pub active: bool,
    pub phase: String,
    pub target: String,
    pub step: u32,
    pub total: u32,
    pub captured: u32,
    pub rows: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub api_version: u32,
    pub connected: bool,
    pub ready: bool,
    pub error: String,
    pub notice: String,
    pub config_path: String,
    pub learned: Vec<String>,
    pub unlearned: Vec<String>,
    pub identify: bool,
    pub learning: String,
    pub last_slot: String,
    pub last_input: String,
    pub sequence: u64,
    pub focus_warning: String,
    #[serde(default)]
    pub calibrating: bool,
    #[serde(default)]
    pub guided_setup: bool,
    #[serde(default)]
    pub setup: SetupStatus,
    #[serde(default)]
    pub kdenlive_api: bool,
    #[serde(default)]
    pub kdenlive_available: bool,
    #[serde(default)]
    pub kdenlive_profile: bool,
}

impl Status {
    pub fn summary(&self) -> String {
        let state = if self.ready {
            "Razer Naga V2 HyperSpeed connected. Button remapping is active."
        } else if self.connected {
            "Razer Naga V2 HyperSpeed connected, but remapping is not ready."
        } else {
            "Razer Naga V2 HyperSpeed is not connected."
        };
        let pending = if self.unlearned.is_empty() {
            String::new()
        } else {
            format!(
                "Assignments waiting for button learning: {}",
                self.unlearned.join(", ")
            )
        };
        let calibration = if self.ready
            && !self.calibrating
            && !self.setup.active
            && !(1..=12).all(|n| self.learned.contains(&format!("grid-{n}")))
        {
            "Side-grid positions are not calibrated. Click Set up this mouse above the diagram to capture actual controls, or Calibrate side grid to learn just buttons 1 through 12."
        } else {
            ""
        };
        let plugin = if self.kdenlive_profile {
            if self.kdenlive_available {
                "Kdenlive control surface API is available for the focused app."
            } else {
                "Kdenlive API profile active, but its interface is not available. Enable Control surface interface in Kdenlive; no shortcut fallback is used."
            }
        } else {
            ""
        };
        [
            state,
            &self.error,
            &self.focus_warning,
            &self.notice,
            &self.last_input,
            &pending,
            calibration,
            plugin,
        ]
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
    }
}

pub enum Command {
    Poll,
    Identify(bool),
    Learn(String),
    Calibrate,
    StartSetup,
    ArmSetup,
    NextSetup,
    SkipSetup,
    RetrySetup,
    SaveSetup,
    CancelSetup,
    Cancel,
    Leave,
}

type Reply = Box<dyn FnOnce(Result<Status, String>) + Send>;
struct Request(Command, Reply);

fn worker() -> &'static Sender<Request> {
    static WORKER: OnceLock<Sender<Request>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Request>();
        std::thread::spawn(move || {
            let mut connection = None;
            for Request(command, reply) in receive {
                let result = (|| {
                    if connection.is_none() {
                        connection = Some(
                            zbus::blocking::connection::Builder::session()
                                .map_err(|e| e.to_string())?
                                .method_timeout(Duration::from_secs(2))
                                .build()
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    let conn = connection
                        .as_ref()
                        .ok_or("Mouse D-Bus connection unavailable")?;
                    let proxy = zbus::blocking::Proxy::new(
                        conn,
                        "org.smplos.MouseRemapper1",
                        "/org/smplos/MouseRemapper1",
                        "org.smplos.MouseRemapper1",
                    )
                    .map_err(|e| e.to_string())?;
                    match command {
                        Command::Poll => {}
                        Command::Identify(on) => proxy
                            .call::<_, _, ()>("SetIdentify", &(on,))
                            .map_err(|e| e.to_string())?,
                        Command::Learn(slot) => proxy
                            .call::<_, _, ()>("Learn", &(slot,))
                            .map_err(|e| e.to_string())?,
                        Command::Calibrate => proxy
                            .call::<_, _, ()>("Calibrate", &())
                            .map_err(|e| e.to_string())?,
                        Command::StartSetup => proxy.call::<_, _, ()>("StartSetup", &()).map_err(|e| e.to_string())?,
                        Command::ArmSetup => proxy.call::<_, _, ()>("ArmSetup", &()).map_err(|e| e.to_string())?,
                        Command::NextSetup => proxy.call::<_, _, ()>("NextSetup", &()).map_err(|e| e.to_string())?,
                        Command::SkipSetup => proxy.call::<_, _, ()>("SkipSetup", &()).map_err(|e| e.to_string())?,
                        Command::RetrySetup => proxy.call::<_, _, ()>("RetrySetup", &()).map_err(|e| e.to_string())?,
                        Command::SaveSetup => proxy.call::<_, _, ()>("SaveSetup", &()).map_err(|e| e.to_string())?,
                        Command::CancelSetup => proxy.call::<_, _, ()>("CancelSetup", &()).map_err(|e| e.to_string())?,
                        Command::Cancel => proxy
                            .call::<_, _, ()>("CancelLearn", &())
                            .map_err(|e| e.to_string())?,
                        Command::Leave => proxy
                            .call::<_, _, ()>("SetIdentify", &(false,))
                            .map_err(|e| e.to_string())?,
                    }
                    let text: String = proxy.call("Status", &()).map_err(|e| e.to_string())?;
                    let status: Status = serde_json::from_str(&text)
                        .map_err(|e| format!("Invalid mouse status: {e}"))?;
                    if status.api_version != 1 {
                        return Err(
                            "Mouse daemon API is not supported by this Settings version".into()
                        );
                    }
                    if !status.kdenlive_api {
                        return Err("Installed mouse daemon lacks shared Kdenlive API support; install the matching Settings and mouse-remapperd binaries".into());
                    }
                    if !status.guided_setup {
                        return Err("Installed mouse daemon lacks guided setup support; install the matching Settings and mouse-remapperd binaries".into());
                    }
                    Ok(status)
                })();
                reply(result);
            }
        });
        send
    })
}

pub fn request(command: Command, reply: impl FnOnce(Result<Status, String>) + Send + 'static) {
    if worker().send(Request(command, Box::new(reply))).is_err() {
        eprintln!("settings: mouse remapper worker stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_is_never_inferred_from_connection_alone() {
        let status = Status {
            connected: true,
            error: "input access denied".into(),
            ..Status::default()
        };
        let summary = status.summary();
        assert!(summary.contains("not ready"));
        assert!(summary.contains("input access denied"));
        assert!(!summary.contains("remapping is active"));
    }

    #[test]
    fn status_requires_the_wire_contract() {
        assert!(serde_json::from_str::<Status>(r#"{"connected":true}"#).is_err());
    }

    #[test]
    fn compiled_support_and_runtime_plugin_availability_are_distinct() {
        let mut status = Status {
            kdenlive_api: true,
            kdenlive_profile: true,
            ..Status::default()
        };
        assert!(status.summary().contains("interface is not available"));
        assert!(status.summary().contains("no shortcut fallback"));
        status.kdenlive_available = true;
        assert!(status.summary().contains("API is available"));
        assert!(!status.summary().contains("interface is not available"));
    }

    #[test]
    fn uncalibrated_grid_has_an_explicit_setup_instruction() {
        let mut status = Status {
            ready: true,
            ..Status::default()
        };
        assert!(status.summary().contains("Click Set up this mouse above the diagram"));
        status.learned = (1..=11).map(|n| format!("grid-{n}")).collect();
        status.learned.push("btn-back".into());
        assert!(status.summary().contains("Side-grid positions are not calibrated"));
        status.learned.push("grid-12".into());
        assert!(!status.summary().contains("Side-grid positions are not calibrated"));
        status.learned.clear();
        status.calibrating = true;
        assert!(!status.summary().contains("Click Set up"));
    }
}
