use super::scheduler::{Request, Scheduler};
use super::*;
use crate::MainWindow;
use slint::ComponentHandle;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

type SharedScheduler = Arc<Mutex<Scheduler>>;

struct DeliveryGuard {
    scheduler: SharedScheduler,
    finished: bool,
}

impl DeliveryGuard {
    fn finish(&mut self) -> scheduler::Completion {
        let completion = self.scheduler.lock().unwrap().finish();
        self.finished = true;
        completion
    }
}

impl Drop for DeliveryGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.scheduler.lock().unwrap().abort();
        }
    }
}

fn config_path() -> Result<PathBuf> {
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .ok_or_else(|| unsupported("HOME is not set; cannot locate saved power preferences"))?;
    Ok(root.join("hypr/hypridle.conf"))
}

fn update(ui: &MainWindow, scheduler: &SharedScheduler, request: Request) {
    let (next, mutating, failure) = {
        let mut state = scheduler.lock().unwrap();
        let next = state.enqueue(request);
        (next, state.mutating(), state.take_failure())
    };
    ui.set_power_busy(mutating);
    if let Some(failure) = failure {
        ui.set_power_action_status(failure.into());
    }
    if let Some(request) = next {
        start_worker(ui, scheduler, request);
    }
}

fn start_worker(ui: &MainWindow, scheduler: &SharedScheduler, request: Request) {
    let weak = ui.as_weak();
    let mut delivery = DeliveryGuard {
        scheduler: scheduler.clone(),
        finished: false,
    };
    let spawned = std::thread::Builder::new()
        .name("settings-power".into())
        .spawn(move || {
            let hyprland =
                std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some_and(|v| !v.is_empty());
            let path = config_path();
            let action_message = match request {
                Request::Refresh => None,
                Request::Profile(index) => Some(match set_profile(&System, index) {
                    Ok(()) => "Power profile confirmed.".into(),
                    Err(error) => format!("Power profile change failed: {error}"),
                }),
                Request::Timer(action, index) => Some(match &path {
                    Ok(path) if hyprland => match PRESETS.get(index as usize) {
                        Some(&seconds) => save_timer(path, &System, action, seconds)
                            .unwrap_or_else(|error| {
                                format!("Save failed; preferences re-read: {error}")
                            }),
                        None => "Invalid timeout selection".into(),
                    },
                    Ok(_) => "Idle settings are only supported in a Hyprland session".into(),
                    Err(error) => format!("Cannot save: {error}"),
                }),
            };
            let runtime = if hyprland {
                match service(&System) {
                    Ok(s) if s.active && s.pid != 0 => {
                        "Hypridle service running; active configuration and rules are unverified."
                            .into()
                    }
                    Ok(_) => {
                        "Hypridle service is not running; unmanaged daemon state is unknown.".into()
                    }
                    Err(error) => error.to_string(),
                }
            } else {
                String::new()
            };
            let profile = profile(&System);
            let config = if hyprland {
                path.and_then(|p| read(&p)).and_then(|text| {
                    let mut revision = DefaultHasher::new();
                    text.hash(&mut revision);
                    Ok((Config::parse(&text)?, revision.finish().to_string()))
                })
            } else {
                Err(unsupported(
                    "Idle settings are unavailable outside a Hyprland session",
                ))
            };
            if let Err(error) = weak.upgrade_in_event_loop(move |ui| {
                let completion = delivery.finish();
                ui.set_power_busy(delivery.scheduler.lock().unwrap().mutating());
                if completion.publish {
                    ui.set_idle_available(config.is_ok());
                    match config {
                        Ok((config, revision)) => {
                            if matches!(request, Request::Refresh)
                                && !ui.get_power_config_revision().is_empty()
                                && ui.get_power_config_revision() != revision
                            {
                                let message = "External configuration change detected; active rules are unverified.";
                                ui.set_power_action_status(message.into());
                            }
                            ui.set_power_config_revision(revision.into());
                            let [lock, dpms, suspend, shutdown] = config.seconds;
                            ui.set_idle_lock_index(preset_index(lock));
                            ui.set_idle_dpms_index(preset_index(dpms));
                            ui.set_idle_suspend_index(preset_index(suspend));
                            ui.set_idle_shutdown_index(preset_index(shutdown));
                            ui.set_idle_lock_saved(saved_label(lock).into());
                            ui.set_idle_dpms_saved(saved_label(dpms).into());
                            ui.set_idle_suspend_saved(saved_label(suspend).into());
                            ui.set_idle_shutdown_saved(saved_label(shutdown).into());
                            ui.set_power_config_status(
                                "Saved preferences from hypridle.conf".into(),
                            );
                        }
                        Err(error) => {
                            ui.set_power_config_revision("".into());
                            ui.set_idle_lock_index(-1);
                            ui.set_idle_dpms_index(-1);
                            ui.set_idle_suspend_index(-1);
                            ui.set_idle_shutdown_index(-1);
                            for set in [
                                MainWindow::set_idle_lock_saved,
                                MainWindow::set_idle_dpms_saved,
                                MainWindow::set_idle_suspend_saved,
                                MainWindow::set_idle_shutdown_saved,
                            ] {
                                set(&ui, "Saved value unavailable".into());
                            }
                            ui.set_power_config_status(error.to_string().into());
                        }
                    }
                    ui.set_power_runtime_status(runtime.into());
                    ui.set_power_profiles_available(profile.is_ok());
                    match profile {
                        Ok(index) => {
                            ui.set_power_profile_index(index);
                            ui.set_power_profile_status("".into());
                        }
                        Err(error) => {
                            ui.set_power_profile_index(-1);
                            ui.set_power_profile_status(
                                format!("Power profiles unavailable: {error}").into(),
                            );
                        }
                    }
                    if let Some(message) = action_message {
                        ui.set_power_action_status(message.into());
                    }
                }
                if let Some(next) = completion.next {
                    start_worker(&ui, &delivery.scheduler, next);
                }
            }) {
                eprintln!("[settings] power UI update failed: {error}");
            }
        });
    if let Err(error) = spawned {
        ui.set_power_busy(false);
        ui.set_power_action_status(
            format!(
                "Power worker could not start; queued selections were cancelled. Retry: {error}"
            )
            .into(),
        );
    }
}

