use crate::models::{Event, NewEvent, Recurrence};
use crate::provider::CalendarProvider;
use chrono::{Datelike, Local, NaiveDate, TimeZone};
#[cfg(test)]
use chrono::Duration;
use rusqlite::{params, Connection};
use std::path::PathBuf;

#[path = "recurrence.rs"]
mod recurrence;

// ── Schema ────────────────────────────────────────────────────────────────────

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous  = NORMAL;

CREATE TABLE IF NOT EXISTS events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    title           TEXT    NOT NULL,
    description     TEXT    NOT NULL DEFAULT '',
    start_ts        INTEGER NOT NULL,
    end_ts          INTEGER NOT NULL,
    all_day         INTEGER NOT NULL DEFAULT 0,
    recurrence      TEXT    NOT NULL DEFAULT 'none',
    recurrence_end  INTEGER,
    color           TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_events_start ON events (start_ts);
CREATE INDEX IF NOT EXISTS idx_events_end   ON events (end_ts);
";

// ── Provider ──────────────────────────────────────────────────────────────────

pub struct LocalProvider {
    conn: Connection,
}

impl LocalProvider {
    /// Open (or create) the local calendar database.
    ///
    /// Stored at `~/.local/share/smplos/calendar/events.db`.
    /// WAL mode + NORMAL sync keeps it fast while still crash-safe.
    pub fn open() -> anyhow::Result<Self> {
        let path = Self::db_path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let conn = Connection::open(&path)?;
        conn.execute_batch(SCHEMA)?;
        // Migrations
        Self::migrate_alert_minutes(&conn)?;
        Ok(Self { conn })
    }

    fn db_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".local/share/smplos/calendar/events.db")
    }

    /// Add alert_minutes column if it doesn't exist yet (migration).
    fn migrate_alert_minutes(conn: &Connection) -> anyhow::Result<()> {
        let has_col: bool = conn
            .prepare("SELECT sql FROM sqlite_master WHERE type='table' AND name='events'")
            .and_then(|mut s| s.query_row([], |r| r.get::<_, String>(0)))
            .map(|sql| sql.contains("alert_minutes"))
            .unwrap_or(false);
        if !has_col {
            conn.execute_batch(
                "ALTER TABLE events ADD COLUMN alert_minutes INTEGER NOT NULL DEFAULT 0;",
            )?;
        }
        Ok(())
    }

    // ── Row → model mapping ───────────────────────────────────────────────────

    fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<Event> {
        let id: i64            = row.get(0)?;
        let title: String      = row.get(1)?;
        let desc: String       = row.get(2)?;
        let start_ts: i64      = row.get(3)?;
        let end_ts: i64        = row.get(4)?;
        let all_day: bool      = row.get::<_, i32>(5)? != 0;
        let rec_str: String    = row.get(6)?;
        let rec_end: Option<i64> = row.get(7)?;
        let color: Option<String> = row.get(8)?;
        let alert_minutes: i32 = row.get::<_, Option<i32>>(9)?.unwrap_or(0);

        let start = Local.timestamp_opt(start_ts, 0)
            .single()
            .unwrap_or_else(Local::now);
        let end = Local.timestamp_opt(end_ts, 0)
            .single()
            .unwrap_or_else(Local::now);
        let recurrence_end = rec_end.and_then(|ts| {
            Local.timestamp_opt(ts, 0)
                .single()
                .map(|dt| dt.date_naive())
        });

        Ok(Event { id, title, description: desc, start, end, all_day,
                   recurrence: Recurrence::from_str(&rec_str),
                   recurrence_end, color, alert_minutes })
    }

    // ── Recurring-event expansion ─────────────────────────────────────────────

    /// Expand a single recurring root event into all concrete instances that
    /// fall within `[range_start, range_end)` (Unix seconds).
    ///
    /// Display and reminders share anchored, local-calendar recurrence rules.
    /// Start before the range when an overnight occurrence can overlap it.
    fn expand_recurring(
        event: &Event,
        range_start: i64,
        range_end: i64,
    ) -> Vec<Event> {
        let duration = event.end - event.start;

        let lookback = duration.num_seconds().max(0)
            .saturating_add(if event.all_day { 86400 } else { 0 });
        let starts = match recurrence::starts(
            &Local, event.start, event.recurrence.as_str(), event.recurrence_end,
            range_start.saturating_sub(lookback), range_end,
        ) {
            Ok(starts) => starts,
            Err(_) => {
                eprintln!("smpl-calendar: invalid recurrence range was skipped");
                return Vec::new();
            }
        };
        let mut instances = Vec::new();
        for current in starts {
            let end = Self::occurrence_end(&Local, event.start, event.end, current, event.all_day);
            let Some(end) = end else { continue; };
            if end.timestamp() <= range_start {
                continue;
            }
            let mut instance = event.clone();
            instance.start = current;
            instance.end = end;
            instances.push(instance);
        }
        instances
    }

    fn occurrence_end<Tz: TimeZone>(
        timezone: &Tz,
        original_start: chrono::DateTime<Tz>,
        original_end: chrono::DateTime<Tz>,
        occurrence: chrono::DateTime<Tz>,
        all_day: bool,
    ) -> Option<chrono::DateTime<Tz>> {
        if all_day {
            let days = original_end.date_naive() - original_start.date_naive();
            occurrence.date_naive().checked_add_signed(days)
                .and_then(|date| recurrence::resolve_wall(timezone, date.and_time(original_end.time())))
        } else {
            occurrence.checked_add_signed(original_end - original_start)
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Unix-second timestamps bracketing an entire calendar month.
    fn month_range(year: i32, month: u32) -> (i64, i64) {
        let first = NaiveDate::from_ymd_opt(year, month, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);

        let next_month = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1)
        };
        let last = next_month
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(i64::MAX);

        (first, last)
    }

    /// Fetch all root event rows whose start or recurrence could overlap
    /// `[range_start, range_end)`.
    fn query_range(&self, range_start: i64, range_end: i64) -> Vec<Event> {
        // We fetch events that:
        //   a) start within the range (non-recurring), OR
        //   b) start before the range ends AND are recurring (need expansion).
        // This may over-fetch slightly, but expansion handles the exact filter.
        let sql = "
            SELECT id, title, description, start_ts, end_ts, all_day,
                   recurrence, recurrence_end, color, alert_minutes
            FROM   events
            WHERE  (start_ts < ? AND end_ts > ?)
                OR (recurrence != 'none' AND start_ts < ?)
            ORDER  BY start_ts
        ";
        let mut stmt = self.conn.prepare_cached(sql).unwrap();
        stmt.query_map(
            params![range_end, range_start, range_end],
            Self::row_to_event,
        )
        .unwrap()
        .flatten()
        .collect()
    }
}

