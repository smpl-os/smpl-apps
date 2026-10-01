use super::*;
use std::cell::RefCell;
use std::os::unix::fs::{symlink, PermissionsExt};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "settings-taskbar-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> PathBuf {
        self.0.join(".config/smplos/bar.conf")
    }

    fn write(&self, text: &str) -> PathBuf {
        let path = self.path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[derive(Default)]
struct Mock {
    calls: RefCell<Vec<(String, Vec<String>)>>,
    fail_at: Option<usize>,
    external: Option<(PathBuf, String)>,
}
impl Runtime for Mock {
    fn run(&self, program: &str, args: &[&str]) -> Result<String> {
        let mut calls = self.calls.borrow_mut();
        calls.push((program.into(), args.iter().map(|s| (*s).into()).collect()));
        if let Some((path, text)) = &self.external {
            fs::write(path, text)?;
        }
        if self.fail_at == Some(calls.len()) {
            return Err(Error::Command("mock command unavailable or failed".into()));
        }
        Ok(if program == "sh" { "12:34" } else { "" }.into())
    }
}

const ORIGINAL: &str = "# Personal bar preferences\nws_count=7\nws_position=left\nws_spacing=1\nws_style=numbers\nclock_format=date\n# Keep clock preferences\nclock_date_fmt=Mon D\ncustom=value # untouched\n";

#[test]
fn all_workspace_controls_save_exact_values_and_only_use_bar_apply() {
    for (setting, value, encoded) in [
        (Setting::Count, 1, "1"),
        (Setting::Count, 10, "10"),
        (Setting::Position, 0, "center"),
        (Setting::Position, 1, "left"),
        (Setting::Spacing, 1, "1"),
        (Setting::Spacing, 10, "10"),
        (Setting::Style, 0, "numbers"),
        (Setting::Style, 1, "squares"),
    ] {
        let fixture = Fixture::new();
        let path = fixture.write(ORIGINAL);
        let runtime = Mock::default();
        assert!(matches!(
            change(&path, &runtime, setting, value).unwrap(),
            SaveOutcome::Updated
        ));
        assert_eq!(read(&path).unwrap().values[setting as usize], value);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains(&format!("{}={encoded}\n", setting.key())));
        for line in ORIGINAL
            .lines()
            .filter(|line| !line.starts_with(&format!("{}=", setting.key())))
        {
            assert!(
                saved.lines().any(|saved_line| saved_line == line),
                "lost: {line}"
            );
        }
        assert_eq!(
            *runtime.calls.borrow(),
            vec![("bar-ctl".into(), vec!["apply".into()])]
        );
    }
}

#[test]
fn all_clock_setters_share_scoped_writer_and_checked_coherent_updates() {
    for setting in [Setting::ClockFormat, Setting::Clock24h, Setting::ClockDate] {
        for value in 0..setting.choices().len() as i32 {
            let fixture = Fixture::new();
            let path = fixture.write(ORIGINAL);
            let runtime = Mock::default();
            change(&path, &runtime, setting, value).unwrap();
            let snapshot = read(&path).unwrap();
            assert_eq!(snapshot.values[setting as usize], value);
            assert_eq!(&snapshot.values[..4], &[7, 1, 1, 0]);
            assert!(fs::read_to_string(&path)
                .unwrap()
                .contains("custom=value # untouched\n"));
            let calls = runtime.calls.borrow();
            assert_eq!(
                calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
                ["sh", "sh", "eww"]
            );
            assert!(calls[0].1[0].ends_with("/.config/eww/scripts/clock-top.sh"));
            assert!(calls[1].1[0].ends_with("/.config/eww/scripts/clock-bot.sh"));
            assert_eq!(
                &calls[2].1[2..],
                ["update", "clock-top=12:34", "clock-bot=12:34"]
            );
        }
    }
}

#[test]
fn reading_missing_or_partial_preferences_never_writes_clock_defaults() {
    let fixture = Fixture::new();
    assert_eq!(&read(&fixture.path()).unwrap().values[..4], &[4, 0, 1, 0]);
    assert!(!fixture.0.join(".config").exists());
    let path = fixture.write("# untouched\nws_count=7\n");
    read(&path).unwrap();
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "# untouched\nws_count=7\n"
    );
}

