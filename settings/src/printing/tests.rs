use super::*;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/status-v1.json");

fn snapshot() -> Snapshot {
    parse_snapshot(FIXTURE, true).unwrap()
}

fn ready() -> State {
    State {
        snapshot: Some(snapshot()),
        ..State::default()
    }
}

#[test]
fn printing_contract_preserves_distinct_queues_and_unlisted_queue_history() {
    let data = snapshot();
    assert_eq!(data.printers.len(), 2);
    assert_eq!(data.printers[0].display_name, data.printers[1].display_name);
    assert_ne!(data.printers[0].id, data.printers[1].id);
    let state = ready();
    assert_eq!(state.jobs(false)[0].id, 41);
    assert_eq!(state.jobs(true)[0].printer_id, "Old_queue");
    assert!(!data.history_complete);
    assert_eq!(data.warnings[0].code, "vpn_lan_blocked");
}

#[test]
fn printing_failures_are_never_empty_success() {
    let error = r#"{"protocol_version":1,"ok":false,"error":{"code":"permission_denied","message":"Check CUPS permissions."},"printers":[],"jobs":[]}"#;
    for success in [true, false] {
        assert!(parse_snapshot(error, success)
            .unwrap_err()
            .contains("permission_denied"));
    }
    for text in [
        "",
        "not json",
        "{}",
        r#"{"protocol_version":2,"ok":true}"#,
        r#"{"protocol_version":1,"ok":true,"printers":[],"jobs":[]}"#,
    ] {
        assert!(parse_snapshot(text, true).is_err());
    }
    assert!(parse_snapshot(FIXTURE, false).is_err());
    let mut invalid: Value = serde_json::from_str(FIXTURE).unwrap();
    invalid["jobs"][1]["id"] = json!(41);
    assert!(parse_snapshot(&invalid.to_string(), true).is_err());
}

#[test]
fn printing_incomplete_visibility_and_errors_keep_last_data_but_disable_actions() {
    let mut state = ready();
    state.apply(Err("CUPS inaccessible".into()), None);
    assert!(!state.ready());
    assert_eq!(state.jobs(false).len(), 1);
    assert!(state.request(0, false, Action::Hold).is_none());
    state.apply(Ok(snapshot()), None);
    assert!(state.ready());
    assert!(!state.snapshot.as_ref().unwrap().history_complete);
}

#[test]
fn printing_cancellation_requires_separate_confirmation_and_exact_identity() {
    let mut state = ready();
    assert!(state.confirm_cancel().is_none());
    assert!(state.request(0, false, Action::Cancel).is_none());
    assert!(state.pending_cancel.is_some());
    let confirmed = state.confirm_cancel().unwrap();
    let args = action_args(&confirmed, Action::Cancel);
    assert_eq!(
        args,
        [
            "job",
            "cancel",
            "--id",
            "41",
            "--printer",
            "Office",
            "--identity",
            "urn:uuid:00000000-0000-4000-8000-000000000041",
            "--json",
            "--confirm",
        ]
    );
    assert!(state.confirm_cancel().is_none());
    state.request(0, false, Action::Cancel);
    state.snapshot.as_mut().unwrap().jobs[0].identity = "new-identity".into();
    assert!(state.confirm_cancel().is_none());
    assert!(state.action_error.contains("changed"));
}

#[test]
fn printing_dismissal_busy_and_capabilities_prevent_unintended_actions() {
    let mut state = ready();
    assert!(state.request(0, true, Action::Cancel).is_none());
    assert!(state.request(0, false, Action::Release).is_none());
    assert!(state.request(0, false, Action::Hold).is_some());
    state.busy = true;
    assert!(state.request(0, false, Action::Cancel).is_none());
    state.busy = false;
    state.request(0, false, Action::Cancel);
    state.pending_cancel = None;
    assert!(state.confirm_cancel().is_none());
    let job = &mut state.snapshot.as_mut().unwrap().jobs[0];
    job.own = false;
    assert!(!job.allows(Action::Hold));
    assert!(!job.allows(Action::Cancel));
    assert!(!job.allows(Action::Release));
}

