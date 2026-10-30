use super::{clean, Action, Backend, Job, Snapshot, State, System};
use crate::{MainWindow, PrintJob, PrintPrinter};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{mpsc, Arc};
use std::time::Duration;

#[cfg(test)]
mod native_tests;

struct Completion {
    snapshot: Result<Snapshot, String>,
    action: Option<Result<(), String>>,
}

struct Controller {
    state: State,
    backend: Arc<dyn Backend>,
    receiver: Option<mpsc::Receiver<Completion>>,
}

impl Controller {
    fn start(&mut self, job: Option<(Job, Action)>) {
        if self.state.busy {
            return;
        }
        self.state.busy = true;
        let backend = self.backend.clone();
        let (sender, receiver) = mpsc::channel();
        match std::thread::Builder::new()
            .name("settings-printing".into())
            .spawn(move || {
                let action = job.map(|(job, action)| backend.action(&job, action));
                let snapshot = backend.status();
                if sender.send(Completion { snapshot, action }).is_err() {
                    eprintln!("[settings] printing result delivery stopped");
                }
            }) {
            Ok(_) => self.receiver = Some(receiver),
            Err(_) => {
                self.state.busy = false;
                self.state.error =
                    "Couldn't start the printing worker. Reopen Settings and try again.".into();
                eprintln!("[settings] printing worker could not start");
            }
        }
    }
}

fn rows<T: Clone + 'static>(values: Vec<T>) -> ModelRc<T> {
    Rc::new(VecModel::from(values)).into()
}

fn reasons(reasons: &[String]) -> String {
    reasons
        .iter()
        .map(|reason| clean(reason))
        .collect::<Vec<_>>()
        .join(", ")
}

fn timestamp(value: Option<i64>) -> String {
    value
        .and_then(|value| chrono::DateTime::from_timestamp(value, 0))
        .map_or_else(
            || "not reported".into(),
            |value| {
                value
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            },
        )
}

fn publish(ui: &MainWindow, state: &State) {
    ui.set_print_busy(state.busy);
    ui.set_print_ready(state.ready());
    ui.set_print_jobs_complete(state.snapshot.as_ref().is_some_and(|snapshot| {
        if ui.get_print_history() {
            snapshot.history_complete
        } else {
            snapshot.active_complete
        }
    }));
    let mut status = if state.snapshot.is_some() {
        "Local CUPS queues. Status refreshes every five seconds while this page is open.".into()
    } else {
        "Checking the local print service...".to_string()
    };
    if !state.error.is_empty() {
        status = format!(
            "{}{}",
            state.error,
            if state.snapshot.is_some() {
                " Showing last known data; job actions are disabled."
            } else {
                " No queue data is available."
            }
        );
    }
    if !state.action_error.is_empty() {
        status.push('\n');
        status.push_str(&state.action_error);
    }
    ui.set_print_status(status.into());
    let mut warnings = Vec::new();
    let mut printers = Vec::new();
    let mut scope = String::new();
    if let Some(snapshot) = &state.snapshot {
        for warning in &snapshot.warnings {
            let mut message = clean(&warning.message);
            if warning.code == "vpn_lan_blocked" {
                message.push_str(" Review local-network/LAN sharing in your VPN settings if you trust this network. Settings will not change VPN policy.");
            }
            warnings.push(message);
        }
        if !snapshot.active_complete {
            warnings.push("Active job visibility is incomplete. Permissions or a CUPS query failed; the list is not proof that other jobs do not exist.".into());
        }
        if !snapshot.history_complete {
            warnings.push("History is incomplete or limited by CUPS retention/permissions. Missing jobs do not mean nothing was submitted.".into());
        }
        for printer in &snapshot.printers {
            printers.push(PrintPrinter {
                name: format!(
                    "{} ({}){}",
                    clean(&printer.display_name),
                    clean(&printer.id),
                    if printer.is_default { " - Default" } else { "" }
                )
                .into(),
                summary: format!(
                    "{} - {}{}",
                    clean(&printer.state),
                    if printer.accepting_jobs {
                        "Accepting jobs"
                    } else {
                        "Not accepting new jobs"
                    },
                    if printer.is_temporary {
                        " - Temporary/discovered queue"
                    } else {
                        ""
                    }
                )
                .into(),
                detail: format!(
                    "Queue: {}\n{} {}",
                    clean(&printer.uri),
                    clean(&printer.state_message),
                    reasons(&printer.state_reasons)
                )
                .into(),
            });
        }
        scope = if ui.get_print_history() {
            format!("Retained completed, canceled and aborted jobs across local queues (up to {}). History is not a permanent print audit.", snapshot.history_limit)
        } else {
            "Active jobs across local queues, including jobs whose printer entry is no longer listed. Other users' jobs may be restricted by CUPS permissions.".into()
        };
    }
    ui.set_print_printers(rows(printers));
    ui.set_print_warning(warnings.join("\n\n").into());
    ui.set_print_scope(scope.into());
    ui.set_print_jobs(rows(
        state
            .jobs(ui.get_print_history())
            .into_iter()
            .map(|job| PrintJob {
                id: job.id,
                printer_id: job.printer_id.clone().into(),
                identity: job.identity.clone().into(),
                title: format!(
                    "#{} - {}",
                    job.id,
                    if job.title.is_empty() {
                        "Untitled job".into()
                    } else {
                        clean(&job.title)
                    }
                )
                .into(),
                detail: format!(
                    "{} - Queue: {} - Owner: {}{}\n{}\nSubmitted: {}{}\n{}{}",
                    clean(&job.state),
                    if job.printer_id.is_empty() {
                        "unavailable".into()
                    } else {
                        clean(&job.printer_id)
                    },
                    if job.user.is_empty() {
                        "unavailable".into()
                    } else {
                        clean(&job.user)
                    },
                    if job.own { " (you)" } else { "" },
                    clean(&job.printer_uri),
                    timestamp(job.created_at),
                    if job.historical() {
                        format!(" - Finished: {}", timestamp(job.completed_at))
                    } else {
                        String::new()
                    },
                    reasons(&job.state_reasons),
                    if job.identity.is_empty() && !job.historical() {
                        "\nControls unavailable: CUPS did not provide a stable job identity."
                    } else {
                        ""
                    }
                )
                .into(),
                can_hold: job.allows(Action::Hold),
                can_release: job.allows(Action::Release),
                can_cancel: job.allows(Action::Cancel),
            })
            .collect(),
    ));
    ui.set_print_confirmation(state.pending_cancel.as_ref().map_or_else(String::new, |job| {
        format!("Cancel job #{} \"{}\" in queue \"{}\"? This stops this job; it cannot be undone. Pages already printed cannot be recalled. No other jobs will be changed.", job.id,
            clean(&job.title).chars().take(100).collect::<String>(),
            clean(&job.printer_id).chars().take(100).collect::<String>())
    }).into());
}

