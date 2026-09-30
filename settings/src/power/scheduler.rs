use super::Action;
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Request {
    Refresh,
    Timer(Action, i32),
    Profile(i32),
}

#[derive(Default)]
pub(super) struct Scheduler {
    running: Option<Request>,
    pending: VecDeque<Request>,
    failure: Option<String>,
}

pub(super) struct Completion {
    pub publish: bool,
    pub next: Option<Request>,
}

impl Scheduler {
    pub fn enqueue(&mut self, request: Request) -> Option<Request> {
        if self.running.is_none() {
            self.running = Some(request);
            return Some(request);
        }
        if request != Request::Refresh
            && self.pending.back().or(self.running.as_ref()) != Some(&request)
        {
            self.pending.push_back(request);
        }
        None
    }

    pub fn mutating(&self) -> bool {
        self.running.is_some_and(|r| r != Request::Refresh) || !self.pending.is_empty()
    }

    pub fn finish(&mut self) -> Completion {
        let current = self.running.take().expect("a power worker is running");
        let publish = current != Request::Refresh || self.pending.is_empty();
        let next = self.pending.pop_front();
        self.running = next;
        Completion { publish, next }
    }

    pub fn abort(&mut self) {
        self.running = None;
        self.pending.clear();
        self.failure = Some(
            "Power worker or UI delivery failed; queued selections were cancelled. Retry the desired settings."
                .into(),
        );
    }

    pub fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_poll_accepts_click_and_cannot_publish_over_queued_selection() {
        let mut scheduler = Scheduler::default();
        assert_eq!(scheduler.enqueue(Request::Refresh), Some(Request::Refresh));
        assert!(!scheduler.mutating());
        let click = Request::Timer(Action::Dpms, 2);
        assert_eq!(scheduler.enqueue(click), None);
        assert!(scheduler.mutating());
        assert_eq!(scheduler.enqueue(Request::Refresh), None);
        let done = scheduler.finish();
        assert!(!done.publish);
        assert_eq!(done.next, Some(click));
        let done = scheduler.finish();
        assert!(done.publish);
        assert_eq!(done.next, None);
        assert!(!scheduler.mutating());
    }

    #[test]
    fn distinct_choices_are_fifo_ahead_of_polls_with_no_duplicate_workers() {
        let mut scheduler = Scheduler::default();
        scheduler.enqueue(Request::Refresh);
        let choices = [
            Request::Timer(Action::Lock, 2),
            Request::Timer(Action::Dpms, 3),
            Request::Profile(0),
            Request::Timer(Action::Lock, 4),
        ];
        for choice in choices {
            assert_eq!(scheduler.enqueue(choice), None);
            assert_eq!(scheduler.enqueue(choice), None);
            assert_eq!(scheduler.enqueue(Request::Refresh), None);
        }
        for (index, choice) in choices.into_iter().enumerate() {
            let done = scheduler.finish();
            assert_eq!(done.publish, index != 0);
            assert_eq!(done.next, Some(choice));
        }
        assert!(scheduler.finish().next.is_none());
        assert!(!scheduler.mutating());
        assert_eq!(scheduler.enqueue(Request::Refresh), Some(Request::Refresh));
    }

    #[test]
    fn later_return_to_an_earlier_choice_is_not_dropped() {
        let mut scheduler = Scheduler::default();
        let first = Request::Timer(Action::Lock, 1);
        let second = Request::Timer(Action::Lock, 2);
        assert_eq!(scheduler.enqueue(first), Some(first));
        assert_eq!(scheduler.enqueue(first), None);
        scheduler.enqueue(second);
        scheduler.enqueue(first);
        assert_eq!(scheduler.finish().next, Some(second));
        assert_eq!(scheduler.finish().next, Some(first));
        assert_eq!(scheduler.finish().next, None);
    }

    #[test]
    fn worker_failure_clears_busy_and_reports_cancelled_choices() {
        let mut scheduler = Scheduler::default();
        scheduler.enqueue(Request::Refresh);
        scheduler.enqueue(Request::Profile(2));
        scheduler.abort();
        assert!(!scheduler.mutating());
        assert!(scheduler.take_failure().unwrap().contains("cancelled"));
        assert!(scheduler.take_failure().is_none());
        assert_eq!(
            scheduler.enqueue(Request::Profile(1)),
            Some(Request::Profile(1))
        );
        assert_eq!(scheduler.finish().next, None);
    }
}
