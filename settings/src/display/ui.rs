//! Slint wiring for the Display tab. All compositor work runs on worker
//! threads; results come back through `upgrade_in_event_loop` and replace
//! the model, which is the only source of the rows the UI shows.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::backend::{ApplyOutcome, DisplayBackend, Snapshot, Target};
use super::model::{Canvas, DisplayModel};
use super::monitor::transform_label;
use super::verify::Drift;
use crate::{MainWindow, MonitorInfo};

const DISPLAY_TAB: i32 = 3;
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLICY_NOTE: &str =
    "Workspace placement follows the display arrangement. Change it in Taskbar settings.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    Load,
    Poll,
    Apply,
    KeepCurrent,
    UseSaved,
    MoveWorkspaceOne,
    Identify,
}

enum Job {
    Load {
        announce: bool,
    },
    Poll,
    Apply {
        baseline: u64,
        targets: Vec<Target>,
        keep_current: bool,
    },
    UseSaved,
    MoveWorkspaceOne(String),
    Identify(Vec<(String, String)>),
}

impl Job {
    fn kind(&self) -> JobKind {
        match self {
            Self::Load { .. } => JobKind::Load,
            Self::Poll => JobKind::Poll,
            Self::Apply {
                keep_current: false,
                ..
            } => JobKind::Apply,
            Self::Apply {
                keep_current: true, ..
            } => JobKind::KeepCurrent,
            Self::UseSaved => JobKind::UseSaved,
            Self::MoveWorkspaceOne(_) => JobKind::MoveWorkspaceOne,
            Self::Identify(_) => JobKind::Identify,
        }
    }
}

enum Done {
    Loaded {
        announce: bool,
        result: Result<Snapshot, String>,
    },
    Polled(Result<Snapshot, String>),
    Applied {
        keep_current: bool,
        outcome: ApplyOutcome,
    },
    UsedSaved(ApplyOutcome),
    Moved {
        connector: String,
        result: Result<(), String>,
        snapshot: Result<Snapshot, String>,
    },
    Identified(Result<(), String>),
}

fn run(backend: &dyn DisplayBackend, job: Job) -> Done {
    match job {
        Job::Load { announce } => Done::Loaded {
            announce,
            result: backend.snapshot(),
        },
        Job::Poll => Done::Polled(backend.snapshot()),
        Job::Apply {
            baseline,
            targets,
            keep_current,
        } => Done::Applied {
            keep_current,
            outcome: backend.apply(baseline, &targets),
        },
        Job::UseSaved => Done::UsedSaved(backend.use_saved()),
        Job::MoveWorkspaceOne(connector) => {
            let result = backend.move_workspace_one(&connector);
            Done::Moved {
                connector,
                result,
                snapshot: backend.snapshot(),
            }
        }
        Job::Identify(labels) => Done::Identified(backend.identify(&labels)),
    }
}

fn count(displays: usize) -> String {
    match displays {
        1 => "1 display".into(),
        n => format!("{n} displays"),
    }
}

/// UI-thread state shared with worker deliveries.
struct Controller {
    backend: Result<Arc<dyn DisplayBackend>, String>,
    model: DisplayModel,
    running: Option<JobKind>,
    /// A user action waiting for a background poll to finish.
    queued: Option<Job>,
    /// The latest action result: (text, is a warning).
    message: Option<(String, bool)>,
    /// Why the first read failed (nothing has been shown yet).
    load_error: Option<String>,
    poll_error: Option<String>,
}

impl Controller {
    fn new(backend: Result<Arc<dyn DisplayBackend>, String>) -> Self {
        Self {
            backend,
            model: DisplayModel::default(),
            running: None,
            queued: None,
            message: None,
            load_error: None,
            poll_error: None,
        }
    }

    fn unavailable(&self) -> Option<String> {
        match &self.backend {
            Err(reason) => Some(reason.clone()),
            Ok(_) if self.model.is_loaded() => None,
            Ok(_) => self.load_error.clone(),
        }
    }

    /// The user action in progress (background polls don't count).
    fn busy_kind(&self) -> Option<JobKind> {
        self.queued
            .as_ref()
            .map(Job::kind)
            .or(self.running.filter(|kind| *kind != JobKind::Poll))
    }

