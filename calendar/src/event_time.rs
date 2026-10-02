//! Event-editor wall-time validation and explicit start-change adjustment.
//!
//! Resolving a manual interval never lengthens it. Call `auto_end` only after
//! an explicit start/date change or a switch to timed mode, not on editor open.

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Duration, Local, LocalResult, NaiveDate, TimeZone, Timelike};

/// Resolve independent start/end dates in the calendar's local timezone.
/// Unchanged displayed minutes retain their original instants, including
/// seconds and the chosen side of an ambiguous DST fold.
#[allow(clippy::too_many_arguments, reason = "Validate the editor's independent raw date/time fields before constructing timestamps")]
pub fn resolve_interval(
    start_date: NaiveDate,
    start_hour: i32,
    start_minute: i32,
    end_date: NaiveDate,
    end_hour: i32,
    end_minute: i32,
    all_day: bool,
    original: Option<(DateTime<Local>, DateTime<Local>)>,
) -> Result<(DateTime<Local>, DateTime<Local>)> {
    resolve_interval_in(
        &Local,
        start_date,
        start_hour,
        start_minute,
        end_date,
        end_hour,
        end_minute,
        all_day,
        original,
    )
}

/// Preserve a sufficiently late end; otherwise use exactly twenty elapsed
/// minutes after the new start, carrying the date and timezone offset correctly.
pub fn auto_end(
    start: DateTime<Local>,
    current_end: Option<DateTime<Local>>,
) -> Result<DateTime<Local>> {
    auto_end_in(start, current_end)
}

/// Resolve a single editor boundary before an explicit automatic adjustment.
pub fn resolve_local(
    date: NaiveDate,
    hour: i32,
    minute: i32,
    original: Option<DateTime<Local>>,
) -> Result<DateTime<Local>> {
    resolve_wall(&Local, date, hour, minute, original, "Event")
}

#[allow(clippy::too_many_arguments, reason = "Timezone-injected version of the editor's raw-field validator")]
fn resolve_interval_in<Tz: TimeZone>(
    timezone: &Tz,
    start_date: NaiveDate,
    start_hour: i32,
    start_minute: i32,
    end_date: NaiveDate,
    end_hour: i32,
    end_minute: i32,
    all_day: bool,
    original: Option<(DateTime<Tz>, DateTime<Tz>)>,
) -> Result<(DateTime<Tz>, DateTime<Tz>)> {
    if all_day {
        return Ok((
            resolve_wall(timezone, start_date, 0, 0, None, "Start")?,
            resolve_wall(timezone, start_date, 23, 59, None, "End")?,
        ));
    }
    let (old_start, old_end) = match original {
        Some((start, end)) => (Some(start), Some(end)),
        None => (None, None),
    };
    let start = resolve_wall(
        timezone,
        start_date,
        start_hour,
        start_minute,
        old_start,
        "Start",
    )?;
    let end = resolve_wall(timezone, end_date, end_hour, end_minute, old_end, "End")?;
    ensure!(end > start, "Timed event end must be after its start");
    Ok((start, end))
}

fn resolve_wall<Tz: TimeZone>(
    timezone: &Tz,
    date: NaiveDate,
    hour: i32,
    minute: i32,
    original: Option<DateTime<Tz>>,
    label: &str,
) -> Result<DateTime<Tz>> {
    ensure!(
        (0..24).contains(&hour),
        "{label} hour must be between 0 and 23"
    );
    ensure!(
        (0..60).contains(&minute),
        "{label} minute must be between 0 and 59"
    );
    if let Some(original) = original {
        let local = original.with_timezone(timezone);
        if local.date_naive() == date
            && local.hour() == hour as u32
            && local.minute() == minute as u32
        {
            return Ok(local);
        }
    }
    let wall = date
        .and_hms_opt(hour as u32, minute as u32, 0)
        .context("Event date/time is out of range")?;
    match timezone.from_local_datetime(&wall) {
        LocalResult::Single(instant) => Ok(instant),
        LocalResult::None => bail!("{label} time does not exist in this timezone (DST gap)"),
        LocalResult::Ambiguous(_, _) => {
            bail!("{label} time is ambiguous in this timezone (DST fold); choose another time")
        }
    }
}

