//! Mode state machine.
//!
//! The daemon is always in exactly one mode. Modes are entered by IPC
//! commands from the CLI and exit automatically after action (or on
//! Escape).
//!
//! ```text
//!                       CLI: hints click
//!                              │
//!                              ▼
//!    ┌───────┐   CLI: cmd   ┌─────────────┐   hint typed    ┌────────┐
//!    │ Idle  │ ───────────▶ │ Selecting   │ ──────────────▶ │ Action │
//!    │       │              │ (target=..) │                  │  fire  │
//!    │       │ ◀─────────── │             │ ◀───── Escape ── │        │
//!    └───────┘   Idle       └─────────────┘                  └────────┘
//! ```

use crate::hint::{self, Match};

/// What the daemon is doing right now.
#[derive(Clone, Debug)]
pub enum State {
    /// Nothing overlaid, no key grab.
    Idle,
    /// Hint overlay is visible; keystrokes are filtered against `prefix`.
    Selecting {
        target: Target,
        /// Currently-typed prefix (filters visible labels live).
        prefix: String,
        /// The set of labels currently painted on screen.
        labels: Vec<String>,
        /// For drag mode: the first hint chosen (source). None otherwise.
        drag_source: Option<usize>,
    },
    /// hjkl cursor motion mode. Overlay shows a small status bar.
    Cursor,
    /// Scroll mode. j/k scroll the widget under the cursor.
    Scroll,
}

/// What the user wants to happen when they select a hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Click,
    RightClick,
    Hover,
    Drag,
}

/// Outcome of feeding a key into the state machine.
#[derive(Debug)]
pub enum Transition {
    /// Do nothing (state unchanged).
    Nothing,
    /// Redraw overlay (labels or prefix changed).
    Redraw,
    /// Commit: perform `target` action at hint index `label_index`.
    Commit { target: Target, label_index: usize },
    /// Drag stage 1 → 2: source hint chosen, re-enumerate & await dest.
    DragSourceChosen { source_label_index: usize },
    /// Exit mode (Escape, or successful action).
    Exit,
}

impl State {
    /// Feed a single character keystroke into the current mode.
    ///
    /// Returns the state transition to perform. The daemon is responsible
    /// for actually mutating `self` based on the transition.
    pub fn on_key(&mut self, ch: char) -> Transition {
        match self {
            State::Idle => Transition::Nothing,
            State::Cursor | State::Scroll => Transition::Nothing, // handled by daemon
            State::Selecting { target, prefix, labels, drag_source } => {
                prefix.push(ch);
                match hint::match_prefix(labels, prefix) {
                    Match::None => {
                        // Wrong key — reset filter, keep overlay open.
                        prefix.clear();
                        Transition::Redraw
                    }
                    Match::Partial => Transition::Redraw,
                    Match::Exact(label) => {
                        let idx = labels.iter().position(|l| l == label)
                            .expect("Exact match must exist in labels");
                        match (*target, drag_source.is_some()) {
                            (Target::Drag, false) => {
                                Transition::DragSourceChosen { source_label_index: idx }
                            }
                            _ => Transition::Commit { target: *target, label_index: idx },
                        }
                    }
                }
            }
        }
    }

    /// Reset to Idle unconditionally.
    pub fn reset(&mut self) { *self = State::Idle; }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selecting(labels: &[&str], target: Target) -> State {
        State::Selecting {
            target,
            prefix: String::new(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            drag_source: None,
        }
    }

    #[test]
    fn typing_wrong_key_resets_prefix() {
        let mut s = selecting(&["aa", "ab"], Target::Click);
        let t = s.on_key('z');
        assert!(matches!(t, Transition::Redraw));
        if let State::Selecting { prefix, .. } = &s {
            assert!(prefix.is_empty());
        } else {
            panic!("expected Selecting");
        }
    }

    #[test]
    fn typing_first_char_partial() {
        let mut s = selecting(&["aa", "ab"], Target::Click);
        let t = s.on_key('a');
        assert!(matches!(t, Transition::Redraw));
    }

    #[test]
    fn typing_full_label_commits_click() {
        let mut s = selecting(&["aa", "bb"], Target::Click);
        let _ = s.on_key('a');
        let t = s.on_key('a');
        assert!(matches!(t, Transition::Commit { target: Target::Click, label_index: 0 }));
    }

    #[test]
    fn drag_first_pick_transitions_to_source_chosen() {
        let mut s = selecting(&["aa", "bb"], Target::Drag);
        let _ = s.on_key('a');
        let t = s.on_key('a');
        assert!(matches!(t, Transition::DragSourceChosen { source_label_index: 0 }));
    }
}
