//! The Pointer card of the Mouse tab, with no Slint in it: the four options
//! as the controls show them, their save to `pointer.conf`, and the
//! bookkeeping that stops live changes piling up while a slider is dragged.
//!
//! A change is written to the file at once. It reaches the running Hyprland
//! a moment later, on a worker thread, and changes that arrive meanwhile are
//! folded into one more apply. Saving and applying are independent: a file
//! that can't be written still applies, a Hyprland that refuses still leaves
//! the choice in the file, and the status says which of the two worked.
//!
//! `ui` copies a [`PointerView`] into the window.

use std::path::{Path, PathBuf};

use super::pointer::{self, Accel, Pointer};
use super::store::{self, Backups};

/// Live applies run one at a time. A change that comes in while one is
/// running waits for it, and any number of them make one more apply.
#[derive(Debug, Default)]
struct Live {
    running: bool,
    waiting: bool,
}

impl Live {
    /// A change wants applying: true when an apply may start now.
    fn request(&mut self) -> bool {
        if self.running {
            self.waiting = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    /// The running apply ended: true when another must start straight away.
    fn finished(&mut self) -> bool {
        if self.waiting {
            self.waiting = false;
            true
        } else {
            self.running = false;
            false
        }
    }
}

/// Everything the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerView {
    pub sensitivity: f64,
    pub sensitivity_text: String,
    pub accel_names: Vec<String>,
    pub accel_index: i32,
    pub natural_scroll: bool,
    pub scroll_factor: f64,
    pub scroll_factor_text: String,
    /// Every option has its default value (nothing for Reset to do).
    pub at_defaults: bool,
    pub status: String,
    pub status_is_error: bool,
}

pub struct PointerState {
    path: PathBuf,
    current: Pointer,
    /// What the card says before anything is changed: where the values are from.
    note: (String, bool),
    /// The file gave the values. When it didn't, Hyprland is asked.
    from_file: bool,
    /// A control was used: values that arrive from elsewhere no longer count.
    touched: bool,
    saved: Result<(), String>,
    /// `None` while the latest change hasn't reached Hyprland.
    applied: Option<Result<(), String>>,
    /// Counts changes, so an apply that ends after a newer change is known
    /// not to cover it.
    generation: u64,
    sent: u64,
    live: Live,
}

impl PointerState {
    /// Reads `path`; without options in it the defaults show until
    /// `adopt_live` says what Hyprland has.
    pub fn load(path: PathBuf) -> Self {
        let (current, from_file, note) = match store::read(&path) {
            Ok(Some(text)) => match pointer::parse(&text) {
                Some(pointer) => (pointer, true, ("Showing what pointer.conf holds.".to_string(), false)),
                None => (Pointer::default(), false, ("Nothing saved yet. Showing the defaults.".to_string(), false)),
            },
            Ok(None) => (Pointer::default(), false, ("Nothing saved yet. Showing the defaults.".to_string(), false)),
            Err(error) => (Pointer::default(), false, (format!("{error}. Showing the defaults."), true)),
        };
        PointerState {
            path,
            current,
            note,
            from_file,
            touched: false,
            saved: Ok(()),
            applied: None,
            generation: 0,
            sent: 0,
            live: Live::default(),
        }
    }

    /// Nothing saved gave the values, so the running Hyprland should be asked.
    pub fn needs_live_read(&self) -> bool {
        !self.from_file && !self.touched
    }

    /// Hyprland's answer to the question `needs_live_read` raised. A control
    /// used meanwhile wins over it.
    pub fn adopt_live(&mut self, result: Result<Pointer, String>) {
        if self.from_file || self.touched {
            return;
        }
        self.note = match result {
            Ok(pointer) => {
                self.current = pointer;
                ("Nothing saved yet. Showing what Hyprland has now.".to_string(), false)
            }
            Err(error) => (
                format!("Nothing saved yet, and Hyprland didn't say what it has ({}). Showing the defaults.", plain(&error)),
                false,
            ),
        };
    }

    // ── Changes ──────────────────────────────────────────────────────────────

    /// True when the value differs from the one shown. Sliders report every
    /// movement, so a change to the same step is ignored here.
    pub fn set_sensitivity(&mut self, value: f64) -> bool {
        self.change(Pointer { sensitivity: value, ..self.current.clone() })
    }

    pub fn set_accel(&mut self, index: usize) -> bool {
        match Accel::choices(&self.current.accel).get(index) {
            Some(accel) => self.change(Pointer { accel: accel.clone(), ..self.current.clone() }),
            None => false,
        }
    }