pub struct Printing {
    _delivery: Rc<Timer>,
    _poll: Timer,
}

fn arm_delivery(ui: &MainWindow, shared: &Rc<RefCell<Controller>>, timer: &Rc<Timer>) {
    if timer.running() {
        return;
    }
    let weak = ui.as_weak();
    let shared = shared.clone();
    let weak_timer = Rc::downgrade(timer);
    timer.start(TimerMode::SingleShot, Duration::from_millis(100), move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(timer) = weak_timer.upgrade() else { return };
        let mut controller = shared.borrow_mut();
        let Some(receiver) = &controller.receiver else { return };
        match receiver.try_recv() {
            Ok(completion) => {
                controller.receiver = None;
                controller.state.apply(completion.snapshot, completion.action);
            }
            Err(mpsc::TryRecvError::Empty) => {
                arm_delivery(&ui, &shared, &timer);
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                controller.receiver = None;
                controller.state.apply(Err("The printing worker stopped. Refresh to check the queue; an action's outcome may be unknown. Nothing is retried automatically.".into()), None);
                eprintln!("[settings] printing worker stopped before delivering a result");
            }
        }
        publish(&ui, &controller.state);
    });
}

pub fn install(ui: &MainWindow) -> Printing {
    install_with(ui, Arc::new(System))
}

fn install_with(ui: &MainWindow, backend: Arc<dyn Backend>) -> Printing {
    let controller = Rc::new(RefCell::new(Controller {
        state: State::default(),
        backend,
        receiver: None,
    }));
    let delivery = Rc::new(Timer::default());
    let delivery_timer = delivery.clone();
    let weak = ui.as_weak();
    let shared = controller.clone();
    ui.on_print_refresh(move || {
        if let Some(ui) = weak.upgrade() {
            let mut controller = shared.borrow_mut();
            if controller.state.pending_cancel.is_none() {
                controller.start(None);
            }
            publish(&ui, &controller.state);
            if controller.receiver.is_some() {
                arm_delivery(&ui, &shared, &delivery_timer);
            }
        }
    });
    let weak = ui.as_weak();
    let shared = controller.clone();
    let delivery_timer = delivery.clone();
    ui.on_print_action(move |id, printer_id, identity, action| {
        if let Some(ui) = weak.upgrade() {
            if ui.get_active_tab() != 13 {
                return;
            }
            let mut controller = shared.borrow_mut();
            let action = match action.as_str() {
                "hold" => Action::Hold,
                "release" => Action::Release,
                "cancel" => Action::Cancel,
                _ => {
                    controller.state.action_error =
                        "Unknown printing action; nothing was changed.".into();
                    publish(&ui, &controller.state);
                    return;
                }
            };
            let index = controller
                .state
                .jobs(ui.get_print_history())
                .iter()
                .position(|job| {
                    job.id == id
                        && job.printer_id == printer_id.as_str()
                        && job.identity == identity.as_str()
                });
            if let Some(index) = index {
                if let Some(job) = controller
                    .state
                    .request(index, ui.get_print_history(), action)
                {
                    controller.start(Some((job, action)));
                }
            } else {
                controller.state.action_error =
                    "The selected job changed. Refresh and select it again; nothing was changed."
                        .into();
            }
            publish(&ui, &controller.state);
            if controller.receiver.is_some() {
                arm_delivery(&ui, &shared, &delivery_timer);
            }
        }
    });
    let weak = ui.as_weak();
    let shared = controller.clone();
    let delivery_timer = delivery.clone();
    ui.on_print_confirm(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_active_tab() != 13 {
                return;
            }
            let mut controller = shared.borrow_mut();
            if let Some(job) = controller.state.confirm_cancel() {
                controller.start(Some((job, Action::Cancel)));
            }
            publish(&ui, &controller.state);
            if controller.receiver.is_some() {
                arm_delivery(&ui, &shared, &delivery_timer);
            }
        }
    });
    let weak = ui.as_weak();
    let shared = controller.clone();
    ui.on_print_dismiss(move || {
        if let Some(ui) = weak.upgrade() {
            let mut controller = shared.borrow_mut();
            controller.state.pending_cancel = None;
            publish(&ui, &controller.state);
        }
    });
    let poll = Timer::default();
    let weak = ui.as_weak();
    poll.start(TimerMode::Repeated, Duration::from_secs(5), move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_active_tab() == 13 && ui.get_print_confirmation().is_empty() {
                ui.invoke_print_refresh();
            }
        }
    });
    if ui.get_active_tab() == 13 {
        ui.invoke_print_refresh();
    }
    Printing {
        _delivery: delivery,
        _poll: poll,
    }
}
