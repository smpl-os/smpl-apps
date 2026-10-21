//! A panic log that survives launchers which discard stderr.
//!
//! The apps are started from compositor key bindings, where stderr is discarded, and release
//! builds use `panic = "abort"`. A panic therefore used to leave nothing but a core dump.
//! [`install`] appends one line per panic to `$XDG_STATE_HOME/smplos/<app-id>/crash.log`
//! (`~/.local/state/...` by default) before the process aborts. Past 64 KiB the file is moved
//! to `crash.log.1`, replacing the older one, so there are never more than two.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::panic::{self, PanicHookInfo};
use std::path::{Path, PathBuf};
use std::sync::Once;

use chrono::{SecondsFormat, Utc};

/// Past this size the log is moved aside.
const MAX_BYTES: u64 = 64 * 1024;
/// Longest panic message kept in a record.
const MAX_MESSAGE_CHARS: usize = 2000;

/// Append every panic of this process to the app's crash log, then run the previous hook.
///
/// Safe to call more than once; only the first call installs anything.
pub fn install(app_id: &'static str) {
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if let Some(state_dir) = dirs::state_dir() {
                let _ = append(&log_file(&state_dir, app_id), &record(app_id, info));
            }
            previous(info);
        }));
    });
}

/// `<state dir>/smplos/<app id>/crash.log`.
pub fn log_file(state_dir: &Path, app_id: &str) -> PathBuf {
    state_dir.join("smplos").join(app_id).join("crash.log")
}

fn record(app_id: &str, info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(the payload is not text)".to_owned());
    let place = info.location().map_or_else(
        || "an unknown place".to_owned(),
        |at| format!("{}:{}:{}", at.file(), at.line(), at.column()),
    );
    let thread = std::thread::current();
    format_record(
        &Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        app_id,
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("<unnamed>"),
        &place,
        &message,
    )
}

/// One line, so the log can be read with `tail` and searched with `grep`.
fn format_record(
    when: &str,
    app_id: &str,
    version: &str,
    thread: &str,
    place: &str,
    message: &str,
) -> String {
    let message: String = message.chars().take(MAX_MESSAGE_CHARS).collect();
    let message = message.lines().collect::<Vec<_>>().join(" | ");
    format!("{when} {app_id} {version} thread '{thread}' panicked at {place}: {message}\n")
}

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    if fs::metadata(path).is_ok_and(|meta| meta.len() >= MAX_BYTES) {
        let mut older: OsString = path.as_os_str().to_owned();
        older.push(".1");
        let _ = fs::rename(path, older);
    }
    let mut file = OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("smpl-common-crash-log-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_log_lives_in_the_apps_state_directory() {
        assert_eq!(
            log_file(Path::new("/state"), "settings"),
            PathBuf::from("/state/smplos/settings/crash.log")
        );
    }

    #[test]
    fn a_record_is_one_line_with_what_is_needed_to_find_the_bug() {
        let line = format_record(
            "2026-10-20T20:05:06Z",
            "settings",
            "0.8.32",
            "main",
            "zbus/src/executor.rs:190:27",
            "there is no reactor running\nmust be called from a runtime",
        );
        assert_eq!(
            line,
            "2026-10-20T20:05:06Z settings 0.8.32 thread 'main' panicked at \
             zbus/src/executor.rs:190:27: there is no reactor running | must be called from a \
             runtime\n"
        );
    }

    #[test]
    fn an_enormous_message_is_cut() {
        let line = format_record("t", "app", "1", "main", "here", &"x".repeat(50_000));
        assert!(line.len() < MAX_MESSAGE_CHARS + 100, "{}", line.len());
        assert!(line.ends_with("x\n"));
    }

    #[test]
    fn append_creates_the_directories_and_keeps_adding() {
        let dir = scratch("append");
        let path = log_file(&dir, "settings");
        append(&path, "first\n").unwrap();
        append(&path, "second\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_is_readable_by_its_owner_only() {
        let dir = scratch("mode");
        let path = log_file(&dir, "settings");
        append(&path, "line\n").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_log_is_moved_aside_and_the_older_one_is_replaced() {
        let dir = scratch("rotate");
        let path = log_file(&dir, "settings");
        let older = path.with_file_name("crash.log.1");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        fs::write(&path, "a".repeat(MAX_BYTES as usize)).unwrap();
        append(&path, "new\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        assert_eq!(fs::metadata(&older).unwrap().len(), MAX_BYTES);

        fs::write(&path, "b".repeat(MAX_BYTES as usize)).unwrap();
        append(&path, "newer\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "newer\n");
        assert!(fs::read_to_string(&older).unwrap().starts_with('b'));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_log_just_under_the_cap_is_kept_in_place() {
        let dir = scratch("under");
        let path = log_file(&dir, "settings");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "a".repeat(MAX_BYTES as usize - 1)).unwrap();
        append(&path, "x").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_BYTES);
        assert!(!path.with_file_name("crash.log.1").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