    pub fn set_natural_scroll(&mut self, on: bool) -> bool {
        self.change(Pointer { natural_scroll: on, ..self.current.clone() })
    }

    pub fn set_scroll_factor(&mut self, value: f64) -> bool {
        self.change(Pointer { scroll_factor: value, ..self.current.clone() })
    }

    pub fn reset(&mut self) -> bool {
        self.change(Pointer::default())
    }

    fn change(&mut self, next: Pointer) -> bool {
        let next = next.normalized();
        if next == self.current {
            return false;
        }
        self.current = next;
        self.touched = true;
        self.generation += 1;
        self.applied = None;
        self.saved = persist(&self.path, &self.current);
        true
    }

    // ── Applying ─────────────────────────────────────────────────────────────

    /// The delay after the last change is over. `Some` is what to apply to
    /// Hyprland now; `None`: an apply is running and will be followed by one
    /// with the newest values.
    pub fn begin_apply(&mut self) -> Option<Pointer> {
        if !self.live.request() {
            return None;
        }
        self.sent = self.generation;
        Some(self.current.clone())
    }

    /// The apply that was running ended. `Some` is the next one to start.
    pub fn finish_apply(&mut self, result: Result<(), String>) -> Option<Pointer> {
        let covers_latest = self.sent == self.generation;
        if self.live.finished() {
            self.sent = self.generation;
            self.applied = None;
            return Some(self.current.clone());
        }
        if covers_latest {
            self.applied = Some(result);
        }
        None
    }

    // ── What the card shows ──────────────────────────────────────────────────