// ── CalendarProvider impl ─────────────────────────────────────────────────────

impl CalendarProvider for LocalProvider {
    fn events_for_month(&self, year: i32, month: u32) -> Vec<Event> {
        let (start, end) = Self::month_range(year, month);
        let rows = self.query_range(start, end);

        let mut out = Vec::new();
        for ev in &rows {
            if ev.recurrence == Recurrence::None {
                out.push(ev.clone());
            } else {
                out.extend(Self::expand_recurring(ev, start, end));
            }
        }
        out.sort_by_key(|e| e.start);
        out
    }

    fn events_for_day(&self, date: NaiveDate) -> Vec<Event> {
        let day_start = date
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);
        let day_end = date
            .and_hms_opt(23, 59, 59)
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp())
            .unwrap_or(i64::MAX);

        let rows = self.query_range(day_start, day_end + 1);

        let mut out = Vec::new();
        for ev in &rows {
            if ev.recurrence == Recurrence::None {
                // Check actual overlap with the day
                if ev.start.timestamp() <= day_end && ev.end.timestamp() >= day_start {
                    out.push(ev.clone());
                }
            } else {
                let instances = Self::expand_recurring(ev, day_start, day_end + 1);
                out.extend(instances);
            }
        }

        // All-day first, then chronological by start
        out.sort_by(|a, b| {
            b.all_day.cmp(&a.all_day).then(a.start.cmp(&b.start))
        });
        out
    }

    fn days_with_events(&self, year: i32, month: u32) -> Vec<u32> {
        let events = self.events_for_month(year, month);
        let mut days: Vec<u32> = events
            .iter()
            .filter(|e| e.start.year() == year && e.start.month() == month)
            .map(|e| e.start.day())
            .collect();
        days.sort_unstable();
        days.dedup();
        days
    }

    fn create_event(&mut self, ev: NewEvent) -> anyhow::Result<Event> {
        anyhow::ensure!(
            ev.all_day || ev.end > ev.start,
            "Timed event end must be after its start"
        );
        let now = Local::now().timestamp();
        let rec_end_ts = ev.recurrence_end
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp());

        self.conn.execute(
            "INSERT INTO events
             (title, description, start_ts, end_ts, all_day, recurrence,
              recurrence_end, color, alert_minutes, created_at, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)",
            params![
                ev.title,
                ev.description,
                ev.start.timestamp(),
                ev.end.timestamp(),
                ev.all_day as i32,
                ev.recurrence.as_str(),
                rec_end_ts,
                ev.color,
                ev.alert_minutes,
                now,
            ],
        )?;
        let id = self.conn.last_insert_rowid();

        Ok(Event {
            id,
            title: ev.title,
            description: ev.description,
            start: ev.start,
            end: ev.end,
            all_day: ev.all_day,
            recurrence: ev.recurrence,
            recurrence_end: ev.recurrence_end,
            color: ev.color,
            alert_minutes: ev.alert_minutes,
        })
    }

    fn update_event(&mut self, ev: Event) -> anyhow::Result<()> {
        anyhow::ensure!(
            ev.all_day || ev.end > ev.start,
            "Timed event end must be after its start"
        );
        let now = Local::now().timestamp();
        let rec_end_ts = ev.recurrence_end
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .and_then(|ndt| Local.from_local_datetime(&ndt).single())
            .map(|dt| dt.timestamp());

        self.conn.execute(
            "UPDATE events
             SET title=?2, description=?3, start_ts=?4, end_ts=?5,
                 all_day=?6, recurrence=?7, recurrence_end=?8,
                 color=?9, alert_minutes=?10, updated_at=?11
             WHERE id=?1",
            params![
                ev.id,
                ev.title,
                ev.description,
                ev.start.timestamp(),
                ev.end.timestamp(),
                ev.all_day as i32,
                ev.recurrence.as_str(),
                rec_end_ts,
                ev.color,
                ev.alert_minutes,
                now,
            ],
        )?;
        Ok(())
    }

    fn delete_event(&mut self, id: i64) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM events WHERE id=?1", params![id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
        use super::*;

    fn provider() -> LocalProvider {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        LocalProvider::migrate_alert_minutes(&conn).unwrap();
        LocalProvider { conn }
        }

    fn new_event() -> NewEvent {
        let start = Local.with_ymd_and_hms(2026, 10, 2, 10, 0, 0).single().unwrap();
        NewEvent {
            title: "Fixture".into(),
            description: "Original description".into(),
            start,
            end: start + Duration::minutes(1),
            all_day: false,
            recurrence: Recurrence::None,
            recurrence_end: None,
            color: None,
            alert_minutes: 0,
        }
        }

    #[test]
    fn old_recurring_series_includes_overnight_overlap_and_honors_last_date() {
        let mut provider = provider();
        let mut event = new_event();
        event.start = Local.with_ymd_and_hms(2000, 1, 1, 23, 30, 0).single().unwrap();
        event.end = event.start + Duration::hours(2);
        event.recurrence = Recurrence::Daily;
        event.recurrence_end = NaiveDate::from_ymd_opt(2026, 10, 1);
        provider.create_event(event).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let occurrences = provider.events_for_day(day);
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].start.date_naive(), day.pred_opt().unwrap());
        assert_eq!(occurrences[0].end.date_naive(), day);
        assert!(provider.events_for_day(day.succ_opt().unwrap()).is_empty());
    }

    #[test]
    fn dst_preserves_timed_elapsed_duration_but_all_day_local_date_span() {
        use chrono::Timelike;
        use chrono_tz::America::New_York;
        let before = New_York.with_ymd_and_hms(2026, 3, 7, 1, 30, 0).single().unwrap();
        let spring = New_York.with_ymd_and_hms(2026, 3, 8, 1, 30, 0).single().unwrap();
        let end = LocalProvider::occurrence_end(
            &New_York, before, before + Duration::hours(2), spring, false).unwrap();
        assert_eq!(end - spring, Duration::hours(2));
        assert_eq!((end.hour(), end.minute()), (4, 30));
        let before = New_York.with_ymd_and_hms(2026, 10, 31, 1, 30, 0).single().unwrap();
        let fall = New_York.with_ymd_and_hms(2026, 11, 1, 1, 30, 0).earliest().unwrap();
        let end = LocalProvider::occurrence_end(
            &New_York, before, before + Duration::hours(2), fall, false).unwrap();
        assert_eq!(end - fall, Duration::hours(2));
        assert_eq!((end.hour(), end.minute()), (2, 30));
        let before = New_York.with_ymd_and_hms(2026, 3, 7, 0, 0, 0).single().unwrap();
        let spring = New_York.with_ymd_and_hms(2026, 3, 8, 0, 0, 0).single().unwrap();
        let end = LocalProvider::occurrence_end(
            &New_York, before, before + Duration::minutes(1439), spring, true).unwrap();
        assert_eq!(end.date_naive(), spring.date_naive());
        assert_eq!((end.hour(), end.minute()), (23, 59));
        assert_eq!(end - spring, Duration::minutes(1379));
        let before = New_York.with_ymd_and_hms(2026, 10, 31, 0, 0, 0).single().unwrap();
        let fall = New_York.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).single().unwrap();
        let end = LocalProvider::occurrence_end(
            &New_York, before, before + Duration::minutes(1439), fall, true).unwrap();
        assert_eq!(end.date_naive(), fall.date_naive());
        assert_eq!((end.hour(), end.minute()), (23, 59));
        assert_eq!(end - fall, Duration::minutes(1499));
        let before = New_York.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).single().unwrap();
        let original_end = New_York.with_ymd_and_hms(2026, 3, 3, 23, 59, 0).single().unwrap();
        let start = New_York.with_ymd_and_hms(2026, 3, 7, 0, 0, 0).single().unwrap();
        let end = LocalProvider::occurrence_end(
            &New_York, before, original_end, start, true).unwrap();
        assert_eq!(end.date_naive(), NaiveDate::from_ymd_opt(2026, 3, 9).unwrap());
        assert_eq!((end.hour(), end.minute()), (23, 59));
        assert_eq!(end - start, Duration::minutes(4259));
    }

    #[test]
    fn all_day_recurrence_end_includes_start_date_without_truncating_span() {
        use chrono::Timelike;
        let mut provider = provider();
        let mut event = new_event();
        event.start = Local.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).single().unwrap();
        event.end = Local.with_ymd_and_hms(2000, 1, 2, 23, 59, 0).single().unwrap();
        event.all_day = true;
        event.recurrence = Recurrence::Daily;
        event.recurrence_end = NaiveDate::from_ymd_opt(2026, 10, 1);
        provider.create_event(event).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        let occurrences = provider.events_for_day(day);
        assert_eq!(occurrences.len(), 1);
        assert_eq!(occurrences[0].start.date_naive(), day.pred_opt().unwrap());
        assert_eq!(occurrences[0].end.date_naive(), day);
        assert_eq!((occurrences[0].end.hour(), occurrences[0].end.minute()), (23, 59));
        assert!(provider.events_for_day(day.succ_opt().unwrap()).is_empty());
        let month = provider.events_for_month(2026, 10);
        assert_eq!(month.len(), 2);
        assert_eq!(month[0].start.date_naive(), NaiveDate::from_ymd_opt(2026, 9, 30).unwrap());
        assert_eq!(month[1].start.date_naive(), NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
    }

    fn count(provider: &LocalProvider) -> i64 {
        provider.conn.query_row("SELECT count(*) FROM events", [], |row| row.get(0)).unwrap()
        }

    fn stored(provider: &LocalProvider, id: i64) -> (String, String, i64, i64, i32, i64) {
        provider.conn.query_row(
            "SELECT title,description,start_ts,end_ts,all_day,updated_at FROM events WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        ).unwrap()
        }

    #[test]
    fn invalid_timed_creates_never_write_a_row() {
        let mut provider = provider();
        for duration in [Duration::zero(), Duration::minutes(-1)] {
            let mut event = new_event();
            event.end = event.start + duration;
            assert!(provider.create_event(event).is_err());
            assert_eq!(count(&provider), 0);
        }
        }

    #[test]
    fn invalid_timed_updates_preserve_existing_data() {
        let mut provider = provider();
        let saved = provider.create_event(new_event()).unwrap();
        let before = stored(&provider, saved.id);
        for duration in [Duration::zero(), Duration::minutes(-1)] {
            let mut invalid = saved.clone();
            invalid.title = "Must not be saved".into();
            invalid.description = "Must not replace description".into();
            invalid.end = invalid.start + duration;
            assert!(provider.update_event(invalid).is_err());
            assert_eq!(stored(&provider, saved.id), before);
            assert_eq!(count(&provider), 1);
        }
        }

    #[test]
    fn short_timed_creation_editing_and_overnight_intervals_remain_valid() {
        let mut provider = provider();
        let mut saved = provider.create_event(new_event()).unwrap();
        assert_eq!(saved.end - saved.start, Duration::minutes(1));
        saved.title = "Valid short edit".into();
        saved.end = saved.start + Duration::seconds(1);
        provider.update_event(saved.clone()).unwrap();
        let row = stored(&provider, saved.id);
        assert_eq!(row.0, "Valid short edit");
        assert_eq!(row.3 - row.2, 1);
        let mut overnight = new_event();
        overnight.start = Local.with_ymd_and_hms(2026, 10, 2, 23, 55, 0).single().unwrap();
        overnight.end = overnight.start + Duration::minutes(20);
        let saved = provider.create_event(overnight).unwrap();
        assert_eq!(saved.end.date_naive(), saved.start.date_naive().succ_opt().unwrap());
        assert_eq!(count(&provider), 2);
        }

    #[test]
    fn all_day_events_remain_outside_the_timed_interval_guard() {
        let mut provider = provider();
        let mut all_day = new_event();
        all_day.all_day = true;
        all_day.end = all_day.start;
        let mut saved = provider.create_event(all_day).unwrap();
        saved.title = "All-day edit".into();
        provider.update_event(saved.clone()).unwrap();
        assert_eq!(stored(&provider, saved.id).0, "All-day edit");
        assert_eq!(count(&provider), 1);
    }
}
