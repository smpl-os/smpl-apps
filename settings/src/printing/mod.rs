//! Native consumer of the OS-owned, local-CUPS printing protocol.
pub mod ui;

use serde::Deserialize;
use std::collections::HashSet;
use std::process::{Command, Stdio};

const HELPER: &str = "smplos-printing";
const UNAVAILABLE: &str = "Printing is unavailable. Check that smplOS is up to date and the local CUPS service is accessible, then Refresh.";

#[derive(Clone, Debug, Deserialize)]
pub struct Printer {
    pub id: String,
    pub display_name: String,
    pub uri: String,
    pub state: String,
    pub accepting_jobs: bool,
    pub is_default: bool,
    pub is_temporary: bool,
    pub state_reasons: Vec<String>,
    pub state_message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Job {
    pub id: i32,
    pub printer_id: String,
    pub printer_uri: String,
    pub identity: String,
    pub title: String,
    pub user: String,
    pub state: String,
    pub state_reasons: Vec<String>,
    pub own: bool,
    pub can_hold: bool,
    pub can_release: bool,
    pub can_cancel: bool,
    pub created_at: Option<i64>,
    pub completed_at: Option<i64>,
}

impl Job {
    pub fn historical(&self) -> bool {
        matches!(self.state.as_str(), "canceled" | "aborted" | "completed")
    }

    fn allows(&self, action: Action) -> bool {
        self.own
            && matches!(
                self.state.as_str(),
                "pending" | "held" | "processing" | "stopped"
            )
            && self.id > 0
            && !self.printer_id.is_empty()
            && !self.identity.is_empty()
            && match action {
                Action::Hold => self.can_hold,
                Action::Release => self.can_release,
                Action::Cancel => self.can_cancel,
            }
    }

    fn same_job(&self, other: &Self) -> bool {
        self.id == other.id
            && self.printer_id == other.printer_id
            && self.identity == other.identity
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Warning {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    pub server: String,
    pub printers: Vec<Printer>,
    pub jobs: Vec<Job>,
    pub warnings: Vec<Warning>,
    pub active_complete: bool,
    pub history_complete: bool,
    pub history_limit: usize,
}

#[derive(Deserialize)]
struct Envelope {
    protocol_version: u32,
    ok: bool,
    error: Option<Warning>,
}

fn envelope(text: &str, success: bool) -> Result<(), String> {
    let response: Envelope = serde_json::from_str(text).map_err(|_| {
        "The printing helper returned an invalid response. Update smplOS and try Refresh."
            .to_string()
    })?;
    if response.protocol_version != 1 {
        return Err("The printing helper uses an unsupported protocol. Update smplOS and Settings together.".into());
    }
    if !success || !response.ok {
        return Err(response.error.map_or_else(
            || UNAVAILABLE.to_string(),
            |error| {
                format!(
                    "Printing: {}. {}",
                    clean(&error.code),
                    clean(&error.message)
                )
            },
        ));
    }
    Ok(())
}

fn parse_snapshot(text: &str, success: bool) -> Result<Snapshot, String> {
    envelope(text, success)?;
    let snapshot: Snapshot = serde_json::from_str(text).map_err(|_| {
        "The printing helper returned incomplete status data. Update smplOS and try Refresh."
            .to_string()
    })?;
    if snapshot.server != "local" || snapshot.history_limit > 100 {
        return Err("The printing helper returned an unsupported queue scope.".into());
    }
    let mut printers = HashSet::new();
    let mut jobs = HashSet::new();
    if snapshot
        .printers
        .iter()
        .any(|printer| printer.id.is_empty() || !printers.insert(&printer.id))
        || snapshot
            .jobs
            .iter()
            .any(|job| job.id <= 0 || !jobs.insert(job.id))
    {
        return Err("The printing helper returned ambiguous queue identities. Try Refresh.".into());
    }
    Ok(snapshot)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Hold,
    Release,
    Cancel,
}

impl Action {
    fn name(self) -> &'static str {
        match self {
            Self::Hold => "hold",
            Self::Release => "release",
            Self::Cancel => "cancel",
        }
    }
}

pub trait Backend: Send + Sync {
    fn status(&self) -> Result<Snapshot, String>;
    fn action(&self, job: &Job, action: Action) -> Result<(), String>;
}

pub struct System;

fn run(args: &[&str]) -> Result<(String, bool), String> {
    let output = Command::new("timeout")
        .args(["--signal=TERM", "--kill-after=1s", "20s", HELPER])
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| UNAVAILABLE.to_string())?;
    match output.status.code() {
        Some(124 | 137) | None => Err("The printing request timed out; its outcome may be unknown. Refresh to check the queue before choosing any action again. Nothing is retried automatically.".into()),
        Some(125..=127) => Err(UNAVAILABLE.into()),
        _ => String::from_utf8(output.stdout)
            .map(|text| (text, output.status.success()))
            .map_err(|_| "The printing helper returned invalid text. Update smplOS and try Refresh.".into()),
    }
}

fn action_args(job: &Job, action: Action) -> Vec<String> {
    let mut args = vec![
        "job".into(),
        action.name().into(),
        "--id".into(),
        job.id.to_string(),
        "--printer".into(),
        job.printer_id.clone(),
        "--identity".into(),
        job.identity.clone(),
        "--json".into(),
    ];
    if action == Action::Cancel {
        args.push("--confirm".into());
    }
    args
}

impl Backend for System {
    fn status(&self) -> Result<Snapshot, String> {
        let (text, success) = run(&["status", "--json"])?;
        parse_snapshot(&text, success)
    }