    pub fn view(&self) -> PointerView {
        let choices = Accel::choices(&self.current.accel);
        let (status, status_is_error) = if self.touched {
            outcome(&self.path, &self.saved, self.applied.as_ref())
        } else {
            self.note.clone()
        };
        PointerView {
            sensitivity: self.current.sensitivity,
            sensitivity_text: pointer::sensitivity_text(self.current.sensitivity),
            accel_names: choices.iter().map(Accel::label).collect(),
            accel_index: choices.iter().position(|a| *a == self.current.accel).map_or(-1, |i| i as i32),
            natural_scroll: self.current.natural_scroll,
            scroll_factor: self.current.scroll_factor,
            scroll_factor_text: pointer::scroll_factor_text(self.current.scroll_factor),
            at_defaults: self.current == Pointer::default(),
            status,
            status_is_error,
        }
    }
}

/// Writes the four options into the file, keeping its other lines.
fn persist(path: &Path, pointer: &Pointer) -> Result<(), String> {
    let existing = store::read(path)?;
    let text = pointer::render(existing.as_deref(), pointer);
    store::save(path, &text, existing.as_deref(), Backups::Skip)
}

/// An error as part of a sentence: no "Not saved:" lead, no closing full stop.
fn plain(error: &str) -> &str {
    error.trim().strip_prefix("Not saved: ").unwrap_or(error.trim()).trim_end_matches('.')
}

/// What the status says once something was changed.
fn outcome(path: &Path, saved: &Result<(), String>, applied: Option<&Result<(), String>>) -> (String, bool) {
    let file = path.file_name().map_or_else(|| "the file".to_string(), |n| n.to_string_lossy().into_owned());
    match (saved, applied) {
        (Ok(()), None) => ("Applying…".to_string(), false),
        (Err(saving), None) => (format!("Not saved ({}). Applying…", plain(saving)), true),
        (Ok(()), Some(Ok(()))) => (format!("Applied and saved to {file}."), false),
        (Ok(()), Some(Err(applying))) => {
            (format!("Saved to {file}, but couldn't apply it now: {}.", plain(applying)), true)
        }
        (Err(saving), Some(Ok(()))) => (format!("Applied now, but not saved: {}.", plain(saving)), true),
        (Err(saving), Some(Err(applying))) => {
            (format!("Not saved ({}) and not applied ({}).", plain(saving), plain(applying)), true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::hyprctl::tests::{reply, MockRunner};
    use crate::mouse::testing::temp_dir;

    const FILE: &str = "# mine\ngeneral {\n    gaps_in = 3\n}\n\ninput {\n    follow_mouse = 1\n    sensitivity = 0.4\n    accel_profile = flat\n    natural_scroll = true\n    scroll_factor = 2\n}\n";

    fn state(name: &str, file: Option<&str>) -> (PointerState, PathBuf) {
        let path = temp_dir(name).join("hypr/pointer.conf");
        if let Some(text) = file {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        (PointerState::load(path.clone()), path)
    }

    fn saved(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    /// A Hyprland that takes whatever it is given.
    fn compositor() -> MockRunner {
        MockRunner::new(|_, args| match args {
            ["systeminfo"] => reply("configProvider: lua\n"),
            _ => reply("ok\n"),
        })
    }

    #[test]
    fn it_starts_from_the_file() {
        let (state, _) = state("ps-file", Some(FILE));
        let view = state.view();
        assert_eq!((view.sensitivity, view.natural_scroll, view.scroll_factor), (0.4, true, 2.0));
        assert_eq!(view.accel_names, ["System default", "Flat (no acceleration)", "Adaptive (faster when you move faster)"]);
        assert_eq!(view.accel_index, 1);
        assert_eq!((view.sensitivity_text.as_str(), view.scroll_factor_text.as_str()), ("+0.4", "2×"));
        assert!(!view.at_defaults);
        assert_eq!((view.status.as_str(), view.status_is_error), ("Showing what pointer.conf holds.", false));
        assert!(!state.needs_live_read());
    }

    #[test]
    fn without_saved_options_it_shows_the_defaults_and_asks_hyprland() {
        for (name, file) in [("ps-none", None), ("ps-empty", Some("# nothing here\n")), ("ps-other", Some("general {\n gaps_in = 2\n}\n"))] {
            let (mut state, path) = state(name, file);
            assert!(state.needs_live_read(), "{name}");
            let view = state.view();
            assert!(view.at_defaults && view.accel_index == 0, "{name}");
            assert_eq!(view.status, "Nothing saved yet. Showing the defaults.", "{name}");

            let live = Pointer { sensitivity: -0.2, accel: Accel::Adaptive, natural_scroll: true, scroll_factor: 1.5 };
            state.adopt_live(Ok(live));
            let view = state.view();
            assert_eq!((view.sensitivity, view.accel_index, view.natural_scroll, view.scroll_factor), (-0.2, 2, true, 1.5), "{name}");
            assert_eq!(view.status, "Nothing saved yet. Showing what Hyprland has now.", "{name}");
            assert_eq!(saved(&path), file.unwrap_or(""), "{name}: looking writes nothing");
        }
    }

    #[test]
    fn a_hyprland_that_does_not_answer_leaves_the_defaults() {
        let (mut state, _) = state("ps-silent", None);
        state.adopt_live(Err("couldn't run hyprctl: not found.".into()));
        let view = state.view();
        assert!(view.at_defaults);
        assert!(view.status.contains("couldn't run hyprctl: not found)"), "{}", view.status);
        assert!(!view.status_is_error, "it isn't a problem with the page");
    }

    #[test]
    fn a_late_answer_does_not_undo_what_was_chosen_meanwhile() {
        let (mut late, _) = state("ps-late", None);
        assert!(late.set_scroll_factor(2.0));
        assert!(!late.needs_live_read());
        late.adopt_live(Ok(Pointer { scroll_factor: 3.0, ..Pointer::default() }));
        assert_eq!(late.view().scroll_factor, 2.0);

        let (mut from_file, _) = state("ps-file-wins", Some(FILE));
        from_file.adopt_live(Ok(Pointer::default()));
        assert_eq!(from_file.view().sensitivity, 0.4, "the file's values count over Hyprland's");
    }

    #[test]
    fn a_change_is_written_to_the_file_at_once_and_the_rest_of_the_file_is_kept() {
        let (mut state, path) = state("ps-write", Some(FILE));
        assert!(state.set_sensitivity(-0.25));
        let file = saved(&path);
        assert!(file.contains("sensitivity = -0.25"), "{file}");
        assert!(file.contains("# mine") && file.contains("gaps_in = 3") && file.contains("follow_mouse = 1"), "{file}");
        assert!(file.contains("accel_profile = flat") && file.contains("natural_scroll = true") && file.contains("scroll_factor = 2"), "{file}");
        assert_eq!(state.view().status, "Applying…");
        assert!(!path.with_file_name("backups").exists(), "a file Settings owns keeps no backups");
    }

    #[test]
    fn a_first_change_creates_the_file_with_all_four_options() {
        let (mut state, path) = state("ps-create", None);
        assert!(state.set_natural_scroll(true));
        let file = saved(&path);
        assert!(file.contains("input {") && file.contains("natural_scroll = true"), "{file}");
        assert!(file.contains("sensitivity = 0") && file.contains("scroll_factor = 1"), "{file}");
        assert!(!file.contains("accel_profile"), "the default profile is left to libinput: {file}");
    }

    #[test]
    fn moving_a_slider_within_one_step_changes_nothing() {
        let (mut state, path) = state("ps-same", Some(FILE));
        let before = saved(&path);
        assert!(!state.set_sensitivity(0.4));
        assert!(!state.set_sensitivity(0.4 + 1e-9), "float noise from the slider");
        assert!(!state.set_scroll_factor(2.0));
        assert!(!state.set_natural_scroll(true));
        assert!(!state.set_accel(1));
        assert_eq!(saved(&path), before);
        assert_eq!(state.view().status, "Showing what pointer.conf holds.", "and nothing counts as touched");
    }

    #[test]
    fn values_are_held_to_what_hyprland_accepts() {
        let (mut state, _) = state("ps-range", None);
        assert!(state.set_sensitivity(7.0));
        assert_eq!(state.view().sensitivity, 1.0);
        assert!(state.set_sensitivity(-7.0));
        assert_eq!(state.view().sensitivity, -1.0);
        assert!(state.set_scroll_factor(99.0));
        assert_eq!(state.view().scroll_factor, 10.0);
        assert!(state.set_scroll_factor(f64::NAN));
        assert_eq!(state.view().scroll_factor, 1.0, "not a number: the default");
        assert!(!state.set_accel(9), "no such entry in the dropdown");
    }

    #[test]
    fn a_profile_the_file_has_that_the_page_does_not_offer_is_kept_until_another_is_chosen() {
        let (mut state, path) = state("ps-custom", Some("input {\n    accel_profile = custom 0.2 1 2 3\n}\n"));
        let view = state.view();
        assert_eq!(view.accel_names.len(), 4);
        assert_eq!((view.accel_names[3].as_str(), view.accel_index), ("Custom (set in the file)", 3));
        assert!(state.set_natural_scroll(true));
        assert!(saved(&path).contains("accel_profile = custom 0.2 1 2 3"), "another change keeps it");

        assert!(state.set_accel(2));
        assert!(saved(&path).contains("accel_profile = adaptive"));
        assert_eq!(state.view().accel_names.len(), 3, "and once replaced it is gone from the list");
    }

    #[test]
    fn reset_puts_every_default_back() {
        let (mut state, path) = state("ps-reset", Some(FILE));
        assert!(state.reset());
        let view = state.view();
        assert!(view.at_defaults);
        assert_eq!((view.sensitivity, view.accel_index, view.natural_scroll, view.scroll_factor), (0.0, 0, false, 1.0));
        let file = saved(&path);
        assert!(file.contains("sensitivity = 0") && file.contains("natural_scroll = false") && file.contains("scroll_factor = 1"), "{file}");
        assert!(!file.contains("accel_profile"), "{file}");
        assert!(file.contains("follow_mouse = 1"), "{file}");
        assert!(!state.reset(), "already there");
    }

    #[test]
    fn what_was_written_is_what_the_next_start_shows() {
        let (mut state, path) = state("ps-roundtrip", None);
        assert!(state.set_sensitivity(0.35));
        assert!(state.set_accel(2));
        assert!(state.set_natural_scroll(true));
        assert!(state.set_scroll_factor(1.75));
        let again = PointerState::load(path);
        assert_eq!(again.view().sensitivity, 0.35);
        assert_eq!(again.view().accel_index, 2);
        assert!(again.view().natural_scroll);
        assert_eq!(again.view().scroll_factor, 1.75);
        assert!(!again.needs_live_read());
    }

    // ── applying ─────────────────────────────────────────────────────────────

    #[test]
    fn an_applied_change_reports_saved_and_applied() {
        let (mut state, _) = state("ps-applied", None);
        state.set_sensitivity(0.3);
        let snapshot = state.begin_apply().expect("nothing is running");
        assert_eq!(snapshot.sensitivity, 0.3);
        assert_eq!(state.finish_apply(Ok(())), None);
        assert_eq!(
            (state.view().status.as_str(), state.view().status_is_error),
            ("Applied and saved to pointer.conf.", false)
        );
    }

    #[test]
    fn applies_run_one_at_a_time_and_the_changes_made_meanwhile_make_one_more() {
        let (mut state, _) = state("ps-queue", None);
        state.set_sensitivity(0.1);
        assert_eq!(state.begin_apply().map(|p| p.sensitivity), Some(0.1));

        state.set_sensitivity(0.2);
        assert_eq!(state.begin_apply(), None, "the first is still running");
        state.set_sensitivity(0.3);
        assert_eq!(state.begin_apply(), None);

        let next = state.finish_apply(Ok(())).expect("the changes made meanwhile are applied next");
        assert_eq!(next.sensitivity, 0.3, "with the newest values only");
        assert_eq!(state.view().status, "Applying…", "the first result doesn't cover them");

        assert_eq!(state.finish_apply(Ok(())), None);
        assert_eq!(state.view().status, "Applied and saved to pointer.conf.");
        assert_eq!(state.begin_apply().map(|p| p.sensitivity), Some(0.3), "and the next change can start again");
    }

    #[test]
    fn a_result_that_ends_after_a_newer_change_is_not_shown_as_the_latest() {
        let (mut state, _) = state("ps-stale", None);
        state.set_sensitivity(0.1);
        state.begin_apply().expect("start");
        state.set_sensitivity(0.2);
        assert_eq!(state.finish_apply(Ok(())), None, "the delay for the newer change is still running");
        assert_eq!(state.view().status, "Applying…");

        assert_eq!(state.begin_apply().map(|p| p.sensitivity), Some(0.2));
        assert_eq!(state.finish_apply(Ok(())), None);
        assert_eq!(state.view().status, "Applied and saved to pointer.conf.");
    }

    #[test]
    fn an_apply_that_fails_says_so_and_the_file_keeps_the_choice() {
        let (mut state, path) = state("ps-apply-fails", None);
        state.set_scroll_factor(2.5);
        state.begin_apply().expect("start");
        state.finish_apply(Err("hyprctl said: not ok\n".into()));
        let view = state.view();
        assert_eq!(view.status, "Saved to pointer.conf, but couldn't apply it now: hyprctl said: not ok.");
        assert!(view.status_is_error);
        assert!(saved(&path).contains("scroll_factor = 2.5"));
    }

    #[test]
    fn a_file_that_cannot_be_written_still_applies_and_says_so() {
        let dir = temp_dir("ps-save-fails");
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        let mut state = PointerState::load(dir.join("blocker/pointer.conf"));
        assert!(state.set_sensitivity(0.5));
        let view = state.view();
        assert!(view.status_is_error && view.status.starts_with("Not saved ("), "{}", view.status);
        assert_eq!(view.sensitivity, 0.5, "the slider keeps what was chosen");

        state.begin_apply().expect("start");
        state.finish_apply(Ok(()));
        let view = state.view();
        assert!(view.status.starts_with("Applied now, but not saved: couldn't"), "{}", view.status);
        assert!(view.status_is_error);

        assert!(state.set_sensitivity(0.6));
        state.begin_apply().expect("start");
        state.finish_apply(Err("no compositor".into()));
        let view = state.view();
        assert!(view.status.starts_with("Not saved (couldn't") && view.status.ends_with("and not applied (no compositor)."), "{}", view.status);
    }

    #[test]
    fn a_file_that_cannot_be_read_says_so_and_still_works_from_the_defaults() {
        let dir = temp_dir("ps-unreadable");
        std::fs::write(dir.join("blocker"), "a file, not a folder").unwrap();
        let state = PointerState::load(dir.join("blocker/pointer.conf"));
        let view = state.view();
        assert!(view.status_is_error && view.status.contains("couldn't read"), "{}", view.status);
        assert!(view.at_defaults);
        assert!(state.needs_live_read());
    }

    #[test]
    fn the_applied_values_reach_hyprland_through_the_runner() {
        let (mut state, _) = state("ps-runner", None);
        state.set_sensitivity(0.3);
        state.set_accel(1);
        state.set_natural_scroll(true);
        state.set_scroll_factor(1.5);
        let runner = compositor();

        let snapshot = state.begin_apply().expect("start");
        let result = pointer::apply(&runner, &snapshot);
        assert_eq!(state.finish_apply(result), None);
        assert_eq!(
            runner.calls(),
            [
                "hyprctl systeminfo",
                "hyprctl eval hl.config({ input = { sensitivity = 0.3, accel_profile = \"flat\", natural_scroll = true, scroll_factor = 1.5 } })"
            ]
        );
        assert_eq!(state.view().status, "Applied and saved to pointer.conf.");
    }

    #[test]
    fn the_live_coalescer_hands_out_one_apply_at_a_time() {
        let mut live = Live::default();
        assert!(live.request());
        assert!(!live.request());
        assert!(!live.request());
        assert!(live.finished(), "waiting changes need one more");
        assert!(!live.finished(), "which is the last");
        assert!(live.request(), "and then it is free again");
    }
}
