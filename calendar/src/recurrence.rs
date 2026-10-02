//! Shared calendar-wall-time recurrence calculation, without persistence.
//! Gap dates are skipped; folds yield one earlier instant. Month/year repeats
//! clamp each target date independently, preserving the original date anchor.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Datelike, Days, LocalResult, Months, NaiveDate, NaiveDateTime, TimeZone};

const MAX_STEPS: usize = 4096;

pub fn resolve_wall<Tz: TimeZone>(timezone: &Tz, wall: NaiveDateTime) -> Option<DateTime<Tz>> {
    match timezone.from_local_datetime(&wall) {
        LocalResult::Single(time) => Some(time),
        LocalResult::Ambiguous(first, second) => Some(first.min(second)),
        LocalResult::None => None,
    }
}

pub fn starts<Tz: TimeZone>(
    timezone: &Tz,
    seed: DateTime<Tz>,
    cadence: &str,
    until: Option<NaiveDate>,
    from: i64,
    to: i64,
) -> Result<Vec<DateTime<Tz>>> {
    if from >= to {
        return Ok(Vec::new());
    }
    let (days, months) = match cadence {
        "none" => {
            return Ok(if from <= seed.timestamp() && seed.timestamp() < to {
                vec![seed]
            } else {
                Vec::new()
            });
        }
        "daily" => (1, 0),
        "weekly" => (7, 0),
        "biweekly" => (14, 0),
        "monthly" => (0, 1),
        "yearly" => (0, 12),
        _ => bail!("Unsupported recurrence"),
    };
    let first_date = timezone
        .timestamp_opt(from, 0)
        .single()
        .context("Recurrence range start is invalid")?
        .date_naive();
    let last_date = timezone
        .timestamp_opt(to - 1, 0)
        .single()
        .context("Recurrence range end is invalid")?
        .date_naive();
    let anchor = seed.date_naive();
    let mut index = if days > 0 {
        ((first_date - anchor).num_days() / days)
            .saturating_sub(1)
            .max(0) as u64
    } else {
        let difference = i64::from(first_date.year() - anchor.year()) * 12
            + i64::from(first_date.month())
            - i64::from(anchor.month());
        (difference / months).saturating_sub(1).max(0) as u64
    };
    let mut result = Vec::new();
    for _ in 0..MAX_STEPS {
        let date = if days > 0 {
            let amount = index
                .checked_mul(days as u64)
                .context("Recurrence date overflow")?;
            anchor.checked_add_days(Days::new(amount))
        } else {
            let amount = index
                .checked_mul(months as u64)
                .and_then(|value| u32::try_from(value).ok())
                .context("Recurrence date overflow")?;
            anchor.checked_add_months(Months::new(amount))
        }
        .context("Recurrence date out of range")?;
        if date > last_date || until.is_some_and(|end| date > end) {
            return Ok(result);
        }
        let occurrence = if index == 0 {
            Some(seed.clone())
        } else {
            resolve_wall(timezone, date.and_time(seed.time()))
        };
        if let Some(occurrence) = occurrence {
            if occurrence.timestamp() >= to {
                return Ok(result);
            }
            if occurrence.timestamp() >= from {
                result.push(occurrence);
            }
        }
        index = index.checked_add(1).context("Recurrence index overflow")?;
    }
    bail!("Recurrence query exceeds 4096 occurrences")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Timelike, Utc};
    use chrono_tz::America::New_York;

    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().unwrap()
    }

    #[test]
    fn old_daily_series_fast_forwards_without_iteration_age_limit() {
        let occurrences = starts(
            &Utc,
            utc("2000-01-01T10:00:00Z"),
            "daily",
            None,
            utc("2026-10-01T00:00:00Z").timestamp(),
            utc("2026-11-01T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(occurrences.len(), 31);
        assert!(occurrences
            .iter()
            .all(|time| time.year() == 2026 && time.month() == 10));
    }

    #[test]
    fn weekly_and_biweekly_keep_weekday_and_wall_clock_over_dst() {
        let seed = utc("2026-03-01T14:00:00Z").with_timezone(&New_York);
        for (cadence, dates) in [("weekly", vec![8, 15]), ("biweekly", vec![15])] {
            let occurrences = starts(
                &New_York,
                seed,
                cadence,
                None,
                utc("2026-03-08T00:00:00Z").timestamp(),
                utc("2026-03-16T00:00:00Z").timestamp(),
            )
            .unwrap();
            assert_eq!(
                occurrences
                    .iter()
                    .map(|time| time.day())
                    .collect::<Vec<_>>(),
                dates
            );
            assert!(occurrences
                .iter()
                .all(|time| time.hour() == 9 && time.weekday() == seed.weekday()));
        }
    }

    #[test]
    fn old_monthly_roots_fast_forward_and_oversized_queries_fail_explicitly() {
        let occurrences = starts(
            &Utc,
            utc("1900-01-31T10:00:00Z"),
            "monthly",
            None,
            utc("2026-10-01T00:00:00Z").timestamp(),
            utc("2026-11-01T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(occurrences, [utc("2026-10-31T10:00:00Z")]);
        assert!(starts(
            &Utc,
            utc("1900-01-01T10:00:00Z"),
            "daily",
            None,
            utc("1900-01-01T00:00:00Z").timestamp(),
            utc("2026-11-01T00:00:00Z").timestamp(),
        )
        .is_err());
    }

    #[test]
    fn original_date_is_included_on_recurrence_end_and_none_ignores_end() {
        let seed = utc("2026-01-31T10:00:00Z");
        for cadence in ["daily", "weekly", "biweekly", "monthly", "yearly"] {
            assert_eq!(
                starts(
                    &Utc,
                    seed,
                    cadence,
                    Some(seed.date_naive()),
                    seed.timestamp(),
                    seed.timestamp() + 86400 * 40,
                )
                .unwrap(),
                [seed]
            );
        }
        assert_eq!(
            starts(
                &Utc,
                seed,
                "none",
                Some("2020-01-01".parse().unwrap()),
                seed.timestamp(),
                seed.timestamp() + 1,
            )
            .unwrap(),
            [seed]
        );
    }

    #[test]
    fn month_and_year_repeats_keep_original_anchor_and_inclusive_end_date() {
        let monthly = starts(
            &Utc,
            utc("2026-01-31T10:00:00Z"),
            "monthly",
            Some("2026-03-31".parse().unwrap()),
            utc("2026-02-01T00:00:00Z").timestamp(),
            utc("2026-05-01T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(
            monthly
                .iter()
                .map(|time| time.date_naive().to_string())
                .collect::<Vec<_>>(),
            ["2026-02-28", "2026-03-31"]
        );
        let yearly = starts(
            &Utc,
            utc("2024-02-29T10:00:00Z"),
            "yearly",
            None,
            utc("2028-01-01T00:00:00Z").timestamp(),
            utc("2029-01-01T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(yearly[0].date_naive().to_string(), "2028-02-29");
    }

    #[test]
    fn calendar_repeats_keep_wall_clock_across_dst() {
        let seed = utc("2026-03-07T14:00:00Z").with_timezone(&New_York);
        let occurrences = starts(
            &New_York,
            seed,
            "daily",
            None,
            utc("2026-03-07T00:00:00Z").timestamp(),
            utc("2026-03-10T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert!(occurrences.iter().all(|time| time.hour() == 9));
        assert_eq!(
            occurrences[1].clone() - occurrences[0].clone(),
            Duration::hours(23)
        );
    }

    #[test]
    fn gaps_are_skipped_and_folds_fire_once_with_known_original_preserved() {
        let gap_seed = utc("2026-03-07T07:30:00Z").with_timezone(&New_York);
        let gap = starts(
            &New_York,
            gap_seed,
            "daily",
            None,
            utc("2026-03-08T00:00:00Z").timestamp(),
            utc("2026-03-10T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(gap.len(), 1);
        assert_eq!(gap[0].date_naive().to_string(), "2026-03-09");
        let fold_seed = utc("2026-10-31T05:30:00Z").with_timezone(&New_York);
        let fold = starts(
            &New_York,
            fold_seed,
            "daily",
            None,
            utc("2026-11-01T00:00:00Z").timestamp(),
            utc("2026-11-02T00:00:00Z").timestamp(),
        )
        .unwrap();
        assert_eq!(fold.len(), 1);
        assert_eq!(fold[0].timestamp(), utc("2026-11-01T05:30:00Z").timestamp());
        let original = utc("2026-11-01T06:30:00Z").with_timezone(&New_York);
        assert_eq!(
            starts(
                &New_York,
                original,
                "daily",
                None,
                utc("2026-11-01T00:00:00Z").timestamp(),
                utc("2026-11-02T00:00:00Z").timestamp()
            )
            .unwrap()[0],
            original
        );
    }
}
