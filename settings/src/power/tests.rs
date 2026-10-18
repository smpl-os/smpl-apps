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

// The hypridle.conf smplOS ships, verbatim. Refresh it with
// `scripts/check-smplos-stock-hypridle.sh --update`.
const SMPLOS_STOCK: &str = include_str!("fixtures/smplos-hypridle.conf");

const ACTIONS: [Action; 4] = [
    Action::Lock,
    Action::Dpms,
    Action::Suspend,
    Action::Shutdown,
];
const LOCK_PREFIX: &str = "loginctl lock-session; sleep 0.5; ";
const LUA_DPMS_ON: &str = "hyprctl dispatch \"hl.dsp.dpms({state='on'})\"";
const HELPER_WAKE_CHAIN: &str =
    "smplos-hypr-dpms on; sleep 0.2; hyprctl reload; sleep 0.2; smplos-hypr-dpms on";

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
            Err(Error::Command("native parse error".into()))
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

fn replace_once(text: &str, from: &str, to: &str) -> String {
    assert_eq!(
        text.matches(from).count(),
        1,
        "expected exactly one {from:?}"
    );
    text.replacen(from, to, 1)
}

fn with_after_sleep(base: &str, command: &str) -> String {
    let line = base
        .lines()
        .find(|line| line.trim_start().starts_with("after_sleep_cmd"))
        .expect("after_sleep_cmd in the base config");
    replace_once(base, line, &format!("    after_sleep_cmd = {command}"))
}

// A general-block hook as hypridle reads it, without comments.
fn hook<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    config.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        let value = value.split('#').next().unwrap_or_default();
        (name.trim() == key).then_some(value.trim())
    })
}

fn assert_every_change_succeeds(config: &str, label: &str) {
    let before = Config::parse(config)
        .unwrap_or_else(|e| panic!("{label}: {e}"))
        .seconds;
    for helper in [false, true] {
        for action in ACTIONS {
            for &seconds in PRESETS {
                let step = format!("{label}: {action:?} -> {seconds} s (helper {helper})");
                let changed = Config::parse(config)
                    .unwrap()
                    .change(action, seconds, helper)
                    .unwrap_or_else(|e| panic!("{step}: {e}"));
                let mut expected = before;
                expected[action as usize] = seconds;
                assert_eq!(Config::parse(&changed).unwrap().seconds, expected, "{step}");
            }
        }
    }
}

// What must hold for every hypridle.conf smplOS or Settings has produced.
fn check_hypridle_conf(config: &str, label: &str) {
    assert_every_change_succeeds(config, label);

    let parsed = Config::parse(config).unwrap_or_else(|e| panic!("{label}: {e}"));
    let lock = parsed.seconds[0];
    let never = parsed
        .change(Action::Lock, 0, false)
        .unwrap_or_else(|e| panic!("{label}: Lock -> Never failed: {e}"));
    assert_eq!(Config::parse(&never).unwrap().seconds[0], 0, "{label}");
    if hook(config, "before_sleep_cmd").is_some() {
        assert_eq!(hook(&never, "before_sleep_cmd"), Some(""), "{label}");
    }
    let wake = hook(config, "after_sleep_cmd");
    assert_eq!(
        hook(&never, "after_sleep_cmd"),
        wake.map(|wake| wake.strip_prefix(LOCK_PREFIX).unwrap_or(wake)),
        "{label}: Never may only drop the lock step of after_sleep_cmd"
    );
    assert_every_change_succeeds(&never, &format!("{label} with lock set to Never"));

    let restored = Config::parse(&never)
        .unwrap()
        .change(Action::Lock, if lock == 0 { 300 } else { lock }, false)
        .unwrap_or_else(|e| panic!("{label}: turning the lock timer back on failed: {e}"));
    assert_eq!(
        hook(&restored, "before_sleep_cmd"),
        Some("loginctl lock-session"),
        "{label}"
    );
    if let Some(wake) = hook(&restored, "after_sleep_cmd").filter(|wake| !wake.is_empty()) {
        assert!(wake.starts_with(LOCK_PREFIX), "{label}: {wake}");
    }
    let stock_hooks = hook(config, "before_sleep_cmd") == Some("loginctl lock-session")
        && wake.is_none_or(|wake| wake.starts_with(LOCK_PREFIX));
    if lock != 0 && stock_hooks {
        assert_eq!(restored, config, "{label}: Never and back changed the file");
    }
}