#[test]
fn external_change_before_save_is_preserved_and_readback_is_current() {
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    let old = read(&path).unwrap();
    assert_eq!(old.values[Setting::Spacing as usize], 1);
    fs::write(&path, ORIGINAL.replace("ws_spacing=1", "ws_spacing=8")).unwrap();
    change(&path, &Mock::default(), Setting::Style, 1).unwrap();
    assert_eq!(&read(&path).unwrap().values[..4], &[7, 1, 8, 1]);
}

#[test]
fn external_change_during_apply_returns_truthful_conflict_and_latest_readback() {
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    let runtime = Mock {
        external: Some((path.clone(), ORIGINAL.replace("ws_count=7", "ws_count=9"))),
        ..Mock::default()
    };
    assert!(matches!(
        change(&path, &runtime, Setting::Style, 1).unwrap(),
        SaveOutcome::SavedButNotApplied(Error::Conflict)
    ));
    assert_eq!(&read(&path).unwrap().values[..4], &[9, 1, 1, 0]);
}

#[test]
fn stale_revision_cannot_replace_an_external_edit_and_temporary_file_is_cleaned() {
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    let original = read_text(&path).unwrap();
    fs::write(&path, "# edited externally\nws_count=3\n").unwrap();
    assert!(matches!(
        replace(&path, &original, "ws_count=8\n"),
        Err(Error::Conflict)
    ));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# edited externally\nws_count=3\n"
    );
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn failed_apply_retains_saved_preferences_and_failed_save_never_runs_commands() {
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    let runtime = Mock {
        fail_at: Some(1),
        ..Mock::default()
    };
    assert!(matches!(
        change(&path, &runtime, Setting::Style, 1).unwrap(),
        SaveOutcome::SavedButNotApplied(_)
    ));
    assert_eq!(read(&path).unwrap().values[Setting::Style as usize], 1);

    let runtime = Mock::default();
    assert!(change(&path.join("not-a-directory"), &runtime, Setting::Count, 2).is_err());
    assert!(runtime.calls.borrow().is_empty());
}

#[test]
fn failure_at_any_clock_apply_stage_is_not_reported_as_success() {
    for fail_at in 1..=3 {
        let fixture = Fixture::new();
        let path = fixture.write(ORIGINAL);
        let runtime = Mock {
            fail_at: Some(fail_at),
            ..Mock::default()
        };
        assert!(matches!(
            change(&path, &runtime, Setting::Clock24h, 1).unwrap(),
            SaveOutcome::SavedButNotApplied(_)
        ));
        assert_eq!(runtime.calls.borrow().len(), fail_at);
    }
}

#[test]
fn invalid_values_duplicates_and_read_errors_preserve_original_bytes() {
    for text in [
        "ws_count=11\n",
        "ws_count=01\n",
        "ws_spacing=+1\n",
        "ws_count=1 0\n",
        "ws_spacing=0\n",
        "ws_position=right\n",
        "ws_style=triangles\n",
        "ws_count=3\nws_count=7\n",
        "clock_24h=yes\n",
    ] {
        let fixture = Fixture::new();
        let path = fixture.write(text);
        let runtime = Mock::default();
        assert!(read(&path).is_err());
        assert!(change(&path, &runtime, Setting::Style, 1).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), text);
        assert!(runtime.calls.borrow().is_empty());
    }
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    fs::write(&path, [0xff]).unwrap();
    assert!(read(&path).is_err());
    assert!(change(&path, &Mock::default(), Setting::Count, 4).is_err());
    assert_eq!(fs::read(path).unwrap(), [0xff]);
}

#[test]
fn invalid_requested_values_do_not_clamp_or_persist() {
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    for setting in SETTINGS {
        for value in [-1, 11] {
            let runtime = Mock::default();
            assert!(change(&path, &runtime, setting, value).is_err());
            assert!(runtime.calls.borrow().is_empty());
            assert_eq!(fs::read_to_string(&path).unwrap(), ORIGINAL);
        }
    }
}

