use super::scheduler::{Request, Scheduler};
use super::*;
use crate::{debug_log, MainWindow};
use slint::ComponentHandle;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex};

type SharedScheduler = Arc<Mutex<Scheduler>>;

const WORKER_FAILURE: &str =
    "Couldn't finish changing power settings. Please select your settings again.";
const HYPRLAND_REQUIRED: &str = "Sign in to a Hyprland session to change idle settings.";

fn config_failure(error: &Error) -> &'static str {
    match error {
        Error::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            "Idle settings file is missing. Restore hypr/hypridle.conf in your configuration folder."
        }
        Error::Io(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            "Can't access idle settings. Check the permissions on hypr/hypridle.conf."
        }
        Error::Unsupported(_) => {
            "These idle settings need manual editing. Check hypr/hypridle.conf in your configuration folder."
        }
        Error::Conflict => "Idle settings changed elsewhere. Please select your timeout again.",
        _ => "Can't read idle settings. Check hypr/hypridle.conf and try again.",
    }
}

fn timer_status(result: Result<SaveOutcome>) -> String {
    match result {
        Ok(SaveOutcome::Restarted) => {
            debug_log!("[settings] power preferences saved; daemon restarted; active rules remain unverified");
            String::new()
        }
        Ok(SaveOutcome::ApplicationUnconfirmed(error)) => {
            eprintln!("[settings] power preferences saved but application unconfirmed: {error}");
            "Saved, but application is unconfirmed. Check the Hypridle service and select your timeout again.".into()
        }
        Err(error) => {
            eprintln!("[settings] power preferences save failed: {error}");
            match error {
                Error::Conflict => config_failure(&error).into(),
                Error::Io(_) => {
                    "Couldn't save idle settings. Check file access and available disk space, then try again.".into()
                }
                _ => "Couldn't save this timeout. Check hypr/hypridle.conf and your smplOS updates, then try again.".into(),
            }
        }
    }
}

