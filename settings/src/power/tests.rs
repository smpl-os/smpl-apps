use super::*;
use std::cell::{Cell, RefCell};

const STOCK: &str = "# user-owned preferences\n\
general {\n\
    lock_cmd = pidof hyprlock || lock-screen\n\
    before_sleep_cmd = loginctl lock-session\n\
    after_sleep_cmd = loginctl lock-session; sleep 0.5; hyprctl dispatch dpms on\n\
    ignore_dbus_inhibit = false # keep this\n\
}\n\
\n\
listener {\n\
    timeout = 300 # lock\n\
    on-timeout = loginctl lock-session\n\
    ignore_inhibit = true\n\
}\n\
listener {\n\
    timeout = 330 # exact stock delay\n\
    on-timeout = systemd-detect-virt -q || hyprctl dispatch dpms off\n\
    on-resume = notify-send awake\n\
}\n\
listener {\n\
    timeout = 719 # user customization\n\
    on-timeout = systemd-detect-virt -q || systemctl suspend\n\
}\n";

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let path = temporary(&std::env::temp_dir(), "smpl-power-test");
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn config(&self, text: &str) -> PathBuf {
        let path = self.0.join("hypridle.conf");
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct Mock {
    calls: RefCell<Vec<String>>,
    shows: Cell<usize>,
    fail_restart: bool,
    stale: bool,
    exits: bool,
    unmanaged: bool,
    wrong_config: bool,
    invalid: bool,
    fail_profile: bool,
    mismatch_profile: bool,
    helper: bool,
    fail_helper: bool,
    no_rules: Cell<Option<bool>>,
    profile: Cell<i32>,
    concurrent_edit: Option<PathBuf>,
}

impl Runtime for Mock {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        self.calls
            .borrow_mut()
            .push(format!("{program} {}", args.join(" ")));
        let mut success = true;
        let stdout = match (program, args.first().copied()) {
            ("systemctl", _) if args.contains(&"show") => {
                let count = self.shows.get();
                self.shows.set(count + 1);
                let pid = if count == 0 || self.stale { 42 } else { 43 };
                if self.exits && count > 0 {
                    "LoadState=loaded\nActiveState=failed\nSubState=failed\nMainPID=0".into()
                } else {
                    format!("LoadState=loaded\nActiveState=active\nSubState=running\nMainPID={pid}")
                }
            }
            ("systemctl", _) if args.contains(&"restart") => {
                success = !self.fail_restart;
                String::new()
            }
            ("pgrep", _) => {
                if self.unmanaged {
                    "42\n99".into()
                } else {
                    "42".into()
                }
            }
            ("powerprofilesctl", Some("set")) => {
                success = !self.fail_profile;
                if success && !self.mismatch_profile {
                    self.profile
                        .set(PROFILES.iter().position(|&p| p == args[1]).unwrap() as i32);
                }
                String::new()
            }
            ("powerprofilesctl", Some("get")) => PROFILES[self.profile.get() as usize].into(),
            (HELPER, Some("--check")) => {
                success = !self.fail_helper;
                String::new()
            }
            _ => panic!("Unexpected command: {program} {args:?}"),
        };
        Ok(Output {
            success,
            code: Some(if success { 0 } else { 1 }),
            stdout,
            stderr: if success {
                "".into()
            } else {
                "mock failure".into()
            },
        })
    }

    fn helper_available(&self) -> bool {
        self.helper
    }

    fn validate(&self, _: &str, no_rules: bool) -> Result<()> {
        self.no_rules.set(Some(no_rules));
        if let Some(path) = &self.concurrent_edit {
            fs::write(path, "# concurrent edit\n").unwrap();
        }
        if self.invalid {
            Err(unsupported("native parse error"))
        } else {
            Ok(())
        }
    }

    fn daemon_uses_config(&self, _: u32, _: &Path) -> Result<()> {
        if self.wrong_config {
            Err(unsupported("different config"))
        } else {
            Ok(())
        }
    }

    fn settle(&self) {}
}

#[test]
fn exact_values_and_custom_seconds_are_not_rounded() {
    assert_eq!(Config::parse(STOCK).unwrap().seconds, [300, 330, 719, 0]);
    assert_eq!(preset_index(330), -1);
    assert_eq!(preset_index(719), -1);
    assert_eq!(preset_index(0), 4);
}

#[test]
fn timeout_labels_show_every_exact_component_without_saved_diagnostics() {
    for (seconds, expected) in [
        (0, "Never"),
        (1, "1 s"),
        (59, "59 s"),
        (60, "1 min"),
        (330, "5 min 30 s"),
        (719, "11 min 59 s"),
        (3600, "1 h"),
        (3601, "1 h 1 s"),
        (3660, "1 h 1 min"),
        (3661, "1 h 1 min 1 s"),
        (28800, "8 h"),
        (u32::MAX, "1193046 h 28 min 15 s"),
    ] {
        assert_eq!(timeout_label(seconds), expected);
    }
}

