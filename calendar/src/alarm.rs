use anyhow::{Context, Result};
use chrono::TimeZone;
use rusqlite::{params, Connection};
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[path = "recurrence.rs"]
mod recurrence;

/// Startup, resume and delivery retries share the inclusive five-minute window.
pub const MAX_LATENESS_SECONDS: i64 = 5 * 60;

#[derive(Clone, Debug)]
struct AlertEvent {
    id: i64,
    title: String,
    start_ts: i64,
    alert_minutes: i32,
    recurrence: String,
    recurrence_end: Option<i64>,
}

pub struct Reminder {
    pub title: String,
    pub occurrence_ts: i64,
    pub due_ts: i64,
}

#[derive(Default, Debug, PartialEq)]
pub struct TickReport {
    pub delivered: usize,
    pub failed: usize,
    pub invalid: usize,
}

pub fn lock_path() -> Result<PathBuf> {
    let directory = dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .context("No reminder runtime directory is available")?;
    anyhow::ensure!(
        directory.is_absolute(),
        "Reminder lock directory must be absolute"
    );
    Ok(directory.join("smplos/calendar-alertd.lock"))
}

pub fn acquire_lock(path: &Path) -> Result<Option<File>> {
    std::fs::create_dir_all(path.parent().context("Invalid reminder lock path")?)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

pub fn ensure_tracking(conn: &Connection) -> Result<()> {
    let transaction = conn.unchecked_transaction()?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS sent_alerts (
            event_id INTEGER NOT NULL,
            alert_ts INTEGER NOT NULL,
            schedule_revision TEXT,
            PRIMARY KEY (event_id, alert_ts)
        );",
    )?;
    let columns = {
        let mut statement = transaction.prepare("PRAGMA table_info(sent_alerts)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns
    };
    if !columns.iter().any(|column| column == "schedule_revision") {
        transaction.execute_batch("ALTER TABLE sent_alerts ADD COLUMN schedule_revision TEXT;")?;
    }
    transaction.execute_batch(
        "CREATE TRIGGER IF NOT EXISTS calendar_alerts_event_deleted
        AFTER DELETE ON events BEGIN
            DELETE FROM sent_alerts WHERE event_id = OLD.id;
        END;
        CREATE TRIGGER IF NOT EXISTS calendar_alerts_schedule_changed
        AFTER UPDATE ON events
        WHEN OLD.start_ts IS NOT NEW.start_ts
          OR OLD.alert_minutes IS NOT NEW.alert_minutes
          OR OLD.recurrence IS NOT NEW.recurrence
          OR OLD.recurrence_end IS NOT NEW.recurrence_end
        BEGIN
            DELETE FROM sent_alerts WHERE event_id = OLD.id;
        END;",
    )?;
    transaction.commit()?;
    Ok(())
}

fn occurrences<Tz: TimeZone>(
    timezone: &Tz,
    event: &AlertEvent,
    due_from: i64,
    now: i64,
) -> Result<Vec<i64>> {
    let offset = i64::from(event.alert_minutes) * 60;
    let from = due_from
        .checked_add(offset)
        .context("Reminder time overflow")?;
    let to = now
        .checked_add(offset)
        .and_then(|value| value.checked_add(1))
        .context("Reminder time overflow")?;
    let seed = timezone
        .timestamp_opt(event.start_ts, 0)
        .single()
        .context("Invalid reminder start")?;
    let until = event
        .recurrence_end
        .map(|timestamp| {
            timezone
                .timestamp_opt(timestamp, 0)
                .single()
                .map(|value| value.date_naive())
                .context("Invalid recurrence end")
        })
        .transpose()?;
    Ok(
        recurrence::starts(timezone, seed, &event.recurrence, until, from, to)?
            .iter()
            .map(|occurrence| occurrence.timestamp())
            .collect(),
    )
}

/// Injected clock/timezone/delivery make the scheduler testable without a desktop.
/// Failed delivery is never marked sent and is retried on subsequent recent ticks.
pub fn tick<Tz: TimeZone>(
    conn: &Connection,
    timezone: &Tz,
    now: i64,
    mut deliver: impl FnMut(&Reminder) -> Result<()>,
) -> Result<TickReport> {
    let due_from = now.saturating_sub(MAX_LATENESS_SECONDS);
    let mut report = TickReport::default();
    let mut statement = conn.prepare_cached(
        "SELECT id, title, start_ts, alert_minutes, recurrence, recurrence_end
         FROM events WHERE alert_minutes > 0",
    )?;
    let events = statement
        .query_map([], |row| {
            Ok(AlertEvent {
                id: row.get(0)?,
                title: row.get(1)?,
                start_ts: row.get(2)?,
                alert_minutes: row.get(3)?,
                recurrence: row.get(4)?,
                recurrence_end: row.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for event in events {
        let revision = serde_json::to_string(&(
            1,
            event.start_ts,
            event.alert_minutes,
            &event.recurrence,
            event.recurrence_end,
        ))?;
        let starts = match occurrences(timezone, &event, due_from, now) {
            Ok(starts) => starts,
            Err(_) => {
                report.invalid += 1;
                continue;
            }
        };
        for occurrence_ts in starts {
            let already_sent: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sent_alerts WHERE event_id=?1 AND alert_ts=?2
                 AND (schedule_revision IS NULL OR schedule_revision=?3))",
                params![event.id, occurrence_ts, revision],
                |row| row.get(0),
            )?;
            if already_sent {
                continue;
            }
            // Revalidate immediately before delivery: another UI may have edited
            // or deleted an event since the candidate list was read.
            let unchanged: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE id=?1 AND start_ts=?2
                 AND alert_minutes=?3 AND recurrence=?4 AND recurrence_end IS ?5 AND title=?6)",
                params![
                    event.id,
                    event.start_ts,
                    event.alert_minutes,
                    event.recurrence,
                    event.recurrence_end,
                    event.title
                ],
                |row| row.get(0),
            )?;
            if !unchanged {
                continue;
            }
            if deliver(&Reminder {
                title: event.title.clone(),
                occurrence_ts,
                due_ts: occurrence_ts - i64::from(event.alert_minutes) * 60,
            })
            .is_err()
            {
                report.failed += 1;
                continue;
            }
            // Do not stamp a newly edited schedule with an old in-flight delivery.
            conn.execute(
                "INSERT INTO sent_alerts (event_id, alert_ts, schedule_revision)
                 SELECT id, ?2, ?7 FROM events WHERE id=?1 AND start_ts=?3
                 AND alert_minutes=?4 AND recurrence=?5 AND recurrence_end IS ?6
                 ON CONFLICT(event_id, alert_ts) DO UPDATE
                 SET schedule_revision=excluded.schedule_revision",
                params![
                    event.id,
                    occurrence_ts,
                    event.start_ts,
                    event.alert_minutes,
                    event.recurrence,
                    event.recurrence_end,
                    revision
                ],
            )?;
            report.delivered += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::sync::atomic::{AtomicU64, Ordering};

    const SCHEMA: &str = "CREATE TABLE events (
        id INTEGER PRIMARY KEY, title TEXT NOT NULL, start_ts INTEGER NOT NULL,
        alert_minutes INTEGER NOT NULL, recurrence TEXT NOT NULL, recurrence_end INTEGER);";

    fn at(value: &str) -> i64 {
        value.parse::<DateTime<Utc>>().unwrap().timestamp()
    }
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        ensure_tracking(&conn).unwrap();
        conn
    }
    fn add(conn: &Connection, id: i64, start: i64, minutes: i32, recurrence: &str) {
        conn.execute(
            "INSERT INTO events VALUES (?1, 'Fixture', ?2, ?3, ?4, NULL)",
            params![id, start, minutes, recurrence],
        )
        .unwrap();
    }
    fn check(conn: &Connection, now: i64) -> TickReport {
        tick(conn, &Utc, now, |_| Ok(())).unwrap()
    }
    fn sent(conn: &Connection) -> i64 {
        conn.query_row("SELECT count(*) FROM sent_alerts", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn one_shot_never_fires_early_stale_or_twice_and_zero_disables() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        add(&conn, 2, due, 0, "none");
        assert_eq!(check(&conn, due - 1).delivered, 0);
        assert_eq!(check(&conn, due).delivered, 1);
        assert_eq!(check(&conn, due + 30).delivered, 0);
        assert_eq!(sent(&conn), 1);
        let stale = db();
        add(&stale, 1, due + 300, 5, "none");
        assert_eq!(check(&stale, due + 301).delivered, 0);
    }

    #[test]
    fn first_start_catches_up_exactly_five_minutes_inclusive_and_never_early() {
        let due = at("2026-10-02T10:00:00Z");
        for age in [-1, 0, 1, 299, 300, 301, 7200] {
            let conn = db();
            add(&conn, 1, due + 300, 5, "none");
            let expected = usize::from((0..=300).contains(&age));
            assert_eq!(check(&conn, due + age).delivered, expected, "age={age}");
            assert_eq!(check(&conn, due + age).delivered, 0);
        }
    }

    #[test]
    fn sleep_resume_uses_current_five_minute_window_not_time_since_last_poll() {
        let conn = db();
        let wake = at("2026-10-02T10:00:00Z");
        for (id, due) in [
            (1, wake - 300),
            (2, wake - 1),
            (3, wake - 301),
            (4, wake - 1800),
            (5, wake + 1),
        ] {
            add(&conn, id, due + 300, 5, "none");
        }
        assert_eq!(check(&conn, wake - 3600).delivered, 0);
        assert_eq!(check(&conn, wake).delivered, 2);
        assert_eq!(check(&conn, wake).delivered, 0);
        assert_eq!(check(&conn, wake + 1).delivered, 1);
    }

    #[test]
    fn delivery_failures_retry_at_five_minute_boundary_but_never_after_it() {
        let due = at("2026-10-02T10:00:00Z");
        let conn = db();
        add(&conn, 1, due + 300, 5, "none");
        for now in [due, due + 299, due + 300] {
            assert_eq!(
                tick(&conn, &Utc, now, |_| anyhow::bail!("Injected timeout"))
                    .unwrap()
                    .failed,
                1
            );
            assert_eq!(sent(&conn), 0);
        }
        assert_eq!(
            tick(&conn, &Utc, due + 301, |_| panic!(
                "Expired reminder retried"
            ))
            .unwrap(),
            TickReport::default()
        );
        let success = db();
        add(&success, 1, due + 300, 5, "none");
        assert_eq!(
            tick(&success, &Utc, due + 299, |_| anyhow::bail!(
                "Injected failure"
            ))
            .unwrap()
            .failed,
            1
        );
        assert_eq!(check(&success, due + 300).delivered, 1);
        assert_eq!(check(&success, due + 301).delivered, 0);
    }

    #[test]
    fn recurring_offsets_cross_previous_day_and_dedup_by_occurrence() {
        let conn = db();
        let first = at("2026-10-02T23:40:00Z");
        add(&conn, 1, at("2000-01-01T00:10:00Z"), 30, "daily");
        assert_eq!(check(&conn, first).delivered, 1);
        assert_eq!(check(&conn, first + 30).delivered, 0);
        assert_eq!(check(&conn, first + 86400).delivered, 1);
        assert_eq!(sent(&conn), 2);
    }

    #[test]
    fn a_daily_reminder_one_day_early_targets_the_next_not_previous_occurrence() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, at("2000-01-01T10:00:00Z"), 1440, "daily");
        let mut delivered_start = None;
        let result = tick(&conn, &Utc, due, |reminder| {
            delivered_start = Some(reminder.occurrence_ts);
            Ok(())
        })
        .unwrap();
        assert_eq!(result.delivered, 1);
        assert_eq!(delivered_start, Some(due + 86400));
    }

    #[test]
    fn recurrence_end_is_inclusive_and_previous_day_reminder_is_not_expired() {
        let conn = db();
        let due = at("2026-10-02T23:40:00Z");
        add(&conn, 1, at("2026-01-01T00:10:00Z"), 30, "daily");
        conn.execute(
            "UPDATE events SET recurrence_end=?1",
            [at("2026-10-03T00:00:00Z")],
        )
        .unwrap();
        assert_eq!(check(&conn, due).delivered, 1);
        assert_eq!(check(&conn, due + 86400).delivered, 0);
    }

    #[test]
    fn failed_delivery_retries_without_marking_and_query_errors_propagate() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        assert_eq!(
            tick(&conn, &Utc, due, |_| anyhow::bail!("Injected failure"))
                .unwrap()
                .failed,
            1
        );
        assert_eq!(sent(&conn), 0);
        assert_eq!(check(&conn, due + 30).delivered, 1);
        assert_eq!(check(&conn, due + 60).delivered, 0);
        let missing = Connection::open_in_memory().unwrap();
        assert!(tick(&missing, &Utc, due, |_| Ok(())).is_err());
    }

    #[test]
    fn invalid_schedule_does_not_starve_valid_reminders() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "unsupported");
        add(&conn, 2, i64::MAX, 5, "daily");
        add(&conn, 3, due + 300, 5, "none");
        let report = check(&conn, due);
        assert_eq!(report.invalid, 2);
        assert_eq!(report.delivered, 1);
    }

    #[test]
    fn in_flight_old_delivery_cannot_stamp_an_edited_or_deleted_schedule() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        tick(&conn, &Utc, due, |_| {
            conn.execute("UPDATE events SET start_ts=?1 WHERE id=1", [due + 330])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(sent(&conn), 0);
        assert_eq!(check(&conn, due + 30).delivered, 1);
        add(&conn, 2, due + 330, 5, "none");
        tick(&conn, &Utc, due + 30, |_| {
            conn.execute("DELETE FROM events WHERE id=2", [])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(sent(&conn), 1);
    }

    #[test]
    fn schedule_edits_and_deletion_invalidate_but_unchanged_save_does_not() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        assert_eq!(check(&conn, due).delivered, 1);
        conn.execute("UPDATE events SET start_ts=start_ts", [])
            .unwrap();
        assert_eq!(sent(&conn), 1);
        conn.execute("UPDATE events SET alert_minutes=10", [])
            .unwrap();
        assert_eq!(sent(&conn), 0);
        assert_eq!(check(&conn, due + 30).delivered, 0);
        conn.execute("UPDATE events SET start_ts=?1", [due + 660])
            .unwrap();
        assert_eq!(check(&conn, due + 60).delivered, 1);
        conn.execute("DELETE FROM events WHERE id=1", []).unwrap();
        assert_eq!(sent(&conn), 0);
        assert_eq!(check(&conn, due + 60).delivered, 0);
    }

    #[test]
    fn persisted_schedule_revision_is_part_of_occurrence_dedup() {
        let conn = db();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        conn.execute(
            "INSERT INTO sent_alerts VALUES (1, ?1, 'previous-schedule')",
            [due + 300],
        )
        .unwrap();
        assert_eq!(check(&conn, due + 299).delivered, 1);
        let revision: String = conn
            .query_row(
                "SELECT schedule_revision FROM sent_alerts WHERE event_id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            revision,
            serde_json::to_string(&(1, due + 300, 5, "none", None::<i64>)).unwrap()
        );
        assert_eq!(check(&conn, due + 300).delivered, 0);
        assert_eq!(sent(&conn), 1);
    }

    #[test]
    fn legacy_schema_upgrade_preserves_rows_and_suppresses_recent_success_replay() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "CREATE TABLE sent_alerts (
            event_id INTEGER NOT NULL, alert_ts INTEGER NOT NULL,
            PRIMARY KEY(event_id, alert_ts));",
        )
        .unwrap();
        let due = at("2026-10-02T10:00:00Z");
        add(&conn, 1, due + 300, 5, "none");
        conn.execute(
            "INSERT INTO sent_alerts VALUES (1, ?1), (99, 1)",
            [due + 300],
        )
        .unwrap();
        ensure_tracking(&conn).unwrap();
        ensure_tracking(&conn).unwrap();
        assert_eq!(sent(&conn), 2);
        let revision: Option<String> = conn
            .query_row(
                "SELECT schedule_revision FROM sent_alerts WHERE event_id=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(revision.is_none());
        assert_eq!(check(&conn, due + 300).delivered, 0);
        assert_eq!(sent(&conn), 2);
    }

    #[test]
    fn dst_uses_wall_time_and_elapsed_offset_without_duplicate_fold() {
        use chrono_tz::America::New_York;
        let conn = db();
        add(&conn, 1, at("2026-10-31T05:30:00Z"), 30, "daily");
        let due = at("2026-11-01T05:00:00Z");
        assert_eq!(
            tick(&conn, &New_York, due, |_| Ok(())).unwrap().delivered,
            1
        );
        assert_eq!(
            tick(&conn, &New_York, due + 3600, |_| Ok(()))
                .unwrap()
                .delivered,
            0
        );
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let parent = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/alarm-tests");
            std::fs::create_dir_all(&parent).unwrap();
            let path = parent.join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            for name in ["fixture.db", "fixture.lock"] {
                let _ = std::fs::remove_file(self.0.join(name));
            }
            let _ = std::fs::remove_dir(&self.0);
        }
    }

    #[test]
    fn restart_preserves_occurrence_dedup_and_delivers_recent_unsent_reminders() {
        let fixture = Fixture::new();
        let path = fixture.0.join("fixture.db");
        let due = at("2026-10-02T10:00:00Z");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            ensure_tracking(&conn).unwrap();
            add(&conn, 1, due + 300, 5, "daily");
            add(&conn, 2, due + 330, 5, "none");
            assert_eq!(check(&conn, due).delivered, 1);
        }
        let conn = Connection::open(&path).unwrap();
        ensure_tracking(&conn).unwrap();
        assert_eq!(check(&conn, due).delivered, 0);
        assert_eq!(check(&conn, due + 31).delivered, 1);
        assert_eq!(check(&conn, due + 60).delivered, 0);
        assert_eq!(check(&conn, due + 86400).delivered, 1);
    }

    #[test]
    fn initialization_and_ticks_preserve_old_dedup_rows_without_bulk_purge() {
        let conn = db();
        conn.execute(
            "INSERT INTO sent_alerts (event_id, alert_ts) VALUES (99, 1)",
            [],
        )
        .unwrap();
        ensure_tracking(&conn).unwrap();
        let now = at("2026-10-02T10:00:00Z");
        assert_eq!(check(&conn, now).delivered, 0);
        assert_eq!(sent(&conn), 1);
    }

    #[test]
    fn advisory_lock_is_exclusive_and_released_on_drop() {
        let fixture = Fixture::new();
        let path = fixture.0.join("fixture.lock");
        let first = acquire_lock(&path).unwrap().unwrap();
        assert!(acquire_lock(&path).unwrap().is_none());
        drop(first);
        assert!(acquire_lock(&path).unwrap().is_some());
    }

    #[test]
    fn notification_children_cannot_inherit_or_release_the_daemon_lock() {
        use std::os::fd::AsRawFd;
        let fixture = Fixture::new();
        let path = fixture.0.join("fixture.lock");
        let lock = acquire_lock(&path).unwrap().unwrap();
        // Read-only fcntl on our live fixture descriptor, never a real daemon.
        let flags = unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0);
        assert_ne!(flags & libc::FD_CLOEXEC, 0);
        assert!(std::process::Command::new("/usr/bin/true")
            .status()
            .unwrap()
            .success());
        assert!(acquire_lock(&path).unwrap().is_none());
        drop(lock);
        assert!(acquire_lock(&path).unwrap().is_some());
    }
}