// Every wake-up command smplOS or an earlier Settings version has written.
fn accepted_wake_commands() -> Vec<(&'static str, String)> {
    vec![
        ("legacy dispatch", "hyprctl dispatch dpms on".into()),
        ("lua dispatch", LUA_DPMS_ON.into()),
        (
            "lua wake chain",
            format!("{LUA_DPMS_ON}; sleep 0.2; hyprctl reload; sleep 0.2; {LUA_DPMS_ON}"),
        ),
        ("helper", "smplos-hypr-dpms on".into()),
        ("helper wake chain", HELPER_WAKE_CHAIN.into()),
    ]
}

fn accepted_configs() -> Vec<(String, String)> {
    let mut configs = Vec::new();
    for (name, wake) in accepted_wake_commands() {
        for (prefix, how) in [(LOCK_PREFIX, "with"), ("", "without")] {
            configs.push((
                format!("{name}, {how} lock prefix"),
                with_after_sleep(SMPLOS_STOCK, &format!("{prefix}{wake}")),
            ));
        }
    }
    configs
}

fn fixtures(dir: &str) -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/power/fixtures")
        .join(dir);
    let mut paths: Vec<_> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "{} holds no fixtures", dir.display());
    paths
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect()
}

#[test]
fn smplos_stock_file_supports_every_timer_change() {
    check_hypridle_conf(SMPLOS_STOCK, "smplOS stock");
    check_hypridle_conf(&SMPLOS_STOCK.replace('\n', "\r\n"), "smplOS stock, CRLF");
}

// The stock file with the lock timer off, written out by hand so the tests that
// use it do not depend on the editor they check.
fn stock_with_lock_never() -> String {
    let never = replace_once(
        SMPLOS_STOCK,
        "    before_sleep_cmd = loginctl lock-session\n    after_sleep_cmd = loginctl lock-session; sleep 0.5; smplos-hypr-dpms on",
        "    before_sleep_cmd = \n    after_sleep_cmd = smplos-hypr-dpms on",
    );
    replace_once(
        &never,
        "listener {\n    timeout = 300\n    on-timeout = loginctl lock-session\n    ignore_inhibit = true\n}\n",
        "# smpl-settings-disabled: listener {\n\
         # smpl-settings-disabled:     timeout = 300\n\
         # smpl-settings-disabled:     on-timeout = loginctl lock-session\n\
         # smpl-settings-disabled:     ignore_inhibit = true\n\
         # smpl-settings-disabled: }\n",
    )
}

#[test]
fn lock_never_on_the_stock_file_drops_only_the_lock_step_and_disables_the_listener() {
    let changed = Config::parse(SMPLOS_STOCK)
        .unwrap()
        .change(Action::Lock, 0, false)
        .unwrap();
    assert_eq!(hook(&changed, "before_sleep_cmd"), Some(""));
    assert_eq!(hook(&changed, "after_sleep_cmd"), Some(HELPER_WAKE_CHAIN));
    assert_eq!(changed, stock_with_lock_never());
    assert_eq!(Config::parse(&changed).unwrap().seconds, [0, 330, 600, 0]);
    for helper in [false, true] {
        let restored = Config::parse(&changed)
            .unwrap()
            .change(Action::Lock, 300, helper)
            .unwrap();
        assert_eq!(restored, SMPLOS_STOCK, "helper {helper}");
    }
}

#[test]
fn timer_edits_on_the_stock_file_preserve_every_other_byte() {
    for helper in [false, true] {
        for (action, seconds, from, to) in [
            (Action::Lock, 1800, "timeout = 300\n", "timeout = 1800\n"),
            (Action::Dpms, 345, "timeout = 330\n", "timeout = 345\n"),
            (Action::Suspend, 601, "timeout = 600\n", "timeout = 601\n"),
        ] {
            let changed = Config::parse(SMPLOS_STOCK)
                .unwrap()
                .change(action, seconds, helper)
                .unwrap();
            assert_eq!(
                changed,
                replace_once(SMPLOS_STOCK, from, to),
                "{action:?} (helper {helper})"
            );
        }
    }
    let added = Config::parse(SMPLOS_STOCK)
        .unwrap()
        .change(Action::Shutdown, 900, true)
        .unwrap();
    assert!(added.starts_with(SMPLOS_STOCK));
    assert_eq!(Config::parse(&added).unwrap().seconds, [300, 330, 600, 900]);
}