#[test]
fn one_timer_edit_preserves_every_other_byte() {
    let changed = Config::parse(STOCK)
        .unwrap()
        .change(Action::Suspend, 601, false)
        .unwrap();
    assert_eq!(changed, STOCK.replace("timeout = 719", "timeout = 601"));
    assert_eq!(Config::parse(&changed).unwrap().seconds, [300, 330, 601, 0]);
}

#[test]
fn indentation_crlf_comments_and_missing_final_newline_survive() {
    let text =
        "listener {\r\n\t timeout\t= 330   # keep\r\n\ton-timeout = hyprctl dispatch dpms off\r\n}";
    let changed = Config::parse(text)
        .unwrap()
        .change(Action::Dpms, 777, false)
        .unwrap();
    assert_eq!(changed, text.replace("330", "777"));
}

#[test]
fn never_is_reversible_and_preserves_custom_listener_properties() {
    let disabled = Config::parse(STOCK)
        .unwrap()
        .change(Action::Dpms, 0, false)
        .unwrap();
    assert_eq!(Config::parse(&disabled).unwrap().seconds, [300, 0, 719, 0]);
    assert!(disabled.contains("# smpl-settings-disabled: on-resume = notify-send awake"));
    let enabled = Config::parse(&disabled)
        .unwrap()
        .change(Action::Dpms, 331, true)
        .unwrap();
    assert!(enabled.contains("on-resume = notify-send awake"));
    assert!(enabled.contains("timeout = 331 # exact stock delay"));
    assert!(enabled.contains("smplos-hypr-dpms off"));
}

#[test]
fn lock_never_preserves_manual_lock_and_disables_sleep_lock() {
    let changed = Config::parse(STOCK)
        .unwrap()
        .change(Action::Lock, 0, false)
        .unwrap();
    assert!(changed.contains("lock_cmd = pidof hyprlock || lock-screen"));
    assert!(changed.contains("before_sleep_cmd = \n"));
    assert!(changed.contains("after_sleep_cmd = hyprctl dispatch dpms on"));
    let restored = Config::parse(&changed)
        .unwrap()
        .change(Action::Lock, 300, false)
        .unwrap();
    assert_eq!(restored, STOCK);
}

#[test]
fn selecting_never_also_corrects_stock_sleep_hooks_without_a_lock_listener() {
    let text = "general {\n lock_cmd = lock-screen\n before_sleep_cmd = loginctl lock-session\n}\n";
    let changed = Config::parse(text)
        .unwrap()
        .change(Action::Lock, 0, false)
        .unwrap();
    assert!(changed.contains("before_sleep_cmd = \n"));
    assert!(changed.contains("lock_cmd = lock-screen"));
}

#[test]
fn all_never_is_an_intentional_empty_rule_set() {
    let mut text = STOCK.to_owned();
    for action in [
        Action::Lock,
        Action::Dpms,
        Action::Suspend,
        Action::Shutdown,
    ] {
        text = Config::parse(&text)
            .unwrap()
            .change(action, 0, false)
            .unwrap();
    }

    assert_eq!(Config::parse(&text).unwrap().seconds, [0; 4]);
    assert!(text.contains("lock_cmd = pidof hyprlock || lock-screen"));
    assert!(Config::parse("").is_ok());
}

#[test]
fn all_never_save_keeps_manual_lock_daemon_instead_of_stopping_it() {
    let temp = Temp::new();
    let path = temp.config("general {\n lock_cmd = lock-screen\n}\n");
    let runtime = Mock::default();
    save_timer(&path, &runtime, Action::Shutdown, 0).unwrap();
    assert_eq!(runtime.no_rules.get(), Some(true));
    assert!(read(&path).unwrap().contains("lock_cmd = lock-screen"));
    let calls = runtime.calls.borrow();
    assert!(calls.iter().any(|c| c.contains("restart")));
    assert!(!calls
        .iter()
        .any(|c| c.contains(" stop ") || c.contains("disable")));
}

#[test]
fn refuses_unsupported_ambiguous_and_invalid_configs() {
    for text in [
        "source = extra.conf\n".into(),
        "$timer = 123\n".into(),
        "general {\n".into(),
        "listener { timeout = 300 }\n".into(),
        STOCK.replace("timeout = 330", "timeout = nope"),
        STOCK.replace("timeout = 330", "timeout = 0"),
        STOCK.replace("timeout = 330", "timeout = 4294967295"),
        STOCK.replace("timeout = 330", "timeout = 330\n timeout = 600"),
        STOCK.replace("systemctl suspend", "echo suspend"),
        STOCK.replace("systemctl suspend", "systemctl hibernate"),
        STOCK.replace("systemctl suspend", "systemctl suspend; custom-hook"),
        STOCK.replace("systemctl suspend", "loginctl lock-session"),
        STOCK.replace("on-resume = notify-send awake", "source = custom.conf"),
        STOCK.replace("on-resume = notify-send awake", "on-resume = echo '#'"),
    ] {
        assert!(Config::parse(&text).is_err(), "{text}");
    }
}

