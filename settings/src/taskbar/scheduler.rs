use super::Setting;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Refresh,
    Set(Setting, i32),
}

#[derive(Default)]
pub(super) struct Scheduler {
    running: Option<Request>,
    pending: VecDeque<Request>,
    failed: bool,
}

impl Scheduler {
    pub fn enqueue(&mut self, request: Request) -> Option<Request> {
        if self.running.is_none() {
            self.running = Some(request);
            return Some(request);
        }
        if let Request::Set(setting, _) = request {
            // Only the newest pending value for each control needs applying.
            self.pending
                .retain(|r| !matches!(r, Request::Set(s, _) if *s == setting));
            if self.running != Some(request) {
                self.pending.push_back(request);
            }
        }
        None
    }

    pub fn finish(&mut self) -> (bool, Option<Request>) {
        let current = self.running.take().expect("a taskbar worker is running");
        let publish = current != Request::Refresh || self.pending.is_empty();
        self.running = self.pending.pop_front();
        (publish, self.running)
    }

    pub fn mutating(&self) -> bool {
        matches!(self.running, Some(Request::Set(..))) || !self.pending.is_empty()
    }

    pub fn abort(&mut self) {
        self.running = None;
        self.pending.clear();
        self.failed = true;
    }

    pub fn take_failure(&mut self) -> bool {
        std::mem::take(&mut self.failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_refresh_cannot_overwrite_queued_choices_and_never_starts_two_workers() {
        let mut scheduler = Scheduler::default();
        assert_eq!(scheduler.enqueue(Request::Refresh), Some(Request::Refresh));
        assert!(!scheduler.mutating());
        for value in 1..=10 {
            assert_eq!(scheduler.enqueue(Request::Set(Setting::Count, value)), None);
            assert_eq!(scheduler.enqueue(Request::Refresh), None);
        }
        assert!(scheduler.mutating());
        assert_eq!(
            scheduler.finish(),
            (false, Some(Request::Set(Setting::Count, 10)))
        );
        assert_eq!(scheduler.finish(), (true, None));
        assert!(!scheduler.mutating());
    }

    #[test]
    fn distinct_controls_survive_coalescing_in_latest_request_order() {
        let mut scheduler = Scheduler::default();
        scheduler.enqueue(Request::Set(Setting::Count, 7));
        for request in [
            Request::Set(Setting::Count, 3),
            Request::Set(Setting::Style, 1),
            Request::Set(Setting::Clock24h, 1),
            Request::Set(Setting::Count, 5),
            Request::Set(Setting::Spacing, 9),
            Request::Set(Setting::Spacing, 9),
        ] {
            assert_eq!(scheduler.enqueue(request), None);
        }
        for request in [
            Request::Set(Setting::Style, 1),
            Request::Set(Setting::Clock24h, 1),
            Request::Set(Setting::Count, 5),
            Request::Set(Setting::Spacing, 9),
        ] {
            assert_eq!(scheduler.finish(), (true, Some(request)));
        }
        assert_eq!(scheduler.finish(), (true, None));
    }

    #[test]
    fn return_to_running_value_discards_superseded_pending_value() {
        let mut scheduler = Scheduler::default();
        let request = Request::Set(Setting::Count, 7);
        scheduler.enqueue(request);
        scheduler.enqueue(Request::Set(Setting::Count, 3));
        scheduler.enqueue(request);
        assert_eq!(scheduler.finish(), (true, None));
    }

    #[test]
    fn delivery_failure_clears_busy_and_allows_retry() {
        let mut scheduler = Scheduler::default();
        scheduler.enqueue(Request::Set(Setting::Count, 7));
        scheduler.enqueue(Request::Set(Setting::Style, 1));
        scheduler.abort();
        assert!(!scheduler.mutating());
        assert!(scheduler.take_failure());
        assert!(!scheduler.take_failure());
        assert_eq!(scheduler.enqueue(Request::Refresh), Some(Request::Refresh));
        assert_eq!(scheduler.finish(), (true, None));
    }
}
