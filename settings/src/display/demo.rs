//! Simulated displays for `settings --demo` only. Nothing is saved and the
//! status line says so; there is no automatic fallback to this backend.

use std::sync::Mutex;

use super::backend::{ApplyOutcome, ApplyReport, DisplayBackend, Snapshot, Target};
use super::monitor::{Mode, Output};
use super::verify;

struct State {
    outputs: Vec<Output>,
    workspace_one: Option<String>,
}

pub struct DemoBackend {
    state: Mutex<State>,
}

fn output(
    id: i64,
    name: &str,
    description: &str,
    modes: &[(i32, i32, f64)],
    position: (i32, i32),
    scale: f64,
    transform: i32,
) -> Output {
    let modes: Vec<Mode> = modes
        .iter()
        .map(|&(width, height, refresh)| Mode {
            width,
            height,
            refresh,
        })
        .collect();
    Output {
        id,
        name: name.into(),
        description: description.into(),
        make: "Demo".into(),
        model: description.into(),
        serial: String::new(),
        mode: modes[0],
        x: position.0,
        y: position.1,
        reported_scale: scale,
        transform,
        disabled: false,
        mirror_of: None,
        available_modes: modes,
    }
}

impl DemoBackend {
    pub fn new() -> Self {
        let outputs = vec![
            output(
                0,
                "DP-1",
                "Demo 27\" 4K Monitor",
                &[
                    (3840, 2160, 144.0),
                    (3840, 2160, 60.0),
                    (2560, 1440, 165.0),
                    (1920, 1080, 60.0),
                ],
                (0, 0),
                1.5,
                // Portrait, so the demo exercises rotated rows and labels.
                1,
            ),
            output(
                1,
                "HDMI-A-1",
                "Demo 24\" 1080p Monitor",
                &[(1920, 1080, 60.0), (1920, 1080, 59.94), (1280, 720, 60.0)],
                (1440, 0),
                1.0,
                0,
            ),
            output(
                2,
                "eDP-1",
                "Demo 14\" Laptop Display",
                &[(2880, 1800, 120.0), (2880, 1800, 60.0), (1920, 1200, 60.0)],
                (1440, 1080),
                2.0,
                0,
            ),
        ];
        Self {
            state: Mutex::new(State {
                outputs,
                workspace_one: Some("DP-1".into()),
            }),
        }
    }
}

impl Default for DemoBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayBackend for DemoBackend {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let state = self.state.lock().unwrap();
        Ok(Snapshot {
            outputs: state.outputs.clone(),
            conf: Ok(None),
            workspace_one: state.workspace_one.clone(),
            workspace_policy: false,
        })
    }

    fn apply(&self, baseline: u64, targets: &[Target]) -> ApplyOutcome {
        let before = match self.snapshot() {
            Ok(snapshot) => snapshot,
            Err(message) => {
                return ApplyOutcome::Failed {
                    message,
                    snapshot: None,
                }
            }
        };
        if before.fingerprint() != baseline {
            return ApplyOutcome::Stale;
        }
        {
            let mut state = self.state.lock().unwrap();
            for target in targets {
                if let Some(output) = state.outputs.iter_mut().find(|o| o.name == target.name) {
                    output.mode = target.mode;
                    output.x = target.x;
                    output.y = target.y;
                    output.reported_scale = (target.scale * 100.0).round() / 100.0;
                    output.transform = target.transform;
                }
            }
        }
        match self.snapshot() {
            Ok(after) => ApplyOutcome::Finished(ApplyReport {
                differences: verify::differences(targets, &after.outputs),
                warnings: vec!["Demo mode: nothing was saved.".into()],
                snapshot: after,
            }),
            Err(message) => ApplyOutcome::Failed {
                message,
                snapshot: None,
            },
        }
    }

    fn use_saved(&self) -> ApplyOutcome {
        match self.snapshot() {
            Ok(snapshot) => ApplyOutcome::Finished(ApplyReport {
                snapshot,
                differences: Vec::new(),
                warnings: vec!["Demo mode: nothing was saved.".into()],
            }),
            Err(message) => ApplyOutcome::Failed {
                message,
                snapshot: None,
            },
        }
    }

    fn move_workspace_one(&self, connector: &str) -> Result<(), String> {
        self.state.lock().unwrap().workspace_one = Some(connector.into());
        Ok(())
    }

    fn identify(&self, labels: &[(String, String)]) -> Result<(), String> {
        for (connector, label) in labels {
            eprintln!("[demo] identify {connector}: {label}");
        }
        Ok(())
    }

    fn is_demo(&self) -> bool {
        true
    }
}