fn runtime_status(result: Result<Service>) -> &'static str {
    match result {
        Ok(service) if service.active && service.pid != 0 => "",
        Ok(_) => {
            "The idle service is stopped. Check or start your Hypridle service to use these timeouts."
        }
        Err(error) => {
            eprintln!("[settings] power service check failed: {error}");
            "Can't check the idle service. Check that the Hypridle user service is installed and available."
        }
    }
}

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
        eprintln!("[settings] {failure}");
        ui.set_power_action_status(WORKER_FAILURE.into());
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
                    Ok(()) => {
                        debug_log!("[settings] power profile confirmed: {index}");
                        String::new()
                    }
                    Err(error) => {
                        eprintln!("[settings] power profile change failed: {error}");
                        "Couldn't change the power profile. Check that the power profiles service is available, then try again.".into()
                    }
                }),
                Request::Timer(action, index) => Some(match &path {
                    Ok(path) if hyprland => match PRESETS.get(index as usize) {
                        Some(&seconds) => timer_status(save_timer(path, &System, action, seconds)),
                        None => {
                            eprintln!("[settings] invalid power timeout index: {index}");
                            "Please choose one of the available timeouts.".into()
                        }
                    },
                    Ok(_) => HYPRLAND_REQUIRED.into(),
                    Err(error) => {
                        eprintln!("[settings] cannot locate power preferences: {error}");
                        "Can't locate idle settings. Check your configuration folder and sign in again.".into()
                    }
                }),
            };
            let runtime = if hyprland {
                runtime_status(service(&System))
            } else {
                ""
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
                                debug_log!("[settings] external power configuration change detected; active rules remain unverified");
                            }
                            ui.set_power_config_revision(revision.into());
                            let [lock, dpms, suspend, shutdown] = config.seconds;
                            ui.set_idle_lock_index(preset_index(lock));
                            ui.set_idle_dpms_index(preset_index(dpms));
                            ui.set_idle_suspend_index(preset_index(suspend));
                            ui.set_idle_shutdown_index(preset_index(shutdown));
                            ui.set_idle_lock_custom_label(timeout_label(lock).into());
                            ui.set_idle_dpms_custom_label(timeout_label(dpms).into());
                            ui.set_idle_suspend_custom_label(timeout_label(suspend).into());
                            ui.set_idle_shutdown_custom_label(timeout_label(shutdown).into());
                            ui.set_power_config_status("".into());
                        }
                        Err(error) => {
                            ui.set_power_config_revision("".into());
                            ui.set_idle_lock_index(-1);
                            ui.set_idle_dpms_index(-1);
                            ui.set_idle_suspend_index(-1);
                            ui.set_idle_shutdown_index(-1);
                            for set in [
                                MainWindow::set_idle_lock_custom_label,
                                MainWindow::set_idle_dpms_custom_label,
                                MainWindow::set_idle_suspend_custom_label,
                                MainWindow::set_idle_shutdown_custom_label,
                            ] {
                                set(&ui, "Custom".into());
                            }
                            let message = if hyprland {
                                eprintln!("[settings] power preferences read failed: {error}");
                                config_failure(&error)
                            } else {
                                HYPRLAND_REQUIRED
                            };
                            ui.set_power_config_status(message.into());
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
                            eprintln!("[settings] power profiles unavailable: {error}");
                            ui.set_power_profile_index(-1);
                            ui.set_power_profile_status(
                                "Power profiles aren't available. Check that the power profiles service is installed and running.".into(),
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
        eprintln!("[settings] power worker could not start: {error}");
        ui.set_power_busy(false);
        ui.set_power_action_status(WORKER_FAILURE.into());
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
    fn successful_save_and_running_service_have_no_user_status() {
        assert!(timer_status(Ok(SaveOutcome::Restarted)).is_empty());
        assert_eq!(
            runtime_status(Ok(Service {
                active: true,
                pid: 42,
            })),
            ""
        );
    }

    #[test]
    fn save_failures_remain_actionable_without_technical_diagnostics() {
        let detail = "native parser trace or command stderr";
        let unconfirmed = timer_status(Ok(SaveOutcome::ApplicationUnconfirmed(Error::Command(
            detail.into(),
        ))));
        assert!(unconfirmed.starts_with("Saved, but application is unconfirmed."));
        assert!(unconfirmed.contains("select your timeout again"));
        assert!(!unconfirmed.contains(detail));
        for error in [
            Error::Command(detail.into()),
            unsupported(detail),
            Error::Io(std::io::Error::other(detail)),
            Error::Conflict,
        ] {
            let message = timer_status(Err(error));
            assert!(!message.is_empty());
            assert!(!message.contains(detail));
            assert!(!message.starts_with("Saved"));
            assert!(message.contains("again"));
        }
    }

    #[test]
    fn configuration_errors_offer_recovery_without_raw_details() {
        for (kind, expected) in [
            (std::io::ErrorKind::NotFound, "Restore"),
            (std::io::ErrorKind::PermissionDenied, "permissions"),
            (std::io::ErrorKind::Other, "try again"),
        ] {
            let error = Error::Io(std::io::Error::new(kind, "technical details"));
            let message = config_failure(&error);
            assert!(message.contains(expected));
            assert!(!message.contains("technical details"));
        }
        assert!(config_failure(&unsupported("unknown command")).contains("manual editing"));
    }

    #[test]
    fn stopped_or_unknown_service_has_friendly_status() {
        for service in [
            Service {
                active: false,
                pid: 0,
            },
            Service {
                active: true,
                pid: 0,
            },
        ] {
            assert!(runtime_status(Ok(service)).contains("Check or start"));
        }
        let message = runtime_status(Err(Error::Command("raw systemctl output".into())));
        assert!(message.contains("Check"));
        assert!(!message.contains("raw systemctl output"));
    }

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