    fn busy(&self) -> bool {
        self.busy_kind().is_some()
    }

    /// The job to start now, if any. User actions wait behind a running poll
    /// (whose result is then discarded); nothing else runs concurrently.
    fn request(&mut self, job: Job) -> Option<Job> {
        if self.backend.is_err() {
            return None;
        }
        match self.running {
            None => {
                self.running = Some(job.kind());
                Some(job)
            }
            Some(JobKind::Poll) if job.kind() != JobKind::Poll => {
                self.queued = Some(job);
                None
            }
            Some(_) => None,
        }
    }

    /// Takes a worker result. Returns whether the UI needs a refresh and the
    /// queued job to start next.
    fn finish(&mut self, done: Done, dragging: bool) -> (bool, Option<Job>) {
        let superseded = self.running == Some(JobKind::Poll) && self.queued.is_some();
        self.running = None;
        let changed = !superseded && self.absorb(done, dragging);
        let next = self.queued.take();
        if let Some(job) = &next {
            self.running = Some(job.kind());
        }
        (changed || next.is_some(), next)
    }

    fn absorb(&mut self, done: Done, dragging: bool) -> bool {
        match done {
            Done::Loaded { announce, result } => {
                match result {
                    Ok(snapshot) => {
                        self.load_error = None;
                        self.poll_error = None;
                        self.model.load(snapshot);
                        self.message = announce.then(|| {
                            (
                                format!(
                                    "Display list refreshed: {}.",
                                    count(self.model.monitors.len())
                                ),
                                false,
                            )
                        });
                    }
                    Err(error) => {
                        eprintln!("[settings] display read failed: {error}");
                        if self.model.is_loaded() {
                            self.message =
                                Some((format!("Couldn't read the displays: {error}"), true));
                        } else {
                            self.load_error =
                                Some(format!("Can't read the displays from Hyprland: {error}"));
                        }
                    }
                }
                true
            }
            Done::Polled(Err(error)) => {
                let changed = self.poll_error.as_deref() != Some(error.as_str());
                self.poll_error = Some(error);
                changed
            }
            Done::Polled(Ok(snapshot)) => {
                let recovered = self.poll_error.take().is_some();
                if !self.model.is_loaded() {
                    self.load_error = None;
                    self.model.load(snapshot);
                    return true;
                }
                if snapshot.fingerprint() == self.model.baseline() {
                    return self.model.refresh_details(snapshot) || recovered;
                }
                if self.model.has_changes() {
                    let changed = !self.model.stale;
                    self.model.stale = true;
                    return changed || recovered;
                }
                if dragging {
                    return recovered;
                }
                self.model.load(snapshot);
                self.message = Some((
                    "The display configuration changed outside Settings; showing the active layout.".into(),
                    false,
                ));
                true
            }
            Done::Applied {
                keep_current,
                outcome,
            } => {
                let success = if keep_current {
                    "Saved the active layout and verified it."
                } else {
                    "Applied and verified."
                };
                self.absorb_outcome(
                    outcome,
                    success,
                    "Applied, but Hyprland is using different settings:",
                );
                true
            }
            Done::UsedSaved(outcome) => {
                self.absorb_outcome(
                    outcome,
                    "The saved display settings are active again.",
                    "Reloaded the saved settings, but Hyprland still differs:",
                );
                true
            }
            Done::Moved {
                connector,
                result,
                snapshot,
            } => {
                self.message = Some(match result {
                    Ok(()) => (format!("Moved workspace 1 to {connector}."), false),
                    Err(error) => (format!("Couldn't move workspace 1: {error}"), true),
                });
                if let Ok(snapshot) = snapshot {
                    self.absorb_snapshot(snapshot);
                }
                true
            }
            Done::Identified(result) => {
                self.message = Some(match result {
                    Ok(()) => (
                        "Showing each display's name on its screen for 5 seconds.".into(),
                        false,
                    ),
                    Err(error) => (format!("Couldn't show the display names: {error}"), true),
                });
                true
            }
        }
    }

