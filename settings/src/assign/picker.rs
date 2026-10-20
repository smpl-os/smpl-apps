//! The action picker, as data: what the "Action" dropdown, its value editor
//! and its hint show for a binding, and what switching the action does.
//!
//! These are pure functions over `Binding`, so every tab that assigns
//! actions (Keypad, Mouse) shares them and only supplies its own wording
//! (what "not set" means for its hardware) and its own list of offered kinds.

use super::action::{ActionKind, Binding, CHEATSHEET_MODES, KDENLIVE_ACTIONS, MEDIA_KEYS, MOUSE_BUTTONS};

/// An id and the text the dropdown shows for it.
pub type Choice = (String, String);

/// The value editor of an action kind.
pub const MODE_NONE: i32 = 0;
/// A text box (a shortcut, a command line).
pub const MODE_TEXT: i32 = 1;
/// A dropdown of fixed choices.
pub const MODE_CHOICE: i32 = 2;
/// Read-only: an assignment the editor can't express.
pub const MODE_ADVANCED: i32 = 3;

/// What "not set" means on one surface. A keypad key nobody mapped does
/// nothing; a mouse button nobody mapped keeps its normal job. (In an app
/// profile "not set" always reads as `ActionKind::Inherit`'s label: use Global.)
pub struct Wording {
    /// The dropdown entry for "not set" while editing the global profile.
    pub unset_global: &'static str,
    pub hint_unset_global: &'static str,
    pub hint_unset_app: &'static str,
    pub hint_advanced: &'static str,
}

pub fn action_name(kind: ActionKind, global: bool, wording: &Wording) -> String {
    match (kind, global) {
        (ActionKind::Inherit, true) => wording.unset_global.to_string(),
        (kind, _) => kind.label().to_string(),
    }
}

/// The value editor (`MODE_*`) and the hint under it for a kind.
pub fn describe(kind: ActionKind, global: bool, wording: &Wording) -> (i32, &'static str) {
    match kind {
        ActionKind::Inherit if global => (MODE_NONE, wording.hint_unset_global),
        ActionKind::Inherit => (MODE_NONE, wording.hint_unset_app),
        ActionKind::Disabled => (MODE_NONE, "Does nothing in this profile (also stops the Global binding)."),
        ActionKind::Shortcut => (MODE_TEXT, "e.g. ctrl+shift+s, F13, super+1; a sequence: ctrl+k x"),
        ActionKind::Command => (MODE_TEXT, "e.g. notify-send hello (runs without a shell)"),
        ActionKind::Media | ActionKind::Mouse | ActionKind::Kdenlive => (MODE_CHOICE, ""),
        ActionKind::Cheatsheet => (MODE_CHOICE, "Shows what every key and knob does in the app you're using."),
        ActionKind::Advanced => (MODE_ADVANCED, wording.hint_advanced),
    }
}

/// The values the choice dropdown lists for the editor's kind. A value set
/// in the file that the table lacks is listed too, so it isn't lost.
/// `live_actions` is a running app's own action list (Kdenlive's): it
/// replaces the built-in table when it isn't empty.
pub fn choices(editor: &Binding, live_actions: &[Choice]) -> Vec<Choice> {
    let table = |t: &[(&str, &str)]| t.iter().map(|(k, l)| (k.to_string(), l.to_string())).collect::<Vec<_>>();
    let mut list = match editor.kind {
        ActionKind::Media => table(MEDIA_KEYS),
        ActionKind::Mouse => table(MOUSE_BUTTONS),
        ActionKind::Cheatsheet => table(CHEATSHEET_MODES),
        ActionKind::Kdenlive if !live_actions.is_empty() => live_actions.to_vec(),
        ActionKind::Kdenlive => table(KDENLIVE_ACTIONS),
        _ => Vec::new(),
    };
    if !editor.value.is_empty() && !list.is_empty() && !list.iter().any(|(k, _)| *k == editor.value) {
        list.push((editor.value.clone(), editor.value.clone()));
    }
    list
}

/// The editor binding after the user picks `kind` in the dropdown: `None` when
/// it is the kind already being edited. Going back to the saved kind restores
/// the saved binding; any other kind starts empty but keeps the label, icon
/// and "only if installed" the user may have typed.
pub fn change_kind(saved: Binding, editor: &Binding, kind: ActionKind) -> Option<Binding> {
    if kind == editor.kind {
        return None;
    }
    if saved.kind == kind {
        return Some(saved);
    }
    Some(Binding::new(kind, "").labelled(&editor.label).with_icon(&editor.icon).needing(&editor.needs.join(" ")))
}

