//! Regression coverage for the notification click -> dispatch wiring.
//!
//! These tests exercise the exact shared function the Slint `activate`
//! callback invokes (`dispatch_notification`) plus the pure decision
//! logic (`decide_dispatch`), with every process effect faked through
//! the injected `run` closure. No real helper, dunstctl, bus, app or
//! history is touched; the recorded commands are the evidence.

use crate::backend::{
    decide_dispatch, dispatch_notification, session_env_passthrough, DispatchCmd,
    DispatchOutcome, Notification,
};
use std::cell::RefCell;
use std::rc::Rc;

fn notif(id: i32, appname: &str, desktop_entry: &str, action: &str) -> Notification {
    Notification {
        id,
        appname: appname.to_string(),
        desktop_entry: desktop_entry.to_string(),
        icon: String::new(),
        summary: String::new(),
        body: String::new(),
        date: String::new(),
        time: String::new(),
        action: action.to_string(),
    }
}

fn recording_run(record: &Rc<RefCell<Vec<DispatchCmd>>>) -> impl FnMut(&DispatchCmd) -> bool + use<'_> {
    move |cmd| {
        record.borrow_mut().push(cmd.clone());
        true
    }
}

#[test]
fn stale_id_is_not_dispatched() {
    let list = vec![notif(1, "signal", "signal-desktop", "")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 99, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Stale);
    assert!(record.borrow().is_empty(), "stale click must not run any effect");
}

#[test]
fn default_path_prefers_desktop_entry_and_strips_suffix() {
    let list = vec![notif(7, "signal", "signal-desktop.desktop", "")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 7, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Dispatched);
    assert_eq!(
        *record.borrow(),
        vec![DispatchCmd::FocusOrLaunch {
            target: "signal-desktop".into()
        }]
    );
}

#[test]
fn default_path_falls_back_to_appname() {
    let list = vec![notif(8, "Brave", "", "")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 8, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Dispatched);
    assert_eq!(
        *record.borrow(),
        vec![DispatchCmd::FocusOrLaunch {
            target: "Brave".into()
        }]
    );
}

#[test]
fn explicit_action_wins_over_target() {
    let list = vec![notif(9, "smplos", "smplos.desktop", "smplos-update")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 9, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Dispatched);
    assert_eq!(
        *record.borrow(),
        vec![DispatchCmd::ShellAction {
            action: "smplos-update".into()
        }]
    );
}

#[test]
fn webapp_log_special_action_resolves_to_webapp_log() {
    let list = vec![notif(10, "launch-webapp", "", "__OPEN_WEBAPP_LOG__")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 10, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Dispatched);
    assert_eq!(*record.borrow(), vec![DispatchCmd::WebappLog]);
}

#[test]
fn empty_target_and_action_fails_without_running_any_effect() {
    let list = vec![notif(11, "", "", "")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 11, &mut recording_run(&record));
    assert_eq!(outcome, DispatchOutcome::Failed);
    assert!(record.borrow().is_empty(), "no target means no effect may run");
}

#[test]
fn failed_effect_reports_failed_but_was_attempted() {
    let list = vec![notif(12, "signal", "signal-desktop", "")];
    let record = Rc::new(RefCell::new(Vec::new()));
    let outcome = dispatch_notification(&list, 12, &mut |cmd| {
        record.borrow_mut().push(cmd.clone());
        false
    });
    assert_eq!(outcome, DispatchOutcome::Failed);
    assert_eq!(record.borrow().len(), 1, "effect was attempted, spawn failed");
}

#[test]
fn decide_dispatch_matches_documented_precedence() {
    assert_eq!(
        decide_dispatch("", "", "__OPEN_WEBAPP_LOG__"),
        DispatchCmd::WebappLog
    );
    assert_eq!(
        decide_dispatch("a", "", "echo hi"),
        DispatchCmd::ShellAction {
            action: "echo hi".into()
        }
    );
    assert_eq!(
        decide_dispatch("a", "b.desktop", ""),
        DispatchCmd::FocusOrLaunch { target: "b".into() }
    );
    assert_eq!(decide_dispatch("", "", ""), DispatchCmd::None);
}

#[test]
fn session_env_forwards_hyprland_signature_and_dbus_bus() {
    let env = session_env_passthrough(
        "wayland-0",
        ":1",
        "/run/user/1000",
        "synthetic-sig-12345",
        "unix:path=/run/user/1000/bus",
    );
    let map: std::collections::HashMap<_, _> = env.into_iter().collect();
    // The installed focus-or-launch helper contract: IS_HYPRLAND is true
    // only when the signature is nonempty, and gtk-launch activation needs
    // the session bus address.
    assert_eq!(
        map.get("HYPRLAND_INSTANCE_SIGNATURE").map(String::as_str),
        Some("synthetic-sig-12345")
    );
    assert_eq!(
        map.get("DBUS_SESSION_BUS_ADDRESS").map(String::as_str),
        Some("unix:path=/run/user/1000/bus")
    );
    // Pre-existing forwarding is preserved, not replaced.
    assert_eq!(map.get("WAYLAND_DISPLAY").map(String::as_str), Some("wayland-0"));
    assert_eq!(map.get("DISPLAY").map(String::as_str), Some(":1"));
    assert_eq!(map.get("XDG_RUNTIME_DIR").map(String::as_str), Some("/run/user/1000"));
    assert_eq!(map.len(), 5);
}

#[test]
fn session_env_omits_empty_discriminators() {
    let env = session_env_passthrough("wayland-0", "", "/run/user/1000", "", "");
    let keys: Vec<&str> = env.iter().map(|(k, _)| k.as_str()).collect();
    // Empty values must not be forwarded as empty discriminators.
    assert_eq!(keys, vec!["WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"]);
}
