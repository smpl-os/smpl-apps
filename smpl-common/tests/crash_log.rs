use std::fs;

// Its own test binary, so replacing the process-wide panic hook cannot affect other tests.
#[test]
fn a_panic_is_appended_to_the_apps_crash_log() {
    let state = std::env::temp_dir().join(format!("smpl-common-crash-hook-{}", std::process::id()));
    let _ = fs::remove_dir_all(&state);
    std::env::set_var("XDG_STATE_HOME", &state);

    smpl_common::crash_log::install("probe-app");
    smpl_common::crash_log::install("probe-app");

    let crashed = std::thread::Builder::new()
        .name("worker".into())
        .spawn(|| panic!("probe went wrong: {}", 42))
        .expect("the thread starts")
        .join();
    assert!(crashed.is_err());

    let log = fs::read_to_string(smpl_common::crash_log::log_file(&state, "probe-app"))
        .expect("the panic was written to the crash log");
    assert_eq!(log.lines().count(), 1, "one panic, one line, however often install ran: {log}");
    assert!(log.contains(" probe-app "), "{log}");
    assert!(log.contains(env!("CARGO_PKG_VERSION")), "{log}");
    assert!(log.contains("thread 'worker' panicked at"), "{log}");
    assert!(log.contains("crash_log.rs:"), "{log}");
    assert!(log.contains("probe went wrong: 42"), "{log}");

    let _ = fs::remove_dir_all(&state);
}