    fn action(&self, job: &Job, action: Action) -> Result<(), String> {
        if !job.allows(action) {
            return Err(
                "This job action is no longer available. Refresh to check its current state."
                    .into(),
            );
        }
        let args = action_args(job, action);
        let (text, success) = run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        parse_action(&text, success, job, action)
    }
}

fn parse_action(text: &str, success: bool, job: &Job, action: Action) -> Result<(), String> {
    envelope(text, success)?;
    #[derive(Deserialize)]
    struct Reply {
        action: String,
        id: i32,
    }
    let reply: Reply = serde_json::from_str(text).map_err(|_| {
        "The job action reply was incomplete. Refresh before choosing an action again.".to_string()
    })?;
    if reply.id != job.id || reply.action != action.name() {
        return Err("The job action reply did not match the request. Refresh before choosing an action again.".into());
    }
    Ok(())
}

pub fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(1000)
        .collect()
}

#[derive(Default)]
pub struct State {
    pub snapshot: Option<Snapshot>,
    pub error: String,
    pub action_error: String,
    pub busy: bool,
    pub pending_cancel: Option<Job>,
}

impl State {
    pub fn jobs(&self, history: bool) -> Vec<&Job> {
        self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
            snapshot
                .jobs
                .iter()
                .filter(|job| job.historical() == history)
                .collect()
        })
    }

    pub fn ready(&self) -> bool {
        self.snapshot.is_some() && self.error.is_empty()
    }

    pub fn request(&mut self, index: usize, history: bool, action: Action) -> Option<Job> {
        if self.busy || !self.ready() || self.pending_cancel.is_some() {
            return None;
        }
        let job = self.jobs(history).get(index).copied()?.clone();
        if !job.allows(action) {
            self.action_error =
                "This action is unavailable for this job. Refresh to check permissions and state."
                    .into();
            return None;
        }
        if action == Action::Cancel {
            self.pending_cancel = Some(job);
            return None;
        }
        Some(job)
    }

    pub fn confirm_cancel(&mut self) -> Option<Job> {
        if self.busy || !self.ready() {
            return None;
        }
        let pending = self.pending_cancel.take()?;
        if self
            .snapshot
            .as_ref()?
            .jobs
            .iter()
            .any(|job| job.same_job(&pending) && job.allows(Action::Cancel))
        {
            Some(pending)
        } else {
            self.action_error =
                "The job changed while confirmation was open. Refresh and select the job again."
                    .into();
            None
        }
    }

    pub fn apply(
        &mut self,
        snapshot: Result<Snapshot, String>,
        action: Option<Result<(), String>>,
    ) {
        self.busy = false;
        if let Some(result) = action {
            self.action_error = result.err().unwrap_or_default();
        }
        match snapshot {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.error.clear();
            }
            Err(error) => {
                self.error = error;
                self.pending_cancel = None;
            }
        }
    }
}

#[cfg(test)]
mod tests;