    /// A snapshot taken after an action that did not write anything.
    fn absorb_snapshot(&mut self, snapshot: Snapshot) {
        if snapshot.fingerprint() == self.model.baseline() {
            self.model.refresh_details(snapshot);
        } else if self.model.has_changes() {
            self.model.stale = true;
        } else {
            self.model.load(snapshot);
        }
    }

    fn absorb_outcome(&mut self, outcome: ApplyOutcome, success: &str, differs: &str) {
        self.message = Some(match outcome {
            ApplyOutcome::Stale => {
                self.model.stale = true;
                (
                    "Displays changed since you started editing, so nothing was saved. Refresh to load the active layout.".into(),
                    true,
                )
            }
            ApplyOutcome::Failed { message, snapshot } => {
                if let Some(snapshot) = snapshot {
                    self.model.load(snapshot);
                }
                (message, true)
            }
            ApplyOutcome::Finished(report) => {
                self.model.load(report.snapshot);
                let mut text = if report.differences.is_empty() {
                    success.to_string()
                } else {
                    format!("{differs} {}.", report.differences.join("; "))
                };
                for warning in &report.warnings {
                    text.push(' ');
                    text.push_str(warning);
                }
                (
                    text,
                    !report.differences.is_empty() || !report.warnings.is_empty(),
                )
            }
        });
    }

    fn status(&self) -> (String, bool) {
        let mut parts = Vec::new();
        let mut warning = false;
        if let Some(kind) = self.busy_kind() {
            parts.push(
                match kind {
                    JobKind::Load | JobKind::Poll => "Reading displays…",
                    JobKind::Apply => "Applying…",
                    JobKind::KeepCurrent => "Saving the active layout…",
                    JobKind::UseSaved => "Applying the saved settings…",
                    JobKind::MoveWorkspaceOne => "Moving workspace 1…",
                    JobKind::Identify => "Identifying displays…",
                }
                .to_string(),
            );
        } else if let Some((text, is_warning)) = &self.message {
            parts.push(text.clone());
            warning = *is_warning;
        } else if self.model.has_changes() {
            parts.push(
                "Unsaved changes. Apply saves them to monitors.conf and activates them.".into(),
            );
        } else if self.model.is_loaded() {
            parts.push(format!("{} active.", count(self.model.monitors.len())));
        }
        if let Some(error) = &self.poll_error {
            parts.push(format!("Can't check the displays for changes: {error}"));
            warning = true;
        }
        if let Some(error) = self.model.conf_error() {
            parts.push(format!("The saved settings can't be read: {error}"));
            warning = true;
        }
        if let Some(note) = self.model.unmanaged_note() {
            parts.push(note);
        }
        if self.backend.as_ref().is_ok_and(|backend| backend.is_demo()) {
            parts.push("Demo mode: simulated displays; nothing is saved.".into());
        }
        (parts.join(" "), warning)
    }

    fn edit(&mut self, change: impl FnOnce(&mut DisplayModel) -> bool) -> bool {
        if self.busy() || !change(&mut self.model) {
            return false;
        }
        self.message = None;
        true
    }
}

fn shared_strings(items: impl Iterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        items.map(SharedString::from).collect::<Vec<_>>(),
    ))
}

/// The rows the UI draws, derived only from the model.
fn rows(model: &DisplayModel) -> Vec<MonitorInfo> {
    let canvas = Canvas::fit(&model.monitors);
    let primary = model.workspace_one();
    model
        .monitors
        .iter()
        .map(|monitor| {
            let (canvas_x, canvas_y) = canvas.position(monitor.x, monitor.y);
            let (w, h) = monitor.logical_size();
            let (canvas_w, canvas_h) = canvas.size(w, h);
            let (size_label, size_label_two_line) = monitor.size_labels();
            MonitorInfo {
                name: monitor.name.as_str().into(),
                description: monitor.description.as_str().into(),
                width: monitor.mode.width,
                height: monitor.mode.height,
                refresh_rate: monitor.mode.refresh as f32,
                pos_x: monitor.x,
                pos_y: monitor.y,
                scale: monitor.scale as f32,
                transform: monitor.transform,
                is_primary: primary == Some(monitor.name.as_str()),
                canvas_x,
                canvas_y,
                canvas_w,
                canvas_h,
                available_modes: shared_strings(monitor.modes.iter().map(|mode| mode.label())),
                current_mode_index: monitor.mode_index().map_or(-1, |index| index as i32),
                orientation_options: shared_strings(
                    monitor
                        .orientation_choices()
                        .into_iter()
                        .map(transform_label),
                ),
                current_orientation_index: monitor
                    .orientation_index()
                    .map_or(-1, |index| index as i32),
                size_label: size_label.into(),
                size_label_two_line: size_label_two_line.into(),
            }
        })
        .collect()
}

