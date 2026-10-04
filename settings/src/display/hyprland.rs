//! The Hyprland backend: read-only snapshots, write-then-verify Apply,
//! provider-aware dispatches and per-screen identify labels.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::backend::{ApplyOutcome, ApplyReport, DisplayBackend, Snapshot, Target};
use super::conf::{self, ConfPath, Planned};
use super::hyprctl::{self, Dispatch, Provider, Runner};
use super::layout::Rect;
use super::monitor::{self, Kind, Output};
use super::verify;

pub struct HyprlandBackend {
    runner: Arc<dyn Runner>,
    home: PathBuf,
    provider: Mutex<Option<Provider>>,
    /// How long to wait for Hyprland to apply a reloaded layout.
    settle: Duration,
    poll: Duration,
    stamp: fn() -> String,
}

#[derive(Deserialize)]
struct Workspace {
    id: i64,
    #[serde(default)]
    monitor: String,
}

#[derive(Deserialize)]
struct Client {
    #[serde(default)]
    address: String,
    #[serde(default = "mapped")]
    mapped: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    at: (i32, i32),
    #[serde(default)]
    size: (i32, i32),
    #[serde(default)]
    floating: bool,
    #[serde(default)]
    monitor: i64,
}

fn mapped() -> bool {
    true
}

impl HyprlandBackend {
    pub fn new(runner: Arc<dyn Runner>, home: PathBuf) -> Self {
        Self {
            runner,
            home,
            provider: Mutex::new(None),
            settle: Duration::from_secs(3),
            poll: Duration::from_millis(150),
            stamp: conf::local_stamp,
        }
    }

    fn hyprctl(&self, args: &[&str]) -> Result<String, String> {
        hyprctl::hyprctl(&*self.runner, args)
    }

    fn outputs(&self) -> Result<Vec<Output>, String> {
        monitor::parse_outputs(&self.hyprctl(&["-j", "monitors", "all"])?)
    }

    fn conf_path(&self) -> Result<ConfPath, String> {
        ConfPath::for_home(&self.home)
    }

    /// Cached per Settings process; a Hyprland instance keeps its provider.
    fn provider(&self) -> Result<Provider, String> {
        let mut cached = self.provider.lock().unwrap();
        if let Some(provider) = *cached {
            return Ok(provider);
        }
        let provider = hyprctl::parse_provider(&self.hyprctl(&["systeminfo"])?)?;
        *cached = Some(provider);
        Ok(provider)
    }

    fn workspace_one(&self) -> Option<String> {
        let json = self
            .hyprctl(&["-j", "workspaces"])
            .map_err(|error| eprintln!("[settings] workspaces unavailable: {error}"))
            .ok()?;
        let workspaces: Vec<Workspace> = serde_json::from_str(&json)
            .map_err(|error| eprintln!("[settings] unexpected workspaces output: {error}"))
            .ok()?;
        workspaces
            .into_iter()
            .find(|workspace| workspace.id == 1 && !workspace.monitor.is_empty())
            .map(|workspace| workspace.monitor)
    }

    fn workspace_policy(&self) -> bool {
        self.runner
            .run("workspace-ctl", &["enabled"])
            .is_ok_and(|output| output.success)
    }

    fn bar(&self, action: &str) -> Result<(), String> {
        match self.runner.run("bar-ctl", &[action]) {
            Ok(output) if output.success => Ok(()),
            Ok(output) => Err(format!("bar-ctl {action} failed: {}", output.stderr.trim())),
            Err(error) => {
                eprintln!("[settings] {error}");
                Ok(())
            }
        }
    }

    fn config_errors(&self) -> Result<Vec<String>, String> {
        let json = self.hyprctl(&["-j", "configerrors"])?;
        let errors: Vec<String> = serde_json::from_str(&json)
            .map_err(|error| format!("unexpected configerrors output: {error}"))?;
        Ok(errors
            .into_iter()
            .map(|error| error.trim().to_string())
            .filter(|error| !error.is_empty())
            .collect())
    }