/// A fresh choice-kind binding starts on the first choice.
pub fn default_choice(editor: &mut Binding, choices: &[Choice]) {
    if let Some((first, _)) = choices.first().filter(|_| editor.value.is_empty()) {
        editor.value = first.clone();
    }
}

/// The binding to save: the editor's, with the text box (shortcut or command
/// line), the cheatsheet label and "only if installed" typed in the form.
/// Kinds that can't carry a label or icon drop them.
pub fn compose_binding(editor: &Binding, text: &str, label: &str, needs: &str) -> Binding {
    let mut binding = editor.clone();
    if matches!(binding.kind, ActionKind::Shortcut | ActionKind::Command) {
        binding.value = text.to_string();
    }
    binding.label = if binding.takes_label() { label.trim().to_string() } else { String::new() };
    if binding.takes_label() {
        binding = binding.needing(needs);
    } else {
        binding.icon.clear();
        binding.needs.clear();
    }
    binding
}

/// "<control> → <what it does>. Not saved yet." with an optional note after it.
pub fn assigned_message(control: &str, binding: &Binding, note: Option<&str>) -> String {
    let what = match binding.summary() {
        v if v.is_empty() => "not set".to_string(),
        v => v,
    };
    let note = note.map(|n| format!(" {n}.")).unwrap_or_default();
    format!("{control} → {what}. Not saved yet.{note}")
}

/// The program a command line starts, past launcher wrappers
/// ("focus-or-launch nemo nemo" and "gtk-launch nemo" -> "nemo").
pub fn launched_program(command: &str) -> Option<&str> {
    const WRAPPERS: [&str; 4] = ["focus-or-launch", "gtk-launch", "uwsm-app", "setsid"];
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.first() {
        Some(first) if WRAPPERS.contains(first) => words.last().copied().filter(|w| w != first),
        first => first.copied(),
    }
}