fn same_strings(a: &ModelRc<SharedString>, b: &ModelRc<SharedString>) -> bool {
    a.row_count() == b.row_count() && a.iter().zip(b.iter()).all(|(x, y)| x == y)
}

fn same_row(a: &MonitorInfo, b: &MonitorInfo) -> bool {
    let scalars = |row: &MonitorInfo| MonitorInfo {
        available_modes: ModelRc::default(),
        orientation_options: ModelRc::default(),
        ..row.clone()
    };
    scalars(a) == scalars(b)
        && same_strings(&a.available_modes, &b.available_modes)
        && same_strings(&a.orientation_options, &b.orientation_options)
}

/// Updates rows in place when the display count is unchanged, so the canvas
/// rectangles (and a drag in progress) survive background refreshes.
fn publish_rows(ui: &MainWindow, rows: Vec<MonitorInfo>) {
    let current = ui.get_disp_monitors();
    if let Some(model) = current.as_any().downcast_ref::<VecModel<MonitorInfo>>() {
        if model.row_count() == rows.len() {
            for (index, row) in rows.into_iter().enumerate() {
                if !model
                    .row_data(index)
                    .is_some_and(|old| same_row(&old, &row))
                {
                    model.set_row_data(index, row);
                }
            }
            return;
        }
    }
    // New rows recreate the canvas rectangles, which ends any press or drag.
    ui.set_disp_dragging(false);
    ui.set_disp_monitors(ModelRc::new(VecModel::from(rows)));
}

fn push(ui: &MainWindow, controller: &Controller) {
    let unavailable = controller.unavailable();
    ui.set_disp_available(unavailable.is_none());
    ui.set_disp_unavailable_reason(unavailable.unwrap_or_default().into());
    publish_rows(ui, rows(&controller.model));
    ui.set_disp_selected_index(controller.model.selected_index());
    ui.set_disp_has_changes(controller.model.has_changes());
    ui.set_disp_busy(controller.busy());
    ui.set_disp_stale(controller.model.stale);
    let drift: Vec<String> = controller
        .model
        .drift()
        .iter()
        .map(Drift::sentence)
        .collect();
    ui.set_disp_drift_text(drift.join("\n").into());
    ui.set_disp_workspace_policy(controller.model.workspace_policy());
    ui.set_disp_primary_name(controller.model.workspace_one().unwrap_or_default().into());
    let (status, warning) = controller.status();
    ui.set_disp_status_text(status.into());
    ui.set_disp_status_warning(warning);
}

type Shared = Arc<Mutex<Controller>>;

fn spawn(ui: &MainWindow, shared: &Shared, job: Job) {
    let Ok(backend) = shared.lock().unwrap().backend.clone() else {
        return;
    };
    let weak = ui.as_weak();
    let delivery = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("settings-display".into())
        .spawn(move || {
            let done = run(&*backend, job);
            if let Err(error) = weak.upgrade_in_event_loop(move |ui| {
                let (changed, next) = delivery
                    .lock()
                    .unwrap()
                    .finish(done, ui.get_disp_dragging());
                if changed {
                    push(&ui, &delivery.lock().unwrap());
                }
                if let Some(next) = next {
                    spawn(&ui, &delivery, next);
                }
            }) {
                eprintln!("[settings] display UI update failed: {error}");
            }
        });
    if let Err(error) = spawned {
        eprintln!("[settings] display worker could not start: {error}");
        let mut controller = shared.lock().unwrap();
        controller.running = None;
        controller.queued = None;
        controller.message = Some((
            "Couldn't start reading the displays. Please try again.".into(),
            true,
        ));
        push(ui, &controller);
    }
}

