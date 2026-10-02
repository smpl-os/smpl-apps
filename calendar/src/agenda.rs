//! Presentation-only compact agenda ordering and current-time policy.

use crate::models::Event;
use chrono::{DateTime, Local, NaiveDate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Event(usize),
    Now,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Update {
    Open,
    Selection,
    Refresh,
}

pub fn is_ongoing(event: &Event, selected: NaiveDate, now: DateTime<Local>) -> bool {
    selected == now.date_naive() && !event.all_day && event.start <= now && now < event.end
}

/// The divider separates timed events that have started from future starts;
/// it is not a proportional timeline. All-day rows always remain above it.
pub fn rows(events: &[Event], selected: NaiveDate, now: DateTime<Local>) -> Vec<Row> {
    let mut indices: Vec<_> = (0..events.len()).collect();
    indices.sort_by_key(|&index| (!events[index].all_day, events[index].start));
    let today = selected == now.date_naive();
    let mut inserted = !today;
    let mut rows = Vec::with_capacity(events.len() + usize::from(today));
    for index in indices {
        let event = &events[index];
        if !inserted && !event.all_day && event.start > now {
            rows.push(Row::Now);
            inserted = true;
        }
        rows.push(Row::Event(index));
    }
    if !inserted {
        rows.push(Row::Now);
    }
    rows
}

pub fn should_scroll_to_now(
    selected: NaiveDate,
    today: NaiveDate,
    update: Update,
    compact_visible: bool,
) -> bool {
    compact_visible && selected == today && matches!(update, Update::Open | Update::Selection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Recurrence;
    use chrono::{Duration, TimeZone};

    fn instant(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 2, hour, minute, 0)
            .single()
            .unwrap()
    }

    fn event(id: i64, start: u32, end: u32, all_day: bool) -> Event {
        Event {
            id,
            title: format!("Event {id}"),
            description: String::new(),
            start: instant(start, 0),
            end: instant(end, 0),
            all_day,
            recurrence: Recurrence::None,
            recurrence_end: None,
            color: None,
            alert_minutes: 0,
        }
    }

    #[test]
    fn ongoing_is_start_inclusive_end_exclusive_and_timed_only() {
        let timed = event(1, 10, 11, false);
        let today = timed.start.date_naive();
        for (now, expected) in [
            (instant(9, 59), false),
            (instant(10, 0), true),
            (instant(10, 30), true),
            (instant(11, 0), false),
            (instant(11, 1), false),
        ] {
            assert_eq!(is_ongoing(&timed, today, now), expected);
        }
        assert!(!is_ongoing(&event(2, 0, 23, true), today, instant(10, 30)));
        assert!(!is_ongoing(
            &timed,
            today.succ_opt().unwrap(),
            instant(10, 30)
        ));
        assert!(!is_ongoing(&event(3, 10, 10, false), today, instant(10, 0)));
    }

    #[test]
    fn overlaps_and_overnight_events_are_each_marked() {
        let now = instant(10, 30);
        let mut overnight = event(3, 0, 11, false);
        overnight.start -= Duration::hours(1);
        overnight.recurrence = Recurrence::Daily;
        let events = [event(1, 9, 11, false), event(2, 10, 12, false), overnight];
        assert!(events
            .iter()
            .all(|event| is_ongoing(event, now.date_naive(), now)));
    }

    #[test]
    fn divider_orders_all_day_then_started_then_future_without_filtering() {
        let now = instant(10, 30);
        let events = [
            event(1, 12, 13, false),
            event(2, 0, 23, true),
            event(3, 10, 11, false),
            event(4, 8, 9, false),
            event(5, 9, 12, false),
        ];
        assert_eq!(
            rows(&events, now.date_naive(), now),
            vec![
                Row::Event(1),
                Row::Event(3),
                Row::Event(4),
                Row::Event(2),
                Row::Now,
                Row::Event(0)
            ]
        );
        assert_eq!(
            rows(&events, now.date_naive().succ_opt().unwrap(), now),
            vec![
                Row::Event(1),
                Row::Event(3),
                Row::Event(4),
                Row::Event(2),
                Row::Event(0)
            ]
        );
    }

    #[test]
    fn empty_all_day_and_midnight_views_do_not_invent_timed_events() {
        let now = instant(23, 59);
        let today = now.date_naive();
        assert_eq!(rows(&[], today, now), vec![Row::Now]);
        assert!(rows(&[], today.pred_opt().unwrap(), now).is_empty());
        assert_eq!(
            rows(&[event(1, 0, 23, true)], today, now),
            vec![Row::Event(0), Row::Now]
        );
        let tomorrow = now + Duration::minutes(1);
        assert!(rows(&[], today, tomorrow).is_empty());
        assert_eq!(rows(&[], tomorrow.date_naive(), tomorrow), vec![Row::Now]);
    }

    #[test]
    fn refreshes_preserve_manual_scroll_and_hidden_views_do_not_scroll() {
        let today = instant(12, 0).date_naive();
        for update in [Update::Open, Update::Selection] {
            assert!(should_scroll_to_now(today, today, update, true));
            assert!(!should_scroll_to_now(today, today, update, false));
            assert!(!should_scroll_to_now(
                today.pred_opt().unwrap(),
                today,
                update,
                true
            ));
        }
        for _ in 0..120 {
            assert!(!should_scroll_to_now(today, today, Update::Refresh, true));
        }
        assert!(should_scroll_to_now(today, today, Update::Selection, true));
    }
}