pub fn install(ui: &MainWindow) -> slint::Timer {
    let scheduler = Arc::new(Mutex::new(Scheduler::default()));
    let weak = ui.as_weak();
    let pending = scheduler.clone();
    ui.on_power_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            update(&ui, &pending, Request::Refresh);
        }
    });
    let weak = ui.as_weak();
    let pending = scheduler.clone();
    ui.on_set_power_profile(move |index| {
        if let Some(ui) = weak.upgrade() {
            update(&ui, &pending, Request::Profile(index));
        }
    });
    let callback = |action| {
        let weak = ui.as_weak();
        let pending = scheduler.clone();
        move |index| {
            if let Some(ui) = weak.upgrade() {
                update(&ui, &pending, Request::Timer(action, index));
            }
        }
    };
    ui.on_set_idle_lock(callback(Action::Lock));
    ui.on_set_idle_dpms(callback(Action::Dpms));
    ui.on_set_idle_suspend(callback(Action::Suspend));
    ui.on_set_idle_shutdown(callback(Action::Shutdown));
    update(ui, &scheduler, Request::Refresh);
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_secs(3),
        move || {
            if let Some(ui) = weak.upgrade() {
                if ui.get_active_tab() == 4 {
                    update(&ui, &scheduler, Request::Refresh);
                }
            }
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_ui_delivery_clears_worker_and_pending_selections() {
        let scheduler = Arc::new(Mutex::new(Scheduler::default()));
        scheduler.lock().unwrap().enqueue(Request::Refresh);
        scheduler.lock().unwrap().enqueue(Request::Profile(2));
        let delivery = DeliveryGuard {
            scheduler: scheduler.clone(),
            finished: false,
        };
        drop(delivery);
        let mut state = scheduler.lock().unwrap();
        assert!(!state.mutating());
        assert!(state.take_failure().is_some());
        assert_eq!(state.enqueue(Request::Refresh), Some(Request::Refresh));
    }

    #[test]
    fn finished_delivery_does_not_cancel_next_queued_worker_on_drop() {
        let scheduler = Arc::new(Mutex::new(Scheduler::default()));
        scheduler.lock().unwrap().enqueue(Request::Refresh);
        scheduler.lock().unwrap().enqueue(Request::Profile(2));
        let mut delivery = DeliveryGuard {
            scheduler: scheduler.clone(),
            finished: false,
        };
        assert_eq!(delivery.finish().next, Some(Request::Profile(2)));
        drop(delivery);
        let mut state = scheduler.lock().unwrap();
        assert!(state.mutating());
        assert!(state.take_failure().is_none());
        assert!(state.finish().next.is_none());
    }
}