fn request(ui: &MainWindow, shared: &Shared, job: Job) {
    // Background polls change nothing visible until their result arrives.
    let user_action = job.kind() != JobKind::Poll;
    let next = shared.lock().unwrap().request(job);
    if let Some(job) = next {
        spawn(ui, shared, job);
    }
    if user_action {
        push(ui, &shared.lock().unwrap());
    }
}

pub fn install(ui: &MainWindow, backend: Result<Arc<dyn DisplayBackend>, String>) -> slint::Timer {
    let shared: Shared = Arc::new(Mutex::new(Controller::new(backend)));
    let handler = |action: fn(&MainWindow, &Shared)| {
        let weak = ui.as_weak();
        let shared = shared.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                action(&ui, &shared);
            }
        }
    };
    let editor = |change: fn(&mut DisplayModel, i32, i32) -> bool| {
        let weak = ui.as_weak();
        let shared = shared.clone();
        move |index: i32, value: i32| {
            let Some(ui) = weak.upgrade() else { return };
            let mut controller = shared.lock().unwrap();
            if controller.edit(|model| change(model, index, value)) {
                push(&ui, &controller);
            }
        }
    };

    {
        let shared = shared.clone();
        ui.on_disp_select_monitor(move |index| shared.lock().unwrap().model.select(index));
    }
    ui.on_disp_change_resolution(editor(|model, index, mode| {
        index >= 0 && mode >= 0 && model.change_mode(index as usize, mode as usize)
    }));
    ui.on_disp_change_orientation(editor(|model, index, choice| {
        index >= 0 && choice >= 0 && model.change_orientation(index as usize, choice as usize)
    }));
    {
        let weak = ui.as_weak();
        let shared = shared.clone();
        ui.on_disp_change_scale(move |index, value| {
            let Some(ui) = weak.upgrade() else { return };
            let mut controller = shared.lock().unwrap();
            if controller
                .edit(|model| index >= 0 && model.change_scale(index as usize, value as f64))
            {
                push(&ui, &controller);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let shared = shared.clone();
        ui.on_disp_drag_finished(move |index, x, y| {
            let Some(ui) = weak.upgrade() else { return };
            let mut controller = shared.lock().unwrap();
            controller.model.select(index);
            let canvas = Canvas::fit(&controller.model.monitors);
            let (x, y) = canvas.logical_at(x, y);
            controller.edit(|model| {
                index >= 0 && model.drop_at(index as usize, x, y, canvas.snap_threshold())
            });
            // Always redraw: the dragged rectangle must return to the model.
            push(&ui, &controller);
        });
    }
    {
        let weak = ui.as_weak();
        let shared = shared.clone();
        ui.on_disp_set_primary(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let job = {
                let mut controller = shared.lock().unwrap();
                if controller.busy() {
                    return;
                }
                if controller.model.workspace_policy() {
                    controller.message = Some((POLICY_NOTE.into(), false));
                    push(&ui, &controller);
                    return;
                }
                usize::try_from(index)
                    .ok()
                    .and_then(|index| controller.model.monitors.get(index))
                    .map(|monitor| Job::MoveWorkspaceOne(monitor.name.clone()))
            };
            if let Some(job) = job {
                request(&ui, &shared, job);
            }
        });
    }
    ui.on_disp_apply_changes(handler(|ui, shared| {
        let job = {
            let controller = shared.lock().unwrap();
            let model = &controller.model;
            (model.has_changes() && !model.stale && !controller.busy()).then(|| Job::Apply {
                baseline: model.baseline(),
                targets: model.targets(),
                keep_current: false,
            })
        };
        if let Some(job) = job {
            request(ui, shared, job);
        }
    }));
    ui.on_disp_keep_current(handler(|ui, shared| {
        let job = {
            let controller = shared.lock().unwrap();
            let model = &controller.model;
            (!model.has_changes() && !model.stale && !controller.busy()).then(|| Job::Apply {
                baseline: model.baseline(),
                targets: model.live_targets(),
                keep_current: true,
            })
        };
        if let Some(job) = job {
            request(ui, shared, job);
        }
    }));
    ui.on_disp_use_saved(handler(|ui, shared| {
        let allowed = {
            let controller = shared.lock().unwrap();
            !controller.model.has_changes() && !controller.model.stale && !controller.busy()
        };
        if allowed {
            request(ui, shared, Job::UseSaved);
        }
    }));
    ui.on_disp_revert_changes(handler(|ui, shared| {
        let stale = {
            let mut controller = shared.lock().unwrap();
            if controller.busy() {
                return;
            }
            controller.model.revert();
            controller.message = Some(("Changes discarded.".into(), false));
            controller.model.stale
        };
        if stale {
            request(ui, shared, Job::Load { announce: false });
        } else {
            push(ui, &shared.lock().unwrap());
        }
    }));
    ui.on_disp_refresh_monitors(handler(|ui, shared| {
        request(ui, shared, Job::Load { announce: true });
    }));
    ui.on_disp_tab_entered(handler(|ui, shared| request(ui, shared, Job::Poll)));
    ui.on_disp_identify_monitors(handler(|ui, shared| {
        let labels: Vec<(String, String)> = shared
            .lock()
            .unwrap()
            .model
            .monitors
            .iter()
            .take(4)
            .map(|monitor| (monitor.name.clone(), monitor.name.clone()))
            .collect();
        if !labels.is_empty() {
            request(ui, shared, Job::Identify(labels));
        }
    }));

    request(ui, &shared, Job::Load { announce: false });
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, POLL_INTERVAL, move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_active_tab() == DISPLAY_TAB {
                request(&ui, &shared, Job::Poll);
            }
        }
    });
    timer
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::backend::ApplyReport;
    use crate::display::demo::DemoBackend;
    use crate::display::model::tests::snapshot;
    use crate::display::monitor::tests::live_pair;

    /// A real (non-demo) backend stand-in; controller tests feed results directly.
    struct Stub;

    impl DisplayBackend for Stub {
        fn snapshot(&self) -> Result<Snapshot, String> {
            Ok(snapshot(live_pair()))
        }

        fn apply(&self, _: u64, _: &[Target]) -> ApplyOutcome {
            ApplyOutcome::Failed {
                message: "stub".into(),
                snapshot: None,
            }
        }

        fn use_saved(&self) -> ApplyOutcome {
            ApplyOutcome::Failed {
                message: "stub".into(),
                snapshot: None,
            }
        }

        fn move_workspace_one(&self, _: &str) -> Result<(), String> {
            Ok(())
        }

        fn identify(&self, _: &[(String, String)]) -> Result<(), String> {
            Ok(())
        }
    }

    fn controller() -> Controller {
        let mut controller = Controller::new(Ok(Arc::new(Stub)));
        let job = controller.request(Job::Load { announce: false }).unwrap();
        assert_eq!(job.kind(), JobKind::Load);
        controller.finish(
            Done::Loaded {
                announce: false,
                result: Ok(snapshot(live_pair())),
            },
            false,
        );
        controller
    }

    fn poll(controller: &mut Controller, snapshot: Snapshot) -> bool {
        controller.request(Job::Poll).unwrap();
        controller.finish(Done::Polled(Ok(snapshot)), false).0
    }

    fn strings(model: &ModelRc<SharedString>) -> Vec<String> {
        model.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn selecting_the_portrait_display_shows_portrait() {
        let mut controller = controller();
        controller.model.select(1);
        let rows = rows(&controller.model);
        let dp3 = &rows[controller.model.selected_index() as usize];
        assert_eq!(dp3.name, "DP-3");
        assert_eq!(dp3.current_orientation_index, 1);
        assert_eq!(strings(&dp3.orientation_options)[1], "Portrait");
        assert_eq!(dp3.transform, 1);
        assert_eq!(dp3.size_label, "1080×1920");
        assert!(dp3.canvas_h > dp3.canvas_w, "drawn as portrait");
        assert_eq!(
            strings(&dp3.available_modes)[dp3.current_mode_index as usize],
            "1920×1080 @ 60 Hz"
        );
        let hdmi = &rows[0];
        assert!(hdmi.is_primary);
        assert_eq!(hdmi.current_orientation_index, 0);
    }

    #[test]
    fn unchanged_rows_compare_equal_so_refreshes_keep_canvas_rectangles() {
        let mut controller = controller();
        let before = rows(&controller.model);
        let again = rows(&controller.model);
        assert!(before.iter().zip(&again).all(|(a, b)| same_row(a, b)));
        assert!(controller.edit(|model| model.change_orientation(1, 0)));
        let after = rows(&controller.model);
        assert!(!same_row(&before[1], &after[1]));
        let mut relabeled = before[1].clone();
        relabeled.orientation_options = shared_strings(["Landscape".to_string()].into_iter());
        assert!(!same_row(&before[1], &relabeled));
    }

    #[test]
    fn mirrored_transform_rows_name_the_exact_value() {
        let mut outputs = live_pair();
        outputs[1].transform = 7;
        let mut controller = controller();
        controller.model.load(snapshot(outputs));
        let rows = rows(&controller.model);
        assert_eq!(rows[1].current_orientation_index, 4);
        assert_eq!(
            strings(&rows[1].orientation_options)[4],
            "Mirrored, rotated 270° (transform 7)"
        );
    }

    #[test]
    fn unchanged_polls_do_not_redraw_and_changes_reload_without_edits() {
        let mut controller = controller();
        assert!(!poll(&mut controller, snapshot(live_pair())));
        let mut moved = snapshot(live_pair());
        moved.workspace_one = Some("DP-3".into());
        assert!(poll(&mut controller, moved), "the primary badge moves");
        assert_eq!(controller.model.workspace_one(), Some("DP-3"));
        controller.model.select(1);
        let mut rotated = live_pair();
        rotated[1].transform = 0;
        rotated.reverse();
        assert!(poll(&mut controller, snapshot(rotated)));
        assert_eq!(controller.model.monitors[0].transform, 0);
        assert_eq!(
            controller.model.selected_index(),
            0,
            "selection kept by name"
        );
        assert!(controller.status().0.contains("changed outside Settings"));
    }

    #[test]
    fn external_changes_with_pending_edits_block_apply() {
        let mut controller = controller();
        assert!(controller.edit(|model| model.change_orientation(1, 0)));
        let mut rotated = live_pair();
        rotated[1].transform = 3;
        assert!(poll(&mut controller, snapshot(rotated)));
        assert!(controller.model.stale);
        assert!(controller.model.has_changes(), "edits are kept");
        // Apply refused by the backend is reported and keeps the edits.
        controller.request(Job::Poll).unwrap();
        controller.finish(
            Done::Applied {
                keep_current: false,
                outcome: ApplyOutcome::Stale,
            },
            false,
        );
        assert!(controller.status().0.contains("nothing was saved"));
        assert!(controller.model.has_changes());
    }

    #[test]
    fn finished_apply_replaces_model_and_original_with_live_state() {
        let mut controller = controller();
        assert!(controller.edit(|model| model.change_orientation(1, 0)));
        assert!(controller.status().0.starts_with("Unsaved changes"));
        let mut after = live_pair();
        after[1].transform = 0;
        controller.request(Job::Poll).unwrap();
        controller.finish(
            Done::Applied {
                keep_current: false,
                outcome: ApplyOutcome::Finished(ApplyReport {
                    snapshot: snapshot(after.clone()),
                    differences: Vec::new(),
                    warnings: Vec::new(),
                }),
            },
            false,
        );
        assert!(!controller.model.has_changes());
        assert_eq!(controller.model.monitors[1].transform, 0);
        assert_eq!(
            controller.status(),
            ("Applied and verified.".to_string(), false)
        );
        controller.request(Job::Poll).unwrap();
        controller.finish(
            Done::Applied {
                keep_current: false,
                outcome: ApplyOutcome::Finished(ApplyReport {
                    snapshot: snapshot(after),
                    differences: vec!["DP-3 orientation: wanted Portrait, active Landscape".into()],
                    warnings: vec!["Hyprland config error: bad".into()],
                }),
            },
            false,
        );
        let (status, warning) = controller.status();
        assert!(warning);
        assert_eq!(
            status,
            "Applied, but Hyprland is using different settings: DP-3 orientation: wanted Portrait, active Landscape. Hyprland config error: bad"
        );
    }

    #[test]
    fn user_actions_wait_for_a_running_poll_and_discard_its_result() {
        let mut controller = controller();
        assert!(controller.request(Job::Poll).is_some());
        assert!(controller.request(Job::Poll).is_none());
        assert!(!controller.busy());
        assert!(controller.request(Job::UseSaved).is_none());
        assert!(controller.busy());
        assert_eq!(controller.status().0, "Applying the saved settings…");
        let mut rotated = live_pair();
        rotated[1].transform = 0;
        let (_, next) = controller.finish(Done::Polled(Ok(snapshot(rotated))), false);
        assert_eq!(next.map(|job| job.kind()), Some(JobKind::UseSaved));
        assert_eq!(
            controller.model.monitors[1].transform, 1,
            "superseded poll ignored"
        );
        assert!(
            controller.request(Job::Load { announce: true }).is_none(),
            "one worker at a time"
        );
        assert!(
            !controller.edit(|model| model.change_orientation(1, 0)),
            "no edits while busy"
        );
    }

    #[test]
    fn unavailable_backend_shows_the_reason_and_no_displays() {
        let mut controller =
            Controller::new(Err("Display settings work with Hyprland only.".into()));
        assert_eq!(
            controller.unavailable().as_deref(),
            Some("Display settings work with Hyprland only.")
        );
        assert!(controller.request(Job::Load { announce: false }).is_none());
        assert!(rows(&controller.model).is_empty());
        let mut failing = Controller::new(Ok(Arc::new(Stub)));
        failing.request(Job::Load { announce: false }).unwrap();
        failing.finish(
            Done::Loaded {
                announce: false,
                result: Err("hyprctl -j monitors all failed: Couldn't connect".into()),
            },
            false,
        );
        assert!(failing.unavailable().unwrap().contains("Couldn't connect"));
        // A later successful poll recovers.
        assert!(poll(&mut failing, snapshot(live_pair())));
        assert_eq!(failing.unavailable(), None);
    }

    #[test]
    fn identify_and_workspace_moves_report_results() {
        let mut controller = controller();
        controller.request(Job::Poll).unwrap();
        controller.finish(Done::Identified(Err("eww open failed".into())), false);
        assert_eq!(
            controller.status(),
            (
                "Couldn't show the display names: eww open failed".into(),
                true
            )
        );
        let mut moved = snapshot(live_pair());
        moved.workspace_one = Some("DP-3".into());
        controller.request(Job::Poll).unwrap();
        controller.finish(
            Done::Moved {
                connector: "DP-3".into(),
                result: Ok(()),
                snapshot: Ok(moved),
            },
            false,
        );
        assert_eq!(controller.model.workspace_one(), Some("DP-3"));
        assert!(rows(&controller.model)[1].is_primary);
    }

    #[test]
    fn demo_jobs_round_trip_through_the_worker_function() {
        let backend = DemoBackend::new();
        let Done::Loaded {
            result: Ok(before), ..
        } = run(&backend, Job::Load { announce: true })
        else {
            panic!("demo load failed");
        };
        let mut controller = Controller::new(Ok(Arc::new(DemoBackend::new())));
        controller.model.load(before.clone());
        assert!(controller.status().0.contains("Demo mode"));
        assert!(controller.model.change_orientation(1, 1));
        let job = Job::Apply {
            baseline: controller.model.baseline(),
            targets: controller.model.targets(),
            keep_current: false,
        };
        let Done::Applied {
            outcome: ApplyOutcome::Finished(report),
            ..
        } = run(&backend, job)
        else {
            panic!("demo apply failed");
        };
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(matches!(
            backend.apply(before.fingerprint(), &controller.model.targets()),
            ApplyOutcome::Stale
        ));
        assert!(matches!(
            run(&backend, Job::UseSaved),
            Done::UsedSaved(ApplyOutcome::Finished(_))
        ));
        assert!(matches!(
            run(
                &backend,
                Job::Identify(vec![("DP-1".into(), "DP-1".into())])
            ),
            Done::Identified(Ok(()))
        ));
        let Done::Moved {
            snapshot: Ok(moved),
            ..
        } = run(&backend, Job::MoveWorkspaceOne("eDP-1".into()))
        else {
            panic!("demo move failed");
        };
        assert_eq!(moved.workspace_one.as_deref(), Some("eDP-1"));
        assert!(matches!(run(&backend, Job::Poll), Done::Polled(Ok(_))));
    }
}