    /// Centers floating windows whose center is on no display. Tiled windows
    /// are re-laid out by Hyprland itself (moving one to the workspace it is
    /// already on is a no-op in 0.56), and in the scrolling layout tiled
    /// windows outside the viewport are expected.
    fn recover_offscreen(&self, provider: Provider, outputs: &[Output]) -> Vec<String> {
        // Every enabled output counts as a screen, including virtual (XR)
        // ones, so their windows are never pulled onto a physical display.
        let mut enabled: Vec<&Output> = outputs
            .iter()
            .filter(|output| output.kind() != Kind::Disabled)
            .collect();
        // The first physical display hosts windows whose output is unknown.
        enabled.sort_by_key(|output| !output.is_managed());
        let screens: Vec<(i64, Rect)> = enabled
            .into_iter()
            .map(|output| {
                let (w, h) = output.logical_size();
                (
                    output.id,
                    Rect {
                        x: output.x,
                        y: output.y,
                        w,
                        h,
                    },
                )
            })
            .collect();
        let Some(&(_, fallback)) = screens.first() else {
            return Vec::new();
        };
        let clients: Vec<Client> = match self
            .hyprctl(&["-j", "clients"])
            .and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
        {
            Ok(clients) => clients,
            Err(error) => return vec![format!("Off-screen windows weren't checked: {error}")],
        };
        let mut errors = Vec::new();
        for client in clients {
            if client.address.is_empty() || !client.mapped || client.hidden || !client.floating {
                continue;
            }
            let center_x = client.at.0 as i64 + client.size.0 as i64 / 2;
            let center_y = client.at.1 as i64 + client.size.1 as i64 / 2;
            if screens
                .iter()
                .any(|(_, screen)| screen.contains(center_x, center_y))
            {
                continue;
            }
            let screen = screens
                .iter()
                .find(|(id, _)| *id == client.monitor)
                .map_or(fallback, |(_, screen)| *screen);
            let action = Dispatch::CenterWindow {
                address: client.address,
                x: screen.x + (screen.w - client.size.0) / 2,
                y: screen.y + (screen.h - client.size.1) / 2,
            };
            if let Err(error) = hyprctl::dispatch(&*self.runner, provider, &action) {
                errors.push(format!(
                    "A window couldn't be moved back on screen: {error}"
                ));
            }
        }
        errors
    }

    /// Stops the bar, reloads Hyprland, waits until `settled` accepts the
    /// live outputs (or the timeout), restarts the bar, recovers off-screen
    /// windows and collects config errors. `Err` only when reload failed.
    fn reload_and_settle(
        &self,
        settled: &dyn Fn(&[Output]) -> bool,
    ) -> Result<Vec<String>, String> {
        let mut warnings = Vec::new();
        if let Err(error) = self.bar("stop") {
            eprintln!("[settings] {error}");
        }
        let reloaded = hyprctl::hyprctl_ok(&*self.runner, &["reload"]);
        let mut latest = None;
        if reloaded.is_ok() {
            let deadline = Instant::now() + self.settle;
            loop {
                match self.outputs() {
                    Ok(outputs) => {
                        let done = settled(&outputs);
                        latest = Some(outputs);
                        if done {
                            break;
                        }
                    }
                    Err(error) => eprintln!("[settings] waiting for displays: {error}"),
                }
                if Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(self.poll);
            }
        }
        if let Err(error) = self.bar("start") {
            warnings.push(format!("The taskbar couldn't be restarted ({error})."));
        }
        reloaded?;
        if let Some(outputs) = latest {
            match self.provider() {
                Ok(provider) => warnings.extend(self.recover_offscreen(provider, &outputs)),
                Err(error) => warnings.push(format!("Off-screen windows weren't checked: {error}")),
            }
        }
        match self.config_errors() {
            Ok(errors) => warnings.extend(
                errors
                    .into_iter()
                    .map(|e| format!("Hyprland config error: {e}")),
            ),
            Err(error) => {
                warnings.push(format!("Hyprland config errors couldn't be read: {error}"))
            }
        }
        Ok(warnings)
    }
}