/// The placeholder of the "only if installed" box: a command suggests the
/// program it launches.
pub fn needs_placeholder(editor: &Binding) -> String {
    match editor.kind {
        ActionKind::Command => format!(
            "Optional, e.g. {}: skip this binding while it isn't installed",
            launched_program(&editor.value).unwrap_or("grafium")
        ),
        _ => "Optional: programs or app ids; skip this binding while one isn't installed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORDING: Wording = Wording {
        unset_global: "Not set (normal button)",
        hint_unset_global: "Unassigned: the button does its normal job.",
        hint_unset_app: "Uses the Global assignment.",
        hint_advanced: "Edit it in the file.",
    };

    #[test]
    fn unset_reads_as_the_surface_words_it() {
        assert_eq!(action_name(ActionKind::Inherit, true, &WORDING), "Not set (normal button)");
        assert_eq!(action_name(ActionKind::Inherit, false, &WORDING), "Not set (use Global)");
        assert_eq!(action_name(ActionKind::Shortcut, true, &WORDING), "Keyboard shortcut");
        assert_eq!(describe(ActionKind::Inherit, true, &WORDING), (MODE_NONE, "Unassigned: the button does its normal job."));
        assert_eq!(describe(ActionKind::Inherit, false, &WORDING), (MODE_NONE, "Uses the Global assignment."));
        assert_eq!(describe(ActionKind::Advanced, false, &WORDING), (MODE_ADVANCED, "Edit it in the file."));
    }

    #[test]
    fn each_kind_picks_its_value_editor() {
        let mode = |k| describe(k, false, &WORDING).0;
        assert_eq!(mode(ActionKind::Disabled), MODE_NONE);
        assert_eq!(mode(ActionKind::Shortcut), MODE_TEXT);
        assert_eq!(mode(ActionKind::Command), MODE_TEXT);
        assert_eq!(mode(ActionKind::Media), MODE_CHOICE);
        assert_eq!(mode(ActionKind::Mouse), MODE_CHOICE);
        assert_eq!(mode(ActionKind::Advanced), MODE_ADVANCED);
    }

    #[test]
    fn choices_list_the_table_and_keep_a_value_it_lacks() {
        let media = choices(&Binding::new(ActionKind::Media, "mute"), &[]);
        assert_eq!(media.len(), MEDIA_KEYS.len());
        assert!(media.contains(&("mute".to_string(), "Mute".to_string())));

        let odd = choices(&Binding::new(ActionKind::Mouse, "side-9"), &[]);
        assert_eq!(odd.len(), MOUSE_BUTTONS.len() + 1);
        assert_eq!(odd.last(), Some(&("side-9".to_string(), "side-9".to_string())));

        assert!(choices(&Binding::new(ActionKind::Shortcut, "ctrl+c"), &[]).is_empty());
        // An empty value adds nothing.
        assert_eq!(choices(&Binding::new(ActionKind::Media, ""), &[]).len(), MEDIA_KEYS.len());
    }

    #[test]
    fn a_running_apps_own_list_replaces_the_builtin_one() {
        let live = vec![("one".to_string(), "One".to_string())];
        let list = choices(&Binding::new(ActionKind::Kdenlive, "one"), &live);
        assert_eq!(list, live);
        assert_eq!(choices(&Binding::new(ActionKind::Kdenlive, "mark_in"), &[]).len(), KDENLIVE_ACTIONS.len());
    }

    #[test]
    fn switching_kind_restores_the_saved_binding_or_starts_clean() {
        let saved = Binding::new(ActionKind::Shortcut, "alt+left").labelled("Back");
        let editor = Binding::new(ActionKind::Media, "mute").labelled("Silence").with_icon("volume-off").needing("pavucontrol");

        assert_eq!(change_kind(saved.clone(), &editor, ActionKind::Media), None);
        assert_eq!(change_kind(saved.clone(), &editor, ActionKind::Shortcut), Some(saved.clone()));

        let fresh = change_kind(saved, &editor, ActionKind::Command).expect("a new kind");
        assert_eq!(fresh.kind, ActionKind::Command);
        assert_eq!(fresh.value, "");
        assert_eq!((fresh.label.as_str(), fresh.icon.as_str()), ("Silence", "volume-off"));
        assert_eq!(fresh.needs, vec!["pavucontrol".to_string()]);
    }

    #[test]
    fn a_new_choice_starts_on_the_first_one() {
        let mut editor = Binding::new(ActionKind::Mouse, "");
        let list = choices(&editor, &[]);
        default_choice(&mut editor, &list);
        assert_eq!(editor.value, "left");

        let mut chosen = Binding::new(ActionKind::Mouse, "back");
        let list = choices(&chosen, &[]);
        default_choice(&mut chosen, &list);
        assert_eq!(chosen.value, "back");
    }

    #[test]
    fn composing_takes_the_form_for_text_kinds_and_drops_meta_for_others() {
        let shortcut = compose_binding(&Binding::new(ActionKind::Shortcut, "old"), "ctrl+w", " Close ", "firefox");
        assert_eq!(shortcut.value, "ctrl+w");
        assert_eq!(shortcut.label, "Close");
        assert_eq!(shortcut.needs, vec!["firefox".to_string()]);

        let media = compose_binding(&Binding::new(ActionKind::Media, "mute"), "ignored", "", "");
        assert_eq!(media.value, "mute");

        let off = compose_binding(&Binding::new(ActionKind::Disabled, "").labelled("x").with_icon("a-b").needing("y"), "", "typed", "z");
        assert_eq!((off.label.as_str(), off.icon.as_str()), ("", ""));
        assert!(off.needs.is_empty());
    }

    #[test]
    fn the_applied_message_names_the_control_and_the_result() {
        let b = Binding::new(ActionKind::Shortcut, "alt+left");
        assert_eq!(assigned_message("Back", &b, None), "Back → alt+left. Not saved yet.");
        assert_eq!(assigned_message("Back", &Binding::new(ActionKind::Inherit, ""), None), "Back → not set. Not saved yet.");
        assert_eq!(assigned_message("Back", &b, Some("Two things changed")), "Back → alt+left. Not saved yet. Two things changed.");
    }

    #[test]
    fn if_installed_suggests_the_launched_program() {
        assert_eq!(launched_program("focus-or-launch nemo nemo"), Some("nemo"));
        assert_eq!(launched_program("gtk-launch grafium"), Some("grafium"));
        assert_eq!(launched_program("smplos-settings"), Some("smplos-settings"));
        assert_eq!(launched_program("gtk-launch"), None);
        assert_eq!(launched_program(""), None);
        let cmd = Binding::new(ActionKind::Command, "gtk-launch nemo");
        assert!(needs_placeholder(&cmd).contains("e.g. nemo"));
        assert!(needs_placeholder(&Binding::new(ActionKind::Shortcut, "a")).starts_with("Optional: programs"));
    }
}
