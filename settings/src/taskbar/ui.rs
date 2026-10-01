use super::scheduler::{Request, Scheduler};
use super::*;
use crate::MainWindow;
use slint::ComponentHandle;
use std::sync::{Arc, Mutex};

const WORKER_FAILURE: &str =
    "Couldn't finish changing taskbar settings. Please select your settings again.";

#[derive(Default)]
struct State {
    scheduler: Scheduler,
    errors: [String; 7],
    read_error: String,
    worker_error: String,
}

impl State {
    fn abort(&mut self) {
        self.scheduler.abort();
        self.worker_error = WORKER_FAILURE.into();
    }

    fn status(&self) -> String {
        self.errors
            .iter()
            .chain([&self.read_error, &self.worker_error])
            .filter(|message| !message.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }
}

type Shared = Arc<Mutex<State>>;

struct Delivery {
    state: Shared,
    weak: slint::Weak<MainWindow>,
    finished: bool,
}

impl Drop for Delivery {
    fn drop(&mut self) {
        if !self.finished {
            self.state.lock().unwrap().abort();
            eprintln!("[settings] taskbar worker/UI delivery stopped; pending choices cancelled");
            let state = self.state.clone();
            if let Err(error) = self.weak.upgrade_in_event_loop(move |ui| {
                let state = state.lock().unwrap();
                ui.set_tb_busy(state.scheduler.mutating());
                ui.set_tb_status(state.status().into());
            }) {
                eprintln!("[settings] taskbar failure notification unavailable: {error}");
            }
        }
    }
}

fn outcome_message(setting: Setting, outcome: Result<SaveOutcome>) -> String {
    let reason = match outcome {
        Ok(SaveOutcome::Updated) => return String::new(),
        Ok(SaveOutcome::SavedButNotApplied(error)) => {
            eprintln!(
                "[settings] {} saved but not applied: {error}",
                setting.key()
            );
            if matches!(error, Error::Conflict) {
                "Saved, but settings changed elsewhere. Check the values and select your setting again."
            } else {
                "Saved, but the taskbar couldn't be updated. Check that the bar is running and smplOS is up to date, then try again."
            }
        }
        Err(error) => {
            eprintln!("[settings] {} save failed: {error}", setting.key());
            match error {
                Error::Conflict => "Settings changed elsewhere. Please select your setting again.",
                Error::Invalid(_) => {
                    "Couldn't save. Check the values in ~/.config/smplos/bar.conf, then try again."
                }
                _ => "Couldn't save. Check file access and free disk space, then try again.",
            }
        }
    };
    format!("{}: {reason}", setting.label())
}

fn publish(ui: &MainWindow, snapshot: Snapshot) {
    let [count, position, spacing, style, format, clock_24h, date] = snapshot.values;
    ui.set_tb_ws_count(count);
    ui.set_tb_ws_position_index(position);
    ui.set_tb_ws_spacing(spacing);
    ui.set_tb_ws_style_index(style);
    ui.set_tb_clock_format_index(format);
    ui.set_tb_clock_24h(clock_24h != 0);
    ui.set_tb_clock_date_fmt_index(date);
}

fn update(ui: &MainWindow, shared: &Shared, request: Request) {
    let next = {
        let mut state = shared.lock().unwrap();
        if state.scheduler.take_failure() {
            eprintln!("[settings] taskbar worker/UI delivery failed; pending choices cancelled");
            state.worker_error = WORKER_FAILURE.into();
        }
        let next = state.scheduler.enqueue(request);
        ui.set_tb_busy(state.scheduler.mutating());
        ui.set_tb_status(state.status().into());
        next
    };
    if let Some(next) = next {
        start_worker(ui, shared, next);
    }
}

fn start_worker(ui: &MainWindow, shared: &Shared, request: Request) {
    let weak = ui.as_weak();
    let mut delivery = Delivery {
        state: shared.clone(),
        weak: weak.clone(),
        finished: false,
    };
    let spawned = std::thread::Builder::new().name("settings-taskbar".into()).spawn(move || {
        let message = match request {
            Request::Refresh => None,
            Request::Set(setting, value) => Some((setting, outcome_message(setting,
                config_path().and_then(|path| change(&path, &System, setting, value))
            ))),
        };
        let snapshot = config_path().and_then(|path| read(&path));
        if let Err(error) = weak.upgrade_in_event_loop(move |ui| {
            let (publish_snapshot, next) = {
                let mut state = delivery.state.lock().unwrap();
                let completed = state.scheduler.finish();
                if let Some((setting, message)) = message {
                    state.errors[setting as usize] = message;
                    state.worker_error.clear();
                }
                if completed.0 {
                    match &snapshot {
                        Ok(_) => state.read_error.clear(),
                        Err(error) => {
                            eprintln!("[settings] taskbar preferences unavailable: {error}");
                            state.read_error = "Can't read taskbar settings. Check ~/.config/smplos/bar.conf, then reopen this tab.".into();
                        }
                    }
                }
                ui.set_tb_busy(state.scheduler.mutating());
                ui.set_tb_status(state.status().into());
                completed
            };
            delivery.finished = true;
            if publish_snapshot {
                if let Ok(snapshot) = snapshot { publish(&ui, snapshot); }
            }
            if let Some(next) = next { start_worker(&ui, &delivery.state, next); }
        }) {
            eprintln!("[settings] taskbar UI delivery failed: {error}");
        }
    });
    if let Err(error) = spawned {
        eprintln!("[settings] cannot start taskbar worker: {error}");
        let mut state = shared.lock().unwrap();
        state.worker_error = WORKER_FAILURE.into();
        ui.set_tb_busy(false);
        ui.set_tb_status(state.status().into());
    }
}

pub fn install(ui: &MainWindow) {
    let shared = Shared::default();
    let weak = ui.as_weak();
    let state = shared.clone();
    ui.on_tb_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            update(&ui, &state, Request::Refresh);
        }
    });
    let callback = |setting| {
        let weak = ui.as_weak();
        let state = shared.clone();
        move |value| {
            if let Some(ui) = weak.upgrade() {
                update(&ui, &state, Request::Set(setting, value));
            }
        }
    };
    ui.on_tb_set_ws_count(callback(Setting::Count));
    ui.on_tb_set_ws_position(callback(Setting::Position));
    ui.on_tb_set_ws_spacing(callback(Setting::Spacing));
    ui.on_tb_set_ws_style(callback(Setting::Style));
    ui.on_tb_set_clock_format(callback(Setting::ClockFormat));
    ui.on_tb_set_clock_date_fmt(callback(Setting::ClockDate));
    let set_24h = callback(Setting::Clock24h);
    ui.on_tb_set_clock_24h(move |on| set_24h(i32::from(on)));
    update(ui, &shared, Request::Refresh);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successful_changes_are_quiet_but_failures_distinguish_saved_values() {
        assert!(outcome_message(Setting::Count, Ok(SaveOutcome::Updated)).is_empty());
        assert!(outcome_message(
            Setting::Count,
            Ok(SaveOutcome::SavedButNotApplied(Error::Command(
                "details".into()
            )))
        )
        .contains("Saved, but"));
        let failed = outcome_message(Setting::Count, Err(Error::Invalid("details".into())));
        assert!(failed.contains("Couldn't save"));
        assert!(!failed.contains("details"));
    }

    #[test]
    fn other_success_and_refresh_do_not_clear_a_failed_choice() {
        let mut state = State::default();
        state.errors[Setting::Count as usize] = "Count failed".into();
        state.errors[Setting::Style as usize].clear();
        state.read_error.clear();
        assert_eq!(state.status(), "Count failed");
    }

    #[test]
    fn aborted_delivery_clears_busy_and_preserves_other_action_errors() {
        let mut state = State::default();
        state.errors[Setting::Count as usize] = "Count failed".into();
        state.scheduler.enqueue(Request::Set(Setting::Style, 1));
        state.abort();
        assert!(!state.scheduler.mutating());
        assert!(state.status().contains("Count failed"));
        assert!(state.status().contains(WORKER_FAILURE));
        assert_eq!(
            state.scheduler.enqueue(Request::Refresh),
            Some(Request::Refresh)
        );
    }
}