impl DisplayBackend for HyprlandBackend {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let outputs = self.outputs()?;
        let conf = self.conf_path().and_then(|path| path.read());
        Ok(Snapshot {
            outputs,
            conf,
            workspace_one: self.workspace_one(),
            workspace_policy: self.workspace_policy(),
        })
    }

    fn apply(&self, baseline: u64, targets: &[Target]) -> ApplyOutcome {
        let before = match self.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return ApplyOutcome::Failed {
                    message: format!("Couldn't read the current displays: {error}"),
                    snapshot: None,
                }
            }
        };
        if before.fingerprint() != baseline {
            return ApplyOutcome::Stale;
        }
        if targets
            .iter()
            .any(|target| !before.managed().any(|output| output.name == target.name))
        {
            return ApplyOutcome::Stale;
        }
        let planned: Vec<Planned> = targets
            .iter()
            .filter_map(|target| {
                let output = before.managed().find(|output| output.name == target.name)?;
                Some(Planned {
                    selector: conf::choose_selector(output, &before.outputs),
                    target: target.clone(),
                })
            })
            .collect();
        let saved = self.conf_path().and_then(|path| {
            let previous = path.read()?;
            let text = conf::merge(previous.as_deref().unwrap_or(""), &planned, &before.outputs);
            conf::replace(&path, previous.as_deref(), &text, &(self.stamp)())
        });
        if let Err(error) = saved {
            return ApplyOutcome::Failed {
                message: format!("Couldn't save display settings: {error}"),
                snapshot: None,
            };
        }
        let settled = |outputs: &[Output]| verify::differences(targets, outputs).is_empty();
        let warnings = match self.reload_and_settle(&settled) {
            Ok(warnings) => warnings,
            Err(error) => {
                return ApplyOutcome::Failed {
                    message: format!("Saved, but Hyprland didn't reload it: {error}"),
                    snapshot: self.snapshot().ok(),
                }
            }
        };
        match self.snapshot() {
            Ok(after) => ApplyOutcome::Finished(ApplyReport {
                differences: verify::differences(targets, &after.outputs),
                warnings,
                snapshot: after,
            }),
            Err(error) => ApplyOutcome::Failed {
                message: format!("Saved and reloaded, but the result couldn't be checked: {error}"),
                snapshot: None,
            },
        }
    }

    fn use_saved(&self) -> ApplyOutcome {
        let saved = match self.conf_path().and_then(|path| path.read()) {
            Ok(text) => text.unwrap_or_default(),
            Err(error) => {
                return ApplyOutcome::Failed {
                    message: format!("Couldn't read the saved display settings: {error}"),
                    snapshot: None,
                }
            }
        };
        let settled = |outputs: &[Output]| verify::drift(outputs, &saved).is_empty();
        let warnings = match self.reload_and_settle(&settled) {
            Ok(warnings) => warnings,
            Err(error) => {
                return ApplyOutcome::Failed {
                    message: format!("Hyprland didn't reload the saved settings: {error}"),
                    snapshot: self.snapshot().ok(),
                }
            }
        };
        match self.snapshot() {
            Ok(after) => ApplyOutcome::Finished(ApplyReport {
                differences: verify::drift(&after.outputs, after.conf_text())
                    .iter()
                    .map(verify::Drift::sentence)
                    .collect(),
                warnings,
                snapshot: after,
            }),
            Err(error) => ApplyOutcome::Failed {
                message: format!("Reloaded, but the result couldn't be checked: {error}"),
                snapshot: None,
            },
        }
    }

    fn move_workspace_one(&self, connector: &str) -> Result<(), String> {
        let action = Dispatch::MoveWorkspaceToMonitor {
            workspace: "1".into(),
            monitor: connector.into(),
        };
        hyprctl::dispatch(&*self.runner, self.provider()?, &action)
    }

    fn identify(&self, labels: &[(String, String)]) -> Result<(), String> {
        let config = self.home.join(".config/eww").to_string_lossy().into_owned();
        let config = config.as_str();
        let mut errors = Vec::new();
        for (index, (connector, label)) in labels.iter().take(4).enumerate() {
            let window = format!("mon-identify-{index}");
            let run = |args: &[&str]| -> Result<(), String> {
                let output = self.runner.run("eww", args)?;
                if output.success {
                    Ok(())
                } else {
                    Err(format!("eww {} failed: {}", args[2], output.stderr.trim()))
                }
            };
            let variable = format!("mon-id-label-{index}={label}");
            let shown = run(&["-c", config, "update", &variable]).and_then(|()| {
                // A window that is still open would stay on its old screen.
                let _ = run(&["-c", config, "close", &window]);
                run(&[
                    "-c",
                    config,
                    "open",
                    &window,
                    "--screen",
                    connector,
                    "--duration",
                    "5s",
                ])
            });
            if let Err(error) = shown {
                errors.push(format!("{connector}: {error}"));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::hyprctl::tests::{failure, reply, MockRunner};
    use crate::display::monitor::tests::output_json;
    use std::fs;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    struct Home(PathBuf);

    impl Home {
        fn new(conf: Option<&str>) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "settings-display-backend-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join(".config/hypr")).unwrap();
            if let Some(conf) = conf {
                fs::write(path.join(".config/hypr/monitors.conf"), conf).unwrap();
            }
            Self(path)
        }

        fn conf(&self) -> String {
            fs::read_to_string(self.0.join(".config/hypr/monitors.conf")).unwrap()
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn monitors_json(dp3_transform: i32) -> String {
        format!(
            "[{},{}]",
            output_json(
                0,
                "HDMI-A-1",
                "Lenovo Group Limited LEN P27h-10 0x01010101",
                (2560, 1440, 59.951),
                (0, 0),
                1.0,
                0,
                &["2560x1440@59.95Hz"]
            ),
            output_json(
                1,
                "DP-3",
                "Dell Inc. DELL P2412H KG49T35D59GU",
                (1920, 1080, 60.0),
                (2560, 0),
                1.0,
                dp3_transform,
                &["1920x1080@60.00Hz"]
            )
        )
    }

    const SAVED: &str = "monitor = HDMI-A-1, 2560x1440@59.95, 0x0, 1.00, transform, 0\nmonitor = DP-3, 1920x1080@60.00, 2560x0, 1.00, transform, 1\n";

    /// A fake Hyprland whose DP-3 transform becomes `after` once reloaded.
    fn fake(
        provider: &'static str,
        before: i32,
        after: i32,
        reload_reply: &'static str,
    ) -> Arc<MockRunner> {
        let reloaded = Arc::new(AtomicUsize::new(0));
        Arc::new(MockRunner::new(move |program, args| {
            match (program, args) {
                ("hyprctl", ["-j", "monitors", "all"]) => {
                    let transform = if reloaded.load(Ordering::SeqCst) > 0 {
                        after
                    } else {
                        before
                    };
                    reply(&monitors_json(transform))
                }
                ("hyprctl", ["reload"]) => {
                    reloaded.fetch_add(1, Ordering::SeqCst);
                    reply(reload_reply)
                }
                ("hyprctl", ["-j", "workspaces"]) => {
                    reply(r#"[{"id": 1, "monitor": "DP-3"}, {"id": 2, "monitor": "HDMI-A-1"}]"#)
                }
                ("hyprctl", ["-j", "configerrors"]) => reply("[\n\t\"\"\n]"),
                ("hyprctl", ["-j", "clients"]) => reply(
                    r#"[{"address": "0xaaa", "mapped": true, "hidden": false, "at": [9000, 50], "size": [400, 300], "workspace": {"id": 3, "name": "3"}, "floating": true, "monitor": 1},
                    {"address": "0xbbb", "mapped": true, "hidden": false, "at": [-5000, 50], "size": [400, 300], "workspace": {"id": 4, "name": "4"}, "floating": false, "monitor": 0},
                    {"address": "0xccc", "mapped": true, "hidden": false, "at": [100, 100], "size": [400, 300], "workspace": {"id": 1, "name": "1"}, "floating": true, "monitor": 0},
                    {"address": "0xddd", "mapped": true, "hidden": true, "at": [-9000, 0], "size": [10, 10], "workspace": {"id": -98, "name": "special:x"}, "floating": true, "monitor": 0}]"#,
                ),
                ("hyprctl", ["systeminfo"]) => {
                    reply(&format!("Hyprland 0.56.0\n\nconfigProvider: {provider}\n"))
                }
                ("hyprctl", ["dispatch", _]) => reply("ok"),
                ("workspace-ctl", ["enabled"]) => failure(""),
                ("bar-ctl", [_]) => reply(""),
                ("eww", _) => reply(""),
                _ => Err(format!("unexpected command {program} {args:?}")),
            }
        }))
    }

    fn backend(runner: Arc<MockRunner>, home: &Home) -> HyprlandBackend {
        HyprlandBackend {
            settle: Duration::from_millis(60),
            poll: Duration::from_millis(1),
            stamp: || "20261003-120000".into(),
            ..HyprlandBackend::new(runner, home.0.clone())
        }
    }

    fn targets(snapshot: &Snapshot, dp3_transform: i32) -> Vec<Target> {
        snapshot
            .managed()
            .map(|o| Target {
                name: o.name.clone(),
                mode: o.mode,
                x: o.x,
                y: o.y,
                scale: o.scale(),
                transform: if o.name == "DP-3" {
                    dp3_transform
                } else {
                    o.transform
                },
            })
            .collect()
    }

    #[test]
    fn snapshot_reads_all_outputs_file_workspace_one_and_policy() {
        let home = Home::new(Some(SAVED));
        let runner = fake("lua", 1, 1, "ok");
        let snapshot = backend(runner.clone(), &home).snapshot().unwrap();
        assert_eq!(snapshot.outputs.len(), 2);
        assert_eq!(snapshot.outputs[1].transform, 1);
        assert_eq!(snapshot.conf_text(), SAVED);
        assert_eq!(snapshot.workspace_one.as_deref(), Some("DP-3"));
        assert!(!snapshot.workspace_policy);
        assert!(runner
            .calls()
            .contains(&"hyprctl -j monitors all".to_string()));
        assert!(runner
            .calls()
            .iter()
            .all(|call| !call.contains("reload") && !call.contains("dispatch")));
    }

    #[test]
    fn apply_writes_reloads_verifies_and_recovers_windows_with_lua_syntax() {
        let home = Home::new(Some(SAVED));
        let runner = fake("lua", 1, 0, "ok");
        let backend = backend(runner.clone(), &home);
        let before = backend.snapshot().unwrap();
        let ApplyOutcome::Finished(report) =
            backend.apply(before.fingerprint(), &targets(&before, 0))
        else {
            panic!("apply did not finish");
        };
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        assert_eq!(report.snapshot.outputs[1].transform, 0);
        assert!(home.conf().contains("monitor = desc:Dell Inc. DELL P2412H KG49T35D59GU, 1920x1080@60.00, 2560x0, 1, transform, 0\n"));
        assert!(home
            .0
            .join(".config/hypr/monitors.conf.bak-20261003-120000")
            .exists());
        let calls = runner.calls();
        let position = |needle: &str| {
            calls
                .iter()
                .position(|c| c == needle)
                .unwrap_or_else(|| panic!("{needle} missing: {calls:?}"))
        };
        assert!(position("bar-ctl stop") < position("hyprctl reload"));
        assert!(position("hyprctl reload") < position("bar-ctl start"));
        assert!(calls.contains(
            &r#"hyprctl dispatch hl.dsp.window.center({window="address:0xaaa"})"#.to_string()
        ));
        // Tiled (0xbbb), on-screen (0xccc) and hidden (0xddd) windows stay put.
        assert!(!calls
            .iter()
            .any(|c| c.contains("0xbbb") || c.contains("0xccc") || c.contains("0xddd")));
        assert!(calls.contains(&"hyprctl -j configerrors".to_string()));
    }

    #[test]
    fn floating_windows_on_virtual_outputs_stay_put() {
        let home = Home::new(Some(SAVED));
        let headless = output_json(
            7,
            "HEADLESS-1",
            "Headless output",
            (1920, 1080, 60.0),
            (9000, 0),
            1.0,
            0,
            &[],
        );
        let runner = Arc::new(MockRunner::new(move |program, args| {
            match (program, args) {
                ("hyprctl", ["-j", "monitors", "all"]) => {
                    reply(&monitors_json(1).replacen('[', &format!("[{headless},"), 1))
                }
                ("hyprctl", ["reload"]) | ("hyprctl", ["dispatch", _]) => reply("ok"),
                ("hyprctl", ["-j", "clients"]) => reply(
                    r#"[{"address": "0xxr", "mapped": true, "hidden": false, "at": [9200, 100], "size": [400, 300], "floating": true, "monitor": 7},
                    {"address": "0xlost", "mapped": true, "hidden": false, "at": [20000, 100], "size": [400, 300], "floating": true, "monitor": 99}]"#,
                ),
                ("hyprctl", ["-j", "configerrors"]) => reply("[]"),
                ("hyprctl", ["systeminfo"]) => reply("configProvider: hyprlang"),
                ("hyprctl", ["-j", "workspaces"]) => reply("[]"),
                ("workspace-ctl", _) => failure(""),
                ("bar-ctl", _) => reply(""),
                _ => Err(format!("unexpected command {program} {args:?}")),
            }
        }));
        let backend = backend(runner.clone(), &home);
        let before = backend.snapshot().unwrap();
        assert!(matches!(
            backend.apply(before.fingerprint(), &targets(&before, 1)),
            ApplyOutcome::Finished(_)
        ));
        let calls = runner.calls();
        assert!(!calls.iter().any(|c| c.contains("0xxr")), "{calls:?}");
        // A window on an unknown output goes to the first physical display.
        assert!(
            calls.contains(
                &"hyprctl dispatch movewindowpixel exact 1080 570,address:0xlost".to_string()
            ),
            "{calls:?}"
        );
    }

    #[test]
    fn legacy_provider_uses_legacy_dispatchers() {
        let home = Home::new(Some(SAVED));
        let runner = fake("hyprlang", 1, 1, "ok");
        let backend = backend(runner.clone(), &home);
        let before = backend.snapshot().unwrap();
        assert!(matches!(
            backend.apply(before.fingerprint(), &targets(&before, 1)),
            ApplyOutcome::Finished(_)
        ));
        let calls = runner.calls();
        // Floating window on DP-3 (portrait 1080x1920 at 2560,0) is centered there.
        assert!(
            calls.contains(
                &"hyprctl dispatch movewindowpixel exact 2900 810,address:0xaaa".to_string()
            ),
            "{calls:?}"
        );
        backend.move_workspace_one("HDMI-A-1").unwrap();
        assert!(runner
            .calls()
            .contains(&"hyprctl dispatch moveworkspacetomonitor 1 HDMI-A-1".to_string()));
    }

    #[test]
    fn apply_lists_every_difference_hyprland_did_not_apply() {
        let home = Home::new(Some(SAVED));
        let backend = backend(fake("lua", 1, 1, "ok"), &home);
        let before = backend.snapshot().unwrap();
        let ApplyOutcome::Finished(report) =
            backend.apply(before.fingerprint(), &targets(&before, 0))
        else {
            panic!("apply did not finish");
        };
        assert_eq!(
            report.differences,
            ["DP-3 orientation: wanted Landscape, active Portrait"]
        );
    }

    #[test]
    fn apply_refuses_stale_edits_without_writing() {
        let home = Home::new(Some(SAVED));
        let runner = fake("lua", 1, 1, "ok");
        let backend = backend(runner.clone(), &home);
        let before = backend.snapshot().unwrap();
        let outcome = backend.apply(before.fingerprint() ^ 1, &targets(&before, 0));
        assert!(matches!(outcome, ApplyOutcome::Stale));
        assert_eq!(home.conf(), SAVED);
        assert!(!runner
            .calls()
            .iter()
            .any(|c| c.contains("reload") || c.contains("bar-ctl")));
    }

    #[test]
    fn rejected_reload_is_a_failure_and_the_bar_still_restarts() {
        let home = Home::new(Some(SAVED));
        let runner = fake("lua", 1, 1, "error: config broken");
        let backend = backend(runner.clone(), &home);
        let before = backend.snapshot().unwrap();
        let ApplyOutcome::Failed { message, snapshot } =
            backend.apply(before.fingerprint(), &targets(&before, 0))
        else {
            panic!("apply should fail");
        };
        assert!(message.contains("config broken"), "{message}");
        assert!(
            snapshot.is_some(),
            "the live state is re-read after writing"
        );
        assert!(runner.calls().contains(&"bar-ctl start".to_string()));
    }

    #[test]
    fn use_saved_reloads_and_reports_remaining_drift() {
        let home = Home::new(Some(SAVED));
        let restored = backend(fake("lua", 0, 1, "ok"), &home);
        let ApplyOutcome::Finished(report) = restored.use_saved() else {
            panic!("use saved did not finish");
        };
        assert!(report.differences.is_empty());
        assert_eq!(report.snapshot.outputs[1].transform, 1);
        let stuck = backend(fake("lua", 0, 0, "ok"), &home).use_saved();
        let ApplyOutcome::Finished(report) = stuck else {
            panic!("use saved did not finish");
        };
        assert_eq!(
            report.differences,
            ["Saved settings for DP-3 differ from what is active: saved Portrait, active Landscape."]
        );
    }

    #[test]
    fn identify_targets_each_screen_by_connector() {
        let home = Home::new(None);
        let runner = fake("lua", 1, 1, "ok");
        let labels = vec![
            ("DP-3".to_string(), "DP-3".to_string()),
            ("HDMI-A-1".to_string(), "HDMI-A-1".to_string()),
        ];
        backend(runner.clone(), &home).identify(&labels).unwrap();
        let config = home.0.join(".config/eww").to_string_lossy().into_owned();
        let calls = runner.calls();
        assert!(calls.contains(&format!("eww -c {config} update mon-id-label-0=DP-3")));
        assert!(calls.contains(&format!(
            "eww -c {config} open mon-identify-0 --screen DP-3 --duration 5s"
        )));
        assert!(calls.contains(&format!(
            "eww -c {config} open mon-identify-1 --screen HDMI-A-1 --duration 5s"
        )));
    }

    #[test]
    fn hyprctl_failures_make_the_snapshot_unavailable() {
        let home = Home::new(None);
        let runner = Arc::new(MockRunner::new(|_, _| {
            failure("HYPRLAND_INSTANCE_SIGNATURE was not set! (3)")
        }));
        let error = backend(runner, &home).snapshot().unwrap_err();
        assert!(error.contains("was not set"), "{error}");
    }
}
