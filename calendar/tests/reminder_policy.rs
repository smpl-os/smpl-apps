#[allow(dead_code)]
#[path = "../src/alarm.rs"]
mod alarm;

use chrono::Utc;
use rusqlite::{params, Connection};

const NOW: i64 = 1_800_000_000;
const SCHEMA: &str = "CREATE TABLE events (
    id INTEGER PRIMARY KEY, title TEXT NOT NULL, start_ts INTEGER NOT NULL,
    alert_minutes INTEGER NOT NULL, recurrence TEXT NOT NULL, recurrence_end INTEGER);";

fn database() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    alarm::ensure_tracking(&conn).unwrap();
    conn
}

fn add(conn: &Connection, id: i64, due: i64) {
    conn.execute("INSERT INTO events VALUES (?1, 'Synthetic fixture', ?2, 15, 'none', NULL)",
        params![id, due + 900]).unwrap();
}

#[test]
fn policy_inclusive_now_minus_300_through_now_never_early_or_stale() {
    assert_eq!(alarm::MAX_LATENESS_SECONDS, 300);
    for age in [-1, 0, 1, 299, 300, 301, 86400] {
        let conn = database();
        add(&conn, 1, NOW - age);
        let report = alarm::tick(&conn, &Utc, NOW, |reminder| {
            assert!((NOW - 300..=NOW).contains(&reminder.due_ts));
            Ok(())
        }).unwrap();
        assert_eq!(report.delivered, usize::from((0..=300).contains(&age)), "age={age}");
    }
}

#[test]
fn policy_startup_after_due_and_sleep_resume_only_catch_up_last_five_minutes() {
    let conn = database();
    for (id, due) in [(1, NOW - 301), (2, NOW - 300), (3, NOW - 60), (4, NOW), (5, NOW + 1)] {
        add(&conn, id, due);
    }
    assert_eq!(alarm::tick(&conn, &Utc, NOW - 86400, |_| Ok(())).unwrap().delivered, 0);
    assert_eq!(alarm::tick(&conn, &Utc, NOW, |_| Ok(())).unwrap().delivered, 3);
    assert_eq!(alarm::tick(&conn, &Utc, NOW, |_| Ok(())).unwrap().delivered, 0);
    assert_eq!(alarm::tick(&conn, &Utc, NOW + 1, |_| Ok(())).unwrap().delivered, 1);
}

#[test]
fn policy_failed_delivery_retries_through_300_seconds_not_301() {
    let conn = database();
    add(&conn, 1, NOW);
    for offset in [0, 60, 299] {
        assert_eq!(alarm::tick(&conn, &Utc, NOW + offset, |_| anyhow::bail!("Injected delivery failure"))
            .unwrap().failed, 1);
    }
    assert_eq!(alarm::tick(&conn, &Utc, NOW + 300, |_| Ok(())).unwrap().delivered, 1);
    assert_eq!(alarm::tick(&conn, &Utc, NOW + 300, |_| Ok(())).unwrap().delivered, 0);
    let expired = database();
    add(&expired, 1, NOW);
    assert_eq!(alarm::tick(&expired, &Utc, NOW + 300, |_| anyhow::bail!("Injected timeout"))
        .unwrap().failed, 1);
    assert_eq!(alarm::tick(&expired, &Utc, NOW + 301, |_| panic!("Expired delivery retried"))
        .unwrap().failed, 0);
}

#[test]
fn policy_restart_preserves_receipts_and_delivers_unsent_due_before_restart() {
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/reminder-policy");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("restart-{}.db", std::process::id()));
    assert!(!path.exists(), "fixture path must be fresh");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        alarm::ensure_tracking(&conn).unwrap();
        add(&conn, 1, NOW);
        add(&conn, 2, NOW + 1);
        assert_eq!(alarm::tick(&conn, &Utc, NOW, |_| Ok(())).unwrap().delivered, 1);
    }
    {
        let conn = Connection::open(&path).unwrap();
        alarm::ensure_tracking(&conn).unwrap();
        assert_eq!(alarm::tick(&conn, &Utc, NOW + 300, |_| Ok(())).unwrap().delivered, 1);
        assert_eq!(alarm::tick(&conn, &Utc, NOW + 300, |_| Ok(())).unwrap().delivered, 0);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM sent_alerts", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 2);
    }
    std::fs::remove_file(path).unwrap();
}

#[test]
fn policy_legacy_null_receipts_survive_idempotent_migration_and_prevent_replay() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute_batch("CREATE TABLE sent_alerts(event_id INTEGER NOT NULL, alert_ts INTEGER NOT NULL,
        PRIMARY KEY(event_id, alert_ts)); INSERT INTO sent_alerts VALUES(99, 1);").unwrap();
    add(&conn, 1, NOW - 120);
    conn.execute("INSERT INTO sent_alerts VALUES(1, ?1)", [NOW - 120 + 900]).unwrap();
    alarm::ensure_tracking(&conn).unwrap();
    alarm::ensure_tracking(&conn).unwrap();
    assert_eq!(alarm::tick(&conn, &Utc, NOW, |_| panic!("Legacy delivered receipt replayed"))
        .unwrap().delivered, 0);
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM sent_alerts WHERE schedule_revision IS NULL",
        [], |row| row.get(0)).unwrap();
    assert_eq!(count, 2);
}