#[test]
fn custom_general_sleep_hooks_are_preserved_or_explicitly_refused() {
    let custom = STOCK.replace(
        "before_sleep_cmd = loginctl lock-session",
        "before_sleep_cmd = custom-sleep",
    );
    assert!(Config::parse(&custom)
        .unwrap()
        .change(Action::Lock, 0, false)
        .is_err());
    let changed = Config::parse(&custom)
        .unwrap()
        .change(Action::Shutdown, 60, false)
        .unwrap();
    assert!(changed.contains("before_sleep_cmd = custom-sleep"));
}

#[test]
fn old_lua_and_helper_commands_are_recognized_and_narrowly_migrated() {
    for command in ["hyprctl dispatch dpms", "smplos-hypr-dpms"] {
        let text = STOCK.replace("hyprctl dispatch dpms", command);
        assert_eq!(Config::parse(&text).unwrap().seconds[1], 330);
    }
    let lua = STOCK
        .replace(
            "hyprctl dispatch dpms on",
            "hyprctl dispatch \"hl.dsp.dpms({state='on'})\"",
        )
        .replace(
            "hyprctl dispatch dpms off",
            "hyprctl dispatch \"hl.dsp.dpms({state='off'})\"",
        );
    let changed = Config::parse(&lua)
        .unwrap()
        .change(Action::Shutdown, 123, true)
        .unwrap();
    assert!(changed.contains("smplos-hypr-dpms on"));
    assert!(changed.contains("smplos-hypr-dpms off"));
    assert!(!changed.contains("hl.dsp"));
    assert!(changed.contains("notify-send awake"));
}

#[test]
fn new_or_restored_dpms_requires_os_helper() {
    let text = "general {\n lock_cmd = lock-screen\n}\n";
    assert!(Config::parse(text)
        .unwrap()
        .change(Action::Dpms, 60, false)
        .is_err());
    let enabled = Config::parse(text)
        .unwrap()
        .change(Action::Dpms, 60, true)
        .unwrap();
    assert!(enabled.contains("smplos-hypr-dpms off"));
    let disabled = Config::parse(&enabled)
        .unwrap()
        .change(Action::Dpms, 0, true)
        .unwrap();
    assert!(Config::parse(&disabled)
        .unwrap()
        .change(Action::Dpms, 60, false)
        .is_err());
}

#[test]
fn unsupported_live_dpms_capability_is_saved_but_never_claimed_applied() {
    let temp = Temp::new();
    let path = temp.config(STOCK);
    let runtime = Mock {
        helper: true,
        fail_helper: true,
        ..Mock::default()
    };
    let outcome = save_timer(&path, &runtime, Action::Dpms, 600).unwrap();
    assert!(matches!(outcome, SaveOutcome::ApplicationUnconfirmed(_)));
    assert!(read(&path).unwrap().contains("smplos-hypr-dpms off"));
    assert!(!runtime.calls.borrow().iter().any(|c| c.contains("restart")));
}

#[test]
fn save_rereads_external_changes_and_preserves_exact_values() {
    let temp = Temp::new();
    let path = temp.config(STOCK);
    let _old_view = Config::parse(&read(&path).unwrap()).unwrap();
    fs::write(&path, STOCK.replace("timeout = 719", "timeout = 1171")).unwrap();
    let runtime = Mock::default();
    let outcome = save_timer(&path, &runtime, Action::Shutdown, 123).unwrap();
    assert!(matches!(outcome, SaveOutcome::Restarted));
    assert_eq!(
        Config::parse(&read(&path).unwrap()).unwrap().seconds,
        [300, 330, 1171, 123]
    );
    assert!(!runtime
        .calls
        .borrow()
        .iter()
        .any(|s| s.contains("poweroff")));
}

#[test]
fn atomic_save_detects_concurrent_edits_and_preserves_permissions() {
    let temp = Temp::new();
    let path = temp.config(STOCK);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    atomic_save(&path, STOCK, "# changed\n").unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert!(matches!(
        atomic_save(&path, STOCK, ""),
        Err(Error::Conflict)
    ));
    assert_eq!(read(&path).unwrap(), "# changed\n");
    assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 1);
}