#[test]
fn pressing_never_on_the_stock_file_saves_and_the_lock_timer_comes_back() {
    let temp = Temp::new();
    let path = temp.config(SMPLOS_STOCK);
    let runtime = || Mock {
        helper: true,
        ..Mock::default()
    };
    let outcome = save_timer(&path, &runtime(), Action::Lock, 0).unwrap();
    assert!(matches!(outcome, SaveOutcome::Restarted), "{outcome:?}");
    let saved = read(&path).unwrap();
    assert_eq!(Config::parse(&saved).unwrap().seconds, [0, 330, 600, 0]);
    assert_eq!(hook(&saved, "before_sleep_cmd"), Some(""));
    assert_eq!(hook(&saved, "after_sleep_cmd"), Some(HELPER_WAKE_CHAIN));
    let outcome = save_timer(&path, &runtime(), Action::Lock, 300).unwrap();
    assert!(matches!(outcome, SaveOutcome::Restarted), "{outcome:?}");
    assert_eq!(read(&path).unwrap(), SMPLOS_STOCK);
}

#[test]
fn every_accepted_after_sleep_cmd_shape_supports_every_timer_change() {
    for (label, config) in accepted_configs() {
        check_hypridle_conf(&config, &label);
    }
}

#[test]
fn lock_never_and_back_keep_every_accepted_wake_command_verbatim() {
    for (name, wake) in accepted_wake_commands() {
        let locked = format!("{LOCK_PREFIX}{wake}");
        for command in [locked.as_str(), wake.as_str()] {
            let label = format!("{name}: after_sleep_cmd = {command}");
            let start = with_after_sleep(SMPLOS_STOCK, command);
            let never = Config::parse(&start)
                .unwrap()
                .change(Action::Lock, 0, false)
                .unwrap_or_else(|e| panic!("{label}: Lock -> Never failed: {e}"));
            assert_eq!(hook(&never, "before_sleep_cmd"), Some(""), "{label}");
            assert_eq!(
                hook(&never, "after_sleep_cmd"),
                Some(wake.as_str()),
                "{label}"
            );
            assert_eq!(Config::parse(&never).unwrap().seconds[0], 0, "{label}");
            let restored = Config::parse(&never)
                .unwrap()
                .change(Action::Lock, 300, false)
                .unwrap_or_else(|e| panic!("{label}: turning lock back on failed: {e}"));
            assert_eq!(restored, with_after_sleep(SMPLOS_STOCK, &locked), "{label}");
        }
    }
}

#[test]
fn lock_changes_with_the_helper_migrate_single_wake_commands_but_keep_chains() {
    let lua_chain = format!("{LUA_DPMS_ON}; sleep 0.2; hyprctl reload; sleep 0.2; {LUA_DPMS_ON}");
    for (wake, migrated) in [
        ("hyprctl dispatch dpms on", "smplos-hypr-dpms on"),
        (LUA_DPMS_ON, "smplos-hypr-dpms on"),
        (lua_chain.as_str(), lua_chain.as_str()),
        ("smplos-hypr-dpms on", "smplos-hypr-dpms on"),
        (HELPER_WAKE_CHAIN, HELPER_WAKE_CHAIN),
    ] {
        let start = with_after_sleep(SMPLOS_STOCK, &format!("{LOCK_PREFIX}{wake}"));
        let never = Config::parse(&start)
            .unwrap()
            .change(Action::Lock, 0, true)
            .unwrap();
        assert_eq!(hook(&never, "after_sleep_cmd"), Some(migrated), "{wake}");
        let restored = Config::parse(&never)
            .unwrap()
            .change(Action::Lock, 300, true)
            .unwrap();
        assert_eq!(
            restored,
            with_after_sleep(SMPLOS_STOCK, &format!("{LOCK_PREFIX}{migrated}")),
            "{wake}"
        );
    }
}

#[test]
fn custom_after_sleep_cmd_is_still_refused_with_and_without_the_lock_prefix() {
    let never = stock_with_lock_never();
    for wake in [
        "notify-send hi; smplos-hypr-dpms on",
        "hyprctl reload",
        "smplos-hypr-dpms on;",
        "smplos-hypr-dpms on; sleep abc",
        "smplos-hypr-dpms on && reboot",
        "smplos-hypr-dpms on; sleep $(reboot)",
    ] {
        for command in [format!("{LOCK_PREFIX}{wake}"), wake.to_owned()] {
            let locked = with_after_sleep(SMPLOS_STOCK, &command);
            let unlocked = with_after_sleep(&never, &command);
            for (config, seconds) in [(&locked, 0), (&unlocked, 300)] {
                let result = Config::parse(config)
                    .unwrap()
                    .change(Action::Lock, seconds, false);
                match result {
                    Err(Error::Unsupported(reason)) => {
                        assert!(reason.contains("after_sleep_cmd"), "{command}: {reason}")
                    }
                    other => panic!("{command}: expected a refusal, got {other:?}"),
                }
            }
            // Timers that leave the sleep hooks alone keep working.
            for (action, seconds) in [
                (Action::Lock, 600),
                (Action::Dpms, 120),
                (Action::Suspend, 1200),
                (Action::Shutdown, 900),
            ] {
                Config::parse(&locked)
                    .unwrap()
                    .change(action, seconds, false)
                    .unwrap_or_else(|e| panic!("{command}: {action:?} -> {seconds} s: {e}"));
            }
        }
    }
}