fn auto_end_in<Tz: TimeZone>(
    start: DateTime<Tz>,
    current_end: Option<DateTime<Tz>>,
) -> Result<DateTime<Tz>> {
    let minimum = start
        .checked_add_signed(Duration::minutes(20))
        .context("Event end is outside the supported date range")?;
    Ok(match current_end {
        Some(end) if end >= minimum => end,
        _ => minimum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use chrono_tz::America::New_York;

    fn date(value: &str) -> NaiveDate {
        value.parse().unwrap()
    }

    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    #[test]
    fn automatic_end_uses_exact_twenty_minutes_and_preserves_later_end() {
        let start = utc("2026-10-02T10:00:00Z");
        let minimum = start + Duration::minutes(20);
        for previous in [
            None,
            Some(start - Duration::minutes(1)),
            Some(start),
            Some(start + Duration::minutes(5)),
            Some(minimum),
        ] {
            assert_eq!(auto_end_in(start, previous).unwrap(), minimum);
        }
        let later = start + Duration::hours(2);
        assert_eq!(auto_end_in(start, Some(later)).unwrap(), later);
        assert!(auto_end_in(DateTime::<Utc>::MAX_UTC, None).is_err());
    }

    #[test]
    fn late_day_adjustment_advances_end_date_instead_of_wrapping() {
        let start = utc("2026-12-31T23:50:00Z");
        assert_eq!(
            auto_end_in(start, None).unwrap(),
            utc("2027-01-01T00:10:00Z")
        );
        let interval = resolve_interval_in(
            &Utc,
            date("2026-12-31"),
            23,
            50,
            date("2027-01-01"),
            0,
            10,
            false,
            None,
        )
        .unwrap();
        assert_eq!(interval.1 - interval.0, Duration::minutes(20));
        assert!(resolve_interval_in(
            &Utc,
            date("2026-12-31"),
            23,
            50,
            date("2026-12-31"),
            0,
            10,
            false,
            None,
        )
        .is_err());
    }

    #[test]
    fn manual_short_intervals_are_allowed_but_equal_reversed_and_invalid_are_not() {
        let day = date("2026-10-02");
        let short = resolve_interval_in(&Utc, day, 10, 0, day, 10, 1, false, None).unwrap();
        assert_eq!(short.1 - short.0, Duration::minutes(1));
        for (hour, minute) in [(10, 0), (9, 59), (24, 0), (10, 60), (-1, 0), (10, -1)] {
            assert!(resolve_interval_in(&Utc, day, 10, 0, day, hour, minute, false, None).is_err());
        }
        for (hour, minute) in [(24, 0), (-1, 0), (0, 60), (0, -1)] {
            assert!(resolve_interval_in(&Utc, day, hour, minute, day, 11, 0, false, None).is_err());
        }
        assert!(resolve_interval_in(
            &Utc,
            day,
            10,
            0,
            day.pred_opt().unwrap(),
            11,
            0,
            false,
            None,
        )
        .is_err());
    }

    #[test]
    fn all_day_keeps_the_selected_start_day_and_ignores_timed_fields() {
        let day = date("2026-10-02");
        let (start, end) = resolve_interval_in(
            &Utc,
            day,
            -1,
            99,
            day.succ_opt().unwrap(),
            99,
            -1,
            true,
            None,
        )
        .unwrap();
        assert_eq!(start, utc("2026-10-02T00:00:00Z"));
        assert_eq!(end, utc("2026-10-02T23:59:00Z"));
    }

    #[test]
    fn dst_gaps_and_new_ambiguous_times_are_explicit_errors() {
        let spring = date("2026-03-08");
        let gap =
            resolve_interval_in(&New_York, spring, 2, 30, spring, 3, 30, false, None).unwrap_err();
        assert!(gap.to_string().contains("does not exist"));
        let gap_end =
            resolve_interval_in(&New_York, spring, 1, 30, spring, 2, 30, false, None).unwrap_err();
        assert!(gap_end.to_string().contains("does not exist"));
        let autumn = date("2026-11-01");
        let fold =
            resolve_interval_in(&New_York, autumn, 1, 15, autumn, 2, 15, false, None).unwrap_err();
        assert!(fold.to_string().contains("ambiguous"));
        assert!(
            resolve_interval_in(&New_York, autumn, 0, 30, autumn, 1, 30, false, None,)
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }

    #[test]
    fn automatic_adjustment_crosses_dst_by_elapsed_time() {
        for (start, expected) in [
            ("2026-03-08T06:50:00Z", "2026-03-08 03:10 -0400"),
            ("2026-11-01T05:50:00Z", "2026-11-01 01:10 -0500"),
        ] {
            let start = utc(start).with_timezone(&New_York);
            let end = auto_end_in(start, None).unwrap();
            assert_eq!(end - start, Duration::minutes(20));
            assert_eq!(end.format("%Y-%m-%d %H:%M %z").to_string(), expected);
        }
    }

    #[test]
    fn unchanged_edits_preserve_fold_choice_seconds_and_short_duration() {
        let old_start = utc("2026-11-01T06:15:42Z").with_timezone(&New_York);
        let old_end = utc("2026-11-01T06:16:07Z").with_timezone(&New_York);
        let day = old_start.date_naive();
        assert_eq!(
            resolve_interval_in(
                &New_York,
                day,
                1,
                15,
                day,
                1,
                16,
                false,
                Some((old_start, old_end)),
            )
            .unwrap(),
            (old_start, old_end)
        );
        assert!(resolve_interval_in(
            &New_York,
            day,
            1,
            14,
            day,
            1,
            16,
            false,
            Some((old_start, old_end)),
        )
        .is_err());
        let day = date("2026-10-02");
        let original = (utc("2026-10-02T10:00:42Z"), utc("2026-10-02T10:01:07Z"));
        let (start, end) =
            resolve_interval_in(&Utc, day, 10, 0, day, 10, 2, false, Some(original)).unwrap();
        assert_eq!(start, original.0);
        assert_eq!(end, utc("2026-10-02T10:02:00Z"));
    }

    #[test]
    fn local_wrappers_validate_without_changing_environment() {
        let day = date("2026-10-02");
        let (start, end) = resolve_interval(day, 10, 0, day, 10, 1, false, None).unwrap();
        assert_eq!(end - start, Duration::minutes(1));
        assert_eq!(
            auto_end(start, Some(end)).unwrap() - start,
            Duration::minutes(20)
        );
    }
}