#[test]
fn printing_opaque_identity_is_one_literal_argument_and_never_a_shell() {
    let mut job = snapshot().jobs.remove(0);
    job.identity = "$(not-a-command); with spaces".into();
    let args = action_args(&job, Action::Hold);
    assert_eq!(args[7], job.identity);
    assert!(!args.iter().any(|arg| arg == "--confirm"));
}

#[test]
fn printing_action_reply_must_match_and_unknown_outcomes_are_not_success() {
    let job = snapshot().jobs.remove(0);
    assert!(parse_action(
        r#"{"protocol_version":1,"ok":true,"action":"hold","id":41}"#,
        true,
        &job,
        Action::Hold
    )
    .is_ok());
    for reply in [
        r#"{"protocol_version":1,"ok":true,"action":"cancel","id":41}"#,
        r#"{"protocol_version":1,"ok":true,"action":"hold","id":99}"#,
        r#"{"protocol_version":1,"ok":true}"#,
        r#"{"protocol_version":1,"ok":false,"error":{"code":"outcome_unknown","message":"Refresh, do not retry automatically."}}"#,
    ] {
        assert!(parse_action(reply, true, &job, Action::Hold).is_err());
    }
    let mut state = ready();
    state.apply(Ok(snapshot()), Some(Err("outcome unknown".into())));
    assert!(state.ready());
    assert_eq!(state.action_error, "outcome unknown");
    state.apply(Ok(snapshot()), None);
    assert_eq!(state.action_error, "outcome unknown");
}

#[test]
fn printing_display_strips_controls_and_bounds_untrusted_text() {
    assert_eq!(clean("fixture\n\r\0\u{1b}title"), "fixturetitle");
    assert_eq!(clean(&"x".repeat(2000)).len(), 1000);
}

#[test]
fn printing_privacy_restricted_jobs_remain_visible_without_controls() {
    let mut state = ready();
    let job = &mut state.snapshot.as_mut().unwrap().jobs[0];
    job.printer_id.clear();
    job.printer_uri.clear();
    job.user.clear();
    job.identity.clear();
    job.own = false;
    assert_eq!(state.jobs(false).len(), 1);
    assert!(state.request(0, false, Action::Cancel).is_none());
}

#[test]
fn printing_unknown_future_job_states_are_visible_without_controls() {
    let mut state = ready();
    state.snapshot.as_mut().unwrap().jobs[0].state = "future-unknown-state".into();
    assert_eq!(state.jobs(false).len(), 1);
    assert!(state.request(0, false, Action::Hold).is_none());
    assert!(state.request(0, false, Action::Cancel).is_none());
}

#[test]
#[ignore = "Read-only integration check; requires peer helper and disposable pycups interpreter paths"]
fn printing_peer_status_read_only() {
    let python = std::env::var("SMPL_PRINTING_CHECK_PYTHON").expect("private interpreter path");
    let helper = std::env::var("SMPL_PRINTING_CHECK_HELPER").expect("OS helper path");
    let output = Command::new("timeout")
        .args([
            "--kill-after=1s",
            "20s",
            &python,
            &helper,
            "status",
            "--json",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let response = String::from_utf8(output.stdout).unwrap();
    let status = parse_snapshot(&response, output.status.success())
        .expect("OS peer status must match the native consumer");
    println!("Local peer protocol v1 accepted: {} queues, {} active jobs, {} history jobs; active_complete={}, history_complete={}; warning codes: {}",
        status.printers.len(), status.jobs.iter().filter(|job| !job.historical()).count(),
        status.jobs.iter().filter(|job| job.historical()).count(),
        status.active_complete, status.history_complete,
        status.warnings.iter().map(|warning| warning.code.as_str()).collect::<Vec<_>>().join(", "));
}

#[test]
#[ignore = "Read-only integration check; requires the matching installed smplos-printing helper"]
fn printing_local_status_read_only() {
    let status = System
        .status()
        .expect("local CUPS status must be available");
    println!("Local printing protocol v1: {} queues, {} active jobs, {} retained history jobs; active_complete={}, history_complete={}",
        status.printers.len(), status.jobs.iter().filter(|job| !job.historical()).count(),
        status.jobs.iter().filter(|job| job.historical()).count(),
        status.active_complete, status.history_complete);
}