#[test]
fn wake_chain_accepts_only_dpms_on_numeric_sleeps_and_reload() {
    for command in [
        "smplos-hypr-dpms on",
        "hyprctl dispatch dpms on",
        LUA_DPMS_ON,
        HELPER_WAKE_CHAIN,
        "hyprctl dispatch dpms on; sleep 0.2; hyprctl reload; sleep 0.2; hyprctl dispatch dpms on",
        "sleep 1; smplos-hypr-dpms on",
        "smplos-hypr-dpms on;sleep .5;hyprctl reload; sleep 1.",
    ] {
        assert!(wake_chain(command), "{command}");
    }
    let huge = format!("smplos-hypr-dpms on; sleep {}", "9".repeat(50));
    for command in [
        "",
        " ",
        ";",
        "hyprctl reload",
        "sleep 0.2",
        "smplos-hypr-dpms off",
        "hyprctl dispatch dpms off",
        "notify-send hi; smplos-hypr-dpms on",
        "smplos-hypr-dpms on; notify-send hi",
        "smplos-hypr-dpms on;",
        ";smplos-hypr-dpms on",
        "smplos-hypr-dpms on;; hyprctl reload",
        "smplos-hypr-dpms on && reboot",
        "smplos-hypr-dpms on || reboot",
        "smplos-hypr-dpms on | cat",
        "smplos-hypr-dpms on &",
        "smplos-hypr-dpms on > /tmp/log",
        "smplos-hypr-dpms on; sleep $(reboot)",
        "smplos-hypr-dpms on; sleep `reboot`",
        "smplos-hypr-dpms on; sleep abc",
        "smplos-hypr-dpms on; sleep",
        "smplos-hypr-dpms on; sleep -1",
        "smplos-hypr-dpms on; sleep +1",
        "smplos-hypr-dpms on; sleep 1e3",
        "smplos-hypr-dpms on; sleep 1.2.3",
        "smplos-hypr-dpms on; sleep .",
        "smplos-hypr-dpms on; sleep 1s",
        "smplos-hypr-dpms on; sleep 1 2",
        "smplos-hypr-dpms on; sleep  1",
        huge.as_str(),
    ] {
        assert!(!wake_chain(command), "{command}");
    }
}

#[test]
fn every_hypridle_conf_smplos_shipped_earlier_supports_every_timer_change() {
    for (name, config) in fixtures("shipped") {
        check_hypridle_conf(&config, &format!("shipped/{name}"));
    }
}

#[test]
fn every_hypridle_conf_earlier_settings_versions_generated_supports_every_timer_change() {
    for (name, config) in fixtures("generated") {
        let label = format!("generated/{name}");
        let lock = Config::parse(&config).unwrap().seconds[0];
        assert_eq!(
            lock,
            if name.contains("lock-never") { 0 } else { 300 },
            "{label}"
        );
        check_hypridle_conf(&config, &label);
    }
}

#[test]
#[ignore = "fetches smplOS's current file: run scripts/check-smplos-stock-hypridle.sh"]
fn upstream_stock_hypridle_conf_supports_every_timer_change() {
    let path = std::env::var_os("SMPLOS_STOCK_HYPRIDLE_CONF")
        .expect("SMPLOS_STOCK_HYPRIDLE_CONF names a copy of the hypridle.conf smplOS ships");
    let config = fs::read_to_string(&path).unwrap();
    check_hypridle_conf(&config, "smplOS stock hypridle.conf (upstream)");
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

    // The file smplOS ships, and what the editor leaves after Lock -> Never.
    System.validate(SMPLOS_STOCK, false).unwrap();
    let never = Config::parse(SMPLOS_STOCK)
        .unwrap()
        .change(Action::Lock, 0, false)
        .unwrap();
    System.validate(&never, false).unwrap();
    assert!(System
        .validate(&never.replace("timeout = 330", "timeout = broken"), false)
        .is_err());
    let mut text = never;
    for action in [Action::Dpms, Action::Suspend] {
        text = Config::parse(&text)
            .unwrap()
            .change(action, 0, false)
            .unwrap();
    }
    System.validate(&text, true).unwrap();
}
