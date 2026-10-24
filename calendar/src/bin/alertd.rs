//! Session-long, notification-only reminders. No sound/action/snooze is invented.
//! A per-user advisory lock guards launches; delivery failures remain retryable.
//! Only --foreground enters runtime; ordinary launches merely check the service.

#[path = "../alarm.rs"]
mod alarm;

use anyhow::{bail, ensure, Context, Result};
use chrono::{Local, TimeZone};
use rusqlite::Connection;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, PartialEq)]
enum Invocation {
    Service,
    Autostart,
    Foreground { persistent: bool },
    Version,
    Check,
    Help,
}

fn invocation(args: &[String]) -> Result<Invocation> {
    match args {
        [] => Ok(Invocation::Service),
        [arg] => match arg.as_str() {
            "--foreground" => Ok(Invocation::Foreground { persistent: false }),
            "--autostart" => Ok(Invocation::Autostart),
            "--version" | "-V" => Ok(Invocation::Version),
            "--check" => Ok(Invocation::Check),
            "--help" | "-h" => Ok(Invocation::Help),
            _ => bail!("Unknown reminder daemon option; use --help"),
        },
        [foreground, persistent] if foreground == "--foreground" && persistent == "--persistent" => {
            Ok(Invocation::Foreground { persistent: true })
        }
        _ => bail!("Invalid reminder daemon options; use --help"),
    }
}

fn db_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required for calendar reminders")?;
    let home = PathBuf::from(home);
    ensure!(
        home.is_absolute(),
        "Calendar reminder HOME must be absolute"
    );
    Ok(home.join(".local/share/smplos/calendar/events.db"))
}

#[derive(Debug, PartialEq)]
enum ServiceState {
    Running,
    Stopped,
    Missing,
}

fn service_state(properties: &str) -> ServiceState {
    let property = |name: &str| {
        properties
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once('=')?;
                (key == name).then_some(value)
            })
            .unwrap_or("")
    };
    match property("LoadState") {
        "masked" => ServiceState::Stopped,
        "loaded" => {
            if matches!(
                property("ActiveState"),
                "active" | "activating" | "reloading"
            ) {
                ServiceState::Running
            } else {
                ServiceState::Stopped
            }
        }
        _ => ServiceState::Missing,
    }
}

fn query_user_service() -> Result<ServiceState> {
    let output = Command::new("systemctl")
        .args([
            "--user",
            "--no-pager",
            "show",
            "--property=LoadState",
            "--property=ActiveState",
            "smpl-calendar-alertd.service",
        ])
        .stdin(Stdio::null())
        .output()
        .context("User reminder service could not be queried")?;
    ensure!(
        output.status.success(),
        "User reminder service is unavailable"
    );
    Ok(service_state(&String::from_utf8_lossy(&output.stdout)))
}

fn check_user_service() -> Result<()> {
    match query_user_service()? {
        ServiceState::Running => Ok(()),
        ServiceState::Stopped => {
            eprintln!(
                "smpl-calendar-alertd: reminder service is stopped or masked; leaving it unchanged"
            );
            Ok(())
        }
        ServiceState::Missing => {
            bail!("Install and explicitly enable/start the calendar reminder user service")
        }
    }
}

fn autostart_service() -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", "start", "smpl-calendar-alertd.service"])
        .stdin(Stdio::null())
        .status()
        .context("Could not start the calendar reminder user service")?;
    ensure!(
        status.success(),
        "Calendar reminders could not start; check systemctl --user status smpl-calendar-alertd.service"
    );
    ensure!(
        query_user_service()? == ServiceState::Running,
        "Calendar reminder user service did not stay running"
    );
    Ok(())
}