#[test]
fn comments_crlf_permissions_and_unknown_keys_survive_while_changed_key_is_canonical() {
    let fixture = Fixture::new();
    let path = fixture.write("# my bar\r\nws_count = 7 \r\nunknown=verbatim\r\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    change(&path, &Mock::default(), Setting::Count, 3).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# my bar\r\nws_count=3\r\nunknown=verbatim\r\n"
    );
    assert_ne!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    change(&path, &Mock::default(), Setting::Style, 1).unwrap();
    assert!(fs::read_to_string(&path)
        .unwrap()
        .ends_with("ws_style=squares\r\n"));
}

#[test]
fn explicit_auto_clock_preferences_match_locale_defaults_without_read_time_writes() {
    let fixture = Fixture::new();
    let text = "# automatic clock\nclock_24h=auto\nclock_date_fmt=auto\n";
    let path = fixture.write(text);
    let snapshot = read(&path).unwrap();
    assert_eq!(snapshot.values[5..], Snapshot::default().values[5..]);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    change(&path, &Mock::default(), Setting::Style, 1).unwrap();
    assert!(fs::read_to_string(path).unwrap().starts_with(text));
}

#[test]
fn symlink_and_hardlink_configs_are_not_silently_replaced() {
    let fixture = Fixture::new();
    let target = fixture.write(ORIGINAL);
    let link = fixture.0.join("link.conf");
    symlink(&target, &link).unwrap();
    assert!(change(&link, &Mock::default(), Setting::Style, 1).is_err());
    fs::hard_link(&target, fixture.0.join("hard.conf")).unwrap();
    assert!(change(&target, &Mock::default(), Setting::Style, 1).is_err());
    assert_eq!(fs::read_to_string(target).unwrap(), ORIGINAL);
}

#[test]
fn coalesced_slider_and_clock_choices_persist_final_values_without_window_moves() {
    use super::scheduler::{Request, Scheduler};
    let fixture = Fixture::new();
    let path = fixture.write(ORIGINAL);
    let runtime = Mock::default();
    let mut scheduler = Scheduler::default();
    scheduler.enqueue(Request::Refresh);
    for value in 1..=10 {
        scheduler.enqueue(Request::Set(Setting::Count, value));
    }
    scheduler.enqueue(Request::Set(Setting::Style, 1));
    scheduler.enqueue(Request::Set(Setting::ClockDate, 2));
    while let Some(Request::Set(setting, value)) = scheduler.finish().1 {
        change(&path, &runtime, setting, value).unwrap();
    }
    assert_eq!(
        read(&path).unwrap().values,
        [10, 1, 1, 1, 2, Snapshot::default().values[5], 2]
    );
    let calls = runtime.calls.borrow();
    assert_eq!(calls.iter().filter(|c| c.0 == "bar-ctl").count(), 2);
    assert!(!calls.iter().any(|c| matches!(
        c.0.as_str(),
        "hyprctl" | "workspace-group" | "workspace-ctl"
    )));
}

#[test]
fn command_runner_reports_missing_and_failed_commands() {
    assert!(System
        .run("/nonexistent/settings-taskbar-test-command", &[])
        .is_err());
    assert!(System
        .run("sh", &["-c", "printf 'test failure' >&2; exit 4"])
        .is_err());
    assert_eq!(
        System.run("sh", &["-c", "printf 'test success'"]).unwrap(),
        "test success"
    );
}

#[test]
fn command_runner_bounds_a_stalled_process_without_touching_live_services() {
    let started = std::time::Instant::now();
    assert!(System.run("sleep", &["30"]).is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
}

#[test]
fn ui_requests_changes_without_optimistic_confirmed_values_and_refreshes_on_entry() {
    let ui = include_str!("../../ui/main.slint");
    for property in [
        "tb-ws-count",
        "tb-ws-spacing",
        "tb-ws-style-index",
        "tb-ws-position-index",
        "tb-clock-format-index",
        "tb-clock-24h",
        "tb-clock-date-fmt-index",
    ] {
        assert!(
            !ui.contains(&format!("root.{property} = ")),
            "{property} assigned optimistically"
        );
    }
    assert!(ui.contains("if root.active-tab == 6 { root.tb-refresh(); }"));
    assert!(ui.contains("if root.tb-status != \"\": Text"));
    assert!(ui.contains("Total workspaces across monitors"));
}