#[test]
fn parse_native_validation_read_and_write_failures_never_apply() {
    let temp = Temp::new();
    let path = temp.config(STOCK);
    let runtime = Mock {
        invalid: true,
        ..Mock::default()
    };
    assert!(save_timer(&path, &runtime, Action::Shutdown, 60).is_err());
    assert_eq!(read(&path).unwrap(), STOCK);
    assert!(runtime.calls.borrow().is_empty());
    assert!(read(&temp.0.join("missing")).is_err());
    assert!(read(&temp.0).is_err());
    assert!(atomic_save(&temp.0.join("missing/child"), "", "").is_err());
    fs::write(&path, "source = custom.conf\n").unwrap();
    assert!(save_timer(&path, &runtime, Action::Shutdown, 60).is_err());
    assert!(runtime.calls.borrow().is_empty());
}

#[test]
fn concurrent_write_after_validation_does_not_overwrite_or_restart() {
    let temp = Temp::new();
    let path = temp.config(STOCK);
    let runtime = Mock {
        concurrent_edit: Some(path.clone()),
        ..Mock::default()
    };
    assert!(matches!(
        save_timer(&path, &runtime, Action::Shutdown, 60),
        Err(Error::Conflict)
    ));
    assert_eq!(read(&path).unwrap(), "# concurrent edit\n");
    assert!(runtime.calls.borrow().is_empty());
}

#[test]
fn failed_restart_stale_pid_wrong_config_and_unmanaged_daemon_are_truthful() {
    for runtime in [
        Mock {
            fail_restart: true,
            ..Mock::default()
        },
        Mock {
            stale: true,
            ..Mock::default()
        },
        Mock {
            exits: true,
            ..Mock::default()
        },
        Mock {
            unmanaged: true,
            ..Mock::default()
        },
        Mock {
            wrong_config: true,
            ..Mock::default()
        },
    ] {
        let temp = Temp::new();
        let path = temp.config(STOCK);
        let outcome = save_timer(&path, &runtime, Action::Shutdown, 60).unwrap();
        assert!(
            matches!(outcome, SaveOutcome::ApplicationUnconfirmed(_)),
            "{outcome:?}"
        );
        assert_eq!(Config::parse(&read(&path).unwrap()).unwrap().seconds[3], 60);
        if runtime.unmanaged || runtime.wrong_config {
            assert!(!runtime.calls.borrow().iter().any(|s| s.contains("restart")));
        }
        assert!(!runtime
            .calls
            .borrow()
            .iter()
            .any(|s| s.contains("pkill") || s.contains("setsid")));
    }
}

#[test]
fn profile_set_failure_and_readback_mismatch_are_errors() {
    let runtime = Mock {
        fail_profile: true,
        ..Mock::default()
    };
    assert!(set_profile(&runtime, 2).is_err());
    assert_eq!(profile(&runtime).unwrap(), 0);
    let runtime = Mock {
        mismatch_profile: true,
        ..Mock::default()
    };
    assert!(set_profile(&runtime, 2).is_err());
    assert!(set_profile(&runtime, 42).is_err());
    let runtime = Mock::default();
    set_profile(&runtime, 2).unwrap();
    assert_eq!(profile(&runtime).unwrap(), 2);
}

#[test]
fn diagnostic_check_distinguishes_no_rules_from_invalid_configuration() {
    let disconnected = "[CRITICAL] Couldn't connect to a wayland compositor";
    assert!(validate_diagnostics(disconnected, false).is_ok());
    let no_rules = format!("[ERR] Config has errors:\nNo rules configured\nProceeding ignoring faulty entries\n{disconnected}");
    assert!(validate_diagnostics(&no_rules, true).is_ok());
    assert!(validate_diagnostics(&no_rules, false).is_err());
    assert!(
        validate_diagnostics(&format!("Config error at line 3\n{disconnected}"), true).is_err()
    );
    let unknown = format!("[ERR] Config has errors:\nconfig option <something> does not exist.\nNo rules configured\nProceeding ignoring faulty entries\n{disconnected}");
    assert!(validate_diagnostics(&unknown, true).is_err());
    assert!(validate_diagnostics("Could not start", false).is_err());
}

#[test]
#[ignore = "requires installed hypridle; uses isolated HOME, Wayland and D-Bus"]
fn native_parser_isolated_from_real_session() {
    System.validate(STOCK, false).unwrap();
    let mut text = STOCK.to_owned();
    for action in [Action::Lock, Action::Dpms, Action::Suspend] {
        text = Config::parse(&text)
            .unwrap()
            .change(action, 0, false)
            .unwrap();
    }
    System.validate(&text, true).unwrap();
    assert!(System
        .validate(&STOCK.replace("timeout = 330", "timeout = broken"), false)
        .is_err());
    assert!(System
        .validate(
            &STOCK.replace("ignore_inhibit = true", "unknown_property = true"),
            false
        )
        .is_err());
}