fn check_configuration() -> Result<()> {
    let _ = db_path()?;
    ensure!(
        alarm::lock_path()?.is_absolute(),
        "Reminder lock directory must be absolute"
    );
    let path = std::env::var_os("PATH").unwrap_or_default();
    ensure!(
        std::env::split_paths(&path).any(|directory| {
            std::fs::metadata(directory.join("notify-send")).is_ok_and(|metadata| {
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        }),
        "notify-send is not available in PATH"
    );
    println!("smpl-calendar-alertd: configuration check passed (database unopened)");
    Ok(())
}

fn notification_command(persistent: bool) -> Command {
    let mut command = Command::new("notify-send");
    command.args(["-a", "smpl-calendar", "-i", "x-office-calendar"]);
    if persistent {
        command.args([
            "-u", "critical", "-t", "0",
            "-h", "boolean:resident:true",
            "-h", "boolean:transient:false",
            "-h", "string:desktop-entry:smpl-calendar",
        ]);
    } else {
        command.args(["-u", "normal"]);
    }
    command
}

fn send_notification(reminder: &alarm::Reminder, persistent: bool) -> Result<()> {
    let now = Local::now().timestamp();
    if now < reminder.due_ts || now.saturating_sub(reminder.due_ts) > alarm::MAX_LATENESS_SECONDS {
        bail!("Reminder delivery window has expired");
    }
    let start = Local
        .timestamp_opt(reminder.occurrence_ts, 0)
        .single()
        .context("Invalid reminder timestamp")?;
    let body = format!("Starts at {}", start.format("%H:%M on %A, %B %e"));
    let mut child = notification_command(persistent)
        .args(["--", &reminder.title, &body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Notification delivery could not start")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!("Desktop notification delivery was rejected");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Desktop notification delivery timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn run(persistent: bool) -> Result<()> {
    let Some(_lock) = alarm::acquire_lock(&alarm::lock_path()?)? else {
        return Ok(());
    };
    let path = db_path()?;
    // Login can start the service before the calendar's first database creation.
    let conn = loop {
        if path.exists() {
            match open_database(&path) {
                Ok(conn) => break conn,
                Err(_) => eprintln!("smpl-calendar-alertd: database is not ready; retrying"),
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    };
    loop {
        match alarm::tick(&conn, &Local, Local::now().timestamp(), |reminder| {
            send_notification(reminder, persistent)
        }) {
            Ok(report) => {
                if report.failed > 0 {
                    eprintln!("smpl-calendar-alertd: delivery failed; retrying recent reminders on the next tick");
                }
                if report.invalid > 0 {
                    eprintln!("smpl-calendar-alertd: invalid reminder schedules were skipped");
                }
            }
            Err(error) => eprintln!("smpl-calendar-alertd: reminder check failed: {error}"),
        }
        wait_for_next_poll(Duration::from_secs(30))?;
    }
}

fn wait_for_next_poll(duration: Duration) -> Result<()> {
    let mut requested = libc::timespec {
        tv_sec: duration.as_secs().try_into()?,
        tv_nsec: duration.subsec_nanos() as libc::c_long,
    };
    loop {
        let mut remaining = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // BOOTTIME counts suspend, unlike ordinary monotonic sleep. An elapsed
        // poll becomes runnable on resume; this is not a system-waking alarm.
        let status =
            unsafe { libc::clock_nanosleep(libc::CLOCK_BOOTTIME, 0, &requested, &mut remaining) };
        match status {
            0 => return Ok(()),
            libc::EINTR => requested = remaining,
            error => return Err(std::io::Error::from_raw_os_error(error).into()),
        }
    }
}

fn open_database(path: &std::path::Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_secs(5))?;
    // A newly created SQLite file can become visible before the UI has finished
    // creating its events table or adding the reminder column.
    conn.prepare(
        "SELECT id, title, start_ts, alert_minutes, recurrence, recurrence_end FROM events LIMIT 0",
    )?;
    alarm::ensure_tracking(&conn)?;
    Ok(conn)
}

fn execute() -> Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|_| anyhow::anyhow!("Arguments must be UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    match invocation(&args)? {
        Invocation::Service => check_user_service(),
        Invocation::Autostart => autostart_service(),
        Invocation::Foreground { persistent } => run(persistent),
        Invocation::Version => {
            println!("smpl-calendar-alertd {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Invocation::Check => check_configuration(),
        Invocation::Help => {
            println!(
                "Usage: smpl-calendar-alertd [--foreground [--persistent] | --autostart | --version | --check | --help]\n\
                No arguments: check the existing user service; never start an unmanaged daemon.\n\
                --autostart: start the managed user service without changing enablement or masks.\n\
                --foreground: run the daemon (service/explicit administration only).\n\
                --persistent: request critical, non-expiring notifications until dismissed.\n\
                --check: read-only dependency/configuration check; never open the database.\n\
                --version: print version; never open the database."
            );
            Ok(())
        }
    }
}

fn main() {
    if let Err(error) = execute() {
        eprintln!("smpl-calendar-alertd: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_foreground_can_enter_database_runtime() {
        assert_eq!(invocation(&[]).unwrap(), Invocation::Service);
        for (option, expected) in [
            ("--foreground", Invocation::Foreground { persistent: false }),
            ("--autostart", Invocation::Autostart),
            ("--version", Invocation::Version),
            ("--check", Invocation::Check),
            ("--help", Invocation::Help),
        ] {
            assert_eq!(invocation(&[option.into()]).unwrap(), expected);
        }
        assert!(invocation(&["--unknown".into()]).is_err());
        assert!(invocation(&["--foreground".into(), "--check".into()]).is_err());
        assert_eq!(
            invocation(&["--foreground".into(), "--persistent".into()]).unwrap(),
            Invocation::Foreground { persistent: true }
        );
        assert!(invocation(&["--persistent".into()]).is_err());
        assert!(invocation(&["--version".into(), "--persistent".into()]).is_err());
    }

    #[test]
    fn persistent_notifications_request_no_expiry_and_kde_desktop_identity() {
        let command = notification_command(true);
        let args: Vec<_> = command.get_args().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(args, [
            "-a", "smpl-calendar", "-i", "x-office-calendar",
            "-u", "critical", "-t", "0",
            "-h", "boolean:resident:true",
            "-h", "boolean:transient:false",
            "-h", "string:desktop-entry:smpl-calendar",
        ]);
        let command = notification_command(false);
        let args: Vec<_> = command.get_args().map(|arg| arg.to_str().unwrap()).collect();
        assert_eq!(args, ["-a", "smpl-calendar", "-i", "x-office-calendar", "-u", "normal"]);
    }

    #[test]
    fn suspend_aware_poll_clock_supports_nonblocking_wait() {
        wait_for_next_poll(Duration::ZERO).unwrap();
    }

    #[test]
    fn inactive_failed_masked_and_custom_units_are_never_started_by_legacy_ui() {
        for active in ["inactive", "failed", "deactivating"] {
            assert_eq!(
                service_state(&format!("LoadState=loaded\nActiveState={active}\n")),
                ServiceState::Stopped
            );
        }
        assert_eq!(
            service_state("LoadState=masked\nActiveState=inactive"),
            ServiceState::Stopped
        );
        assert_eq!(
            service_state("LoadState=loaded\nActiveState=active"),
            ServiceState::Running
        );
        assert_eq!(
            service_state("LoadState=loaded\nActiveState=activating"),
            ServiceState::Running
        );
        assert_eq!(
            service_state("LoadState=not-found\nActiveState=inactive"),
            ServiceState::Missing
        );
        assert_eq!(service_state(""), ServiceState::Missing);
    }
}
