//! The bookkeeping behind the Connections page: what is shown, what is being
//! read or switched right now, and which results are still current.
//!
//! It holds no window and starts no threads, so the rules are unit-tested.

use super::airplane::Outcome;
use super::radio::{Radio, Reading, Snapshot};
use super::view::{self, View};

/// Which radios a round of reads covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    /// Every radio, including ones that were missing (opening the page, "Check again").
    All,
    /// Only radios that exist. Asking for a missing Bluetooth adapter can take
    /// seconds, so the periodic poll leaves those alone.
    Present,
}

/// What to push to the window after a result came in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub previous: Snapshot,
    pub shown: Snapshot,
    pub view: View,
}

#[derive(Debug)]
pub struct State {
    busy: bool,
    /// Counts switching actions. A read that started in an earlier epoch may have
    /// seen the radios before the switch, so its result is dropped.
    epoch: u64,
    shown: Snapshot,
    airplane: bool,
    reading: [bool; 2],
}

fn slot(radio: Radio) -> usize {
    match radio {
        Radio::Wifi => 0,
        Radio::Bluetooth => 1,
    }
}

impl State {
    pub fn new() -> Self {
        Self {
            busy: false,
            epoch: 0,
            shown: Snapshot::unread(),
            airplane: false,
            reading: [false; 2],
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The radios to read now, each with the epoch its result must still match.
    /// Nothing is read while a switch is under way, and a radio that is already
    /// being read is not read twice.
    pub fn begin_reads(&mut self, reads: Reads) -> Vec<(Radio, u64)> {
        if self.busy {
            return Vec::new();
        }
        let mut jobs = Vec::new();
        for radio in Radio::ALL {
            let skip = self.reading[slot(radio)]
                || (reads == Reads::Present && *self.shown.get(radio) == Reading::Missing);
            if !skip {
                self.reading[slot(radio)] = true;
                jobs.push((radio, self.epoch));
            }
        }
        jobs
    }

    /// Take a finished read. `None` when the result is out of date.
    pub fn finish_read(&mut self, radio: Radio, epoch: u64, reading: Reading) -> Option<Change> {
        self.reading[slot(radio)] = false;
        if self.busy || epoch != self.epoch {
            return None;
        }
        let next = view::with_reading(&self.shown, radio, reading);
        Some(self.show(next))
    }

    /// Start a switching action. Returns what is shown now, or `None` when
    /// another action is still under way.
    pub fn begin_action(&mut self) -> Option<Snapshot> {
        if self.busy {
            return None;
        }
        self.busy = true;
        self.epoch += 1;
        Some(self.shown.clone())
    }

    /// An action's own read-back is the freshest view of the radios, so it
    /// replaces what was shown outright.
    pub fn finish_action(&mut self, outcome: &Outcome) -> Change {
        self.busy = false;
        self.epoch += 1;
        self.show(outcome.snapshot.clone())
    }

    fn show(&mut self, next: Snapshot) -> Change {
        let previous = std::mem::replace(&mut self.shown, next);
        let view = View::of(&self.shown, self.airplane);
        self.airplane = view.airplane_on;
        Change { previous, shown: self.shown.clone(), view }
    }
}
