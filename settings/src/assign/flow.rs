//! The profile controls every assignment tab offers: add an app, remove it,
//! edit its match, choose whether unset controls fall through to Global.
//!
//! Each function applies one edit to a `Profiles` document and says what to
//! tell the user; the tab applies its own side effects (marking the page
//! unsaved, reloading its editor, moving the selection).

use super::profile::Profiles;
use super::Saving;

/// What a successful edit asks of the tab.
#[derive(Debug, PartialEq, Eq)]
pub struct Done {
    /// The profile to show next, when the edit moved the selection.
    pub select: Option<usize>,
    pub message: String,
}

/// A profile named after the last part of its window class
/// (`org.kde.kdenlive` -> `kdenlive`).
pub fn default_profile_name(class: &str) -> String {
    class.rsplit('.').next().unwrap_or(class).to_string()
}

pub fn add_app(config: &mut impl Profiles, class: &str, saving: Saving) -> Result<Done, String> {
    let index = config.add_app_profile(&default_profile_name(class), class)?;
    let next = match saving {
        Saving::Manual => "Map its controls, then Save.",
        Saving::Immediate => "Pick a button to assign it.",
    };
    Ok(Done { select: Some(index), message: format!("Added a profile for {class}. {next}") })
}

pub fn remove(config: &mut impl Profiles, index: usize, saving: Saving) -> Result<Done, String> {
    let name = config.profiles().get(index).map(|p| p.name.clone()).unwrap_or_default();
    config.remove_profile(index)?;
    Ok(Done { select: Some(index.saturating_sub(1)), message: format!("Removed the {name} profile. {}", saving.tail()) })
}

/// Applies the name, window class and window title fields; only what differs
/// from the profile is written. `None` when nothing changed.
pub fn apply_details(
    config: &mut impl Profiles,
    index: usize,
    name: &str,
    class: &str,
    title: &str,
    saving: Saving,
) -> Result<Option<Done>, String> {
    let Some(before) = config.profiles().get(index).cloned() else {
        return Ok(None);
    };
    if name.trim() != before.name {
        config.set_profile_name(index, name)?;
    }
    if !before.global && class.trim() != before.class {
        config.set_profile_class(index, class)?;
    }
    if !before.global && title.trim() != before.title {
        config.set_profile_title(index, title)?;
    }
    if config.profiles().get(index) == Some(&before) {
        return Ok(None);
    }
    Ok(Some(Done { select: None, message: format!("Profile updated. {}", saving.tail()) }))
}

pub fn set_fallthrough(config: &mut impl Profiles, index: usize, on: bool, saving: Saving) -> Option<Done> {
    config.set_profile_fallthrough(index, on).ok()?;
    let effect = if on {
        "Controls this profile leaves unset now do what Global maps them to."
    } else {
        "Controls this profile leaves unset now do nothing in this app."
    };
    Some(Done { select: None, message: format!("{effect} {}", saving.tail()) })
}

/// The window classes of the open windows, for the "Add app" picker.
pub fn parse_window_classes(clients_json: &[u8]) -> Vec<String> {
    let Ok(serde_json::Value::Array(clients)) = serde_json::from_slice(clients_json) else {
        return Vec::new();
    };
    let mut classes: Vec<String> = clients
        .iter()
        .filter_map(|c| c.get("class").and_then(|v| v.as_str()))
        .filter(|c| !c.is_empty() && *c != "settings")
        .map(String::from)
        .collect();
    classes.sort();
    classes.dedup();
    classes
}

/// Asks the compositor which windows are open (a read-only query).
pub fn running_app_classes() -> Vec<String> {
    let Ok(out) = std::process::Command::new("hyprctl").args(["clients", "-j"]).output() else {
        return Vec::new();
    };
    parse_window_classes(&out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assign::testing::{doc_from, global_only as doc, Doc};

    const SAVE: Saving = Saving::Manual;

    #[test]
    fn adding_an_app_names_it_after_its_class_and_selects_it() {
        let mut d = doc();
        let done = add_app(&mut d, "org.mozilla.firefox", SAVE).unwrap();
        assert_eq!(done.select, Some(0), "app profiles go before the global one");
        assert_eq!(done.message, "Added a profile for org.mozilla.firefox. Map its controls, then Save.");
        assert_eq!(d.profiles()[0].name, "firefox");
        assert_eq!(d.profiles()[0].class, r"^org\.mozilla\.firefox$");
        assert!(d.profiles()[1].global);
        assert!(add_app(&mut d, "org.mozilla.firefox", SAVE).unwrap_err().contains("already matches"));
        assert!(add_app(&mut d, "  ", SAVE).is_err());
    }

    #[test]
    fn removing_selects_the_previous_profile_and_never_removes_global() {
        let mut d = doc();
        add_app(&mut d, "a.one", SAVE).unwrap();
        add_app(&mut d, "b.two", SAVE).unwrap();
        let done = remove(&mut d, 1, SAVE).unwrap();
        assert_eq!(done.message, "Removed the two profile. Not saved yet.");
        assert_eq!(done.select, Some(0));
        let global = d.profiles().iter().position(|p| p.global).unwrap();
        assert_eq!(remove(&mut d, global, SAVE).unwrap_err(), "the Global profile can't be removed");
    }

    #[test]
    fn details_write_only_what_changed() {
        let mut d = doc();
        add_app(&mut d, "org.example.app", SAVE).unwrap();
        let class = d.profiles()[0].class.clone();

        assert_eq!(apply_details(&mut d, 0, "app", &class, "", SAVE).unwrap(), None);
        let done = apply_details(&mut d, 0, "Example", &class, "Inbox", SAVE).unwrap().expect("changed");
        assert_eq!(done.message, "Profile updated. Not saved yet.");
        assert_eq!((d.profiles()[0].name.as_str(), d.profiles()[0].title.as_str()), ("Example", "Inbox"));

        // The global profile keeps its (absent) match whatever the fields say.
        let global = d.profiles().iter().position(|p| p.global).unwrap();
        assert_eq!(apply_details(&mut d, global, "global", "x", "y", SAVE).unwrap(), None);
        assert!(d.profiles()[global].global);

        assert!(apply_details(&mut d, 0, "global", &class, "", SAVE).unwrap_err().contains("already a profile named"));
        assert_eq!(apply_details(&mut d, 9, "n", "c", "t", SAVE).unwrap(), None);
    }

    #[test]
    fn fallthrough_is_stored_only_when_off() {
        let mut d = doc();
        add_app(&mut d, "a.one", SAVE).unwrap();
        let off = set_fallthrough(&mut d, 0, false, SAVE).unwrap();
        assert!(off.message.contains("do nothing in this app"));
        assert!(!d.profiles()[0].fallthrough);
        let on = set_fallthrough(&mut d, 0, true, SAVE).unwrap();
        assert!(on.message.contains("do what Global maps them to"));
        assert!(d.profiles()[0].fallthrough);
        assert!(d.profile_list()[0].get("fallthrough").is_none());
        assert_eq!(set_fallthrough(&mut d, 9, true, SAVE), None);
    }

    #[test]
    fn a_tab_that_saves_at_once_is_told_so_instead_of_to_press_save() {
        let now = Saving::Immediate;
        let mut d = doc();
        let added = add_app(&mut d, "org.example.app", now).unwrap();
        assert_eq!(added.message, "Added a profile for org.example.app. Pick a button to assign it.");
        let class = d.profiles()[0].class.clone();
        let updated = apply_details(&mut d, 0, "Example", &class, "", now).unwrap().expect("changed");
        assert_eq!(updated.message, "Profile updated. Saved.");
        let off = set_fallthrough(&mut d, 0, false, now).unwrap();
        assert_eq!(off.message, "Controls this profile leaves unset now do nothing in this app. Saved.");
        let removed = remove(&mut d, 0, now).unwrap();
        assert_eq!(removed.message, "Removed the Example profile. Saved.");
        for message in [added.message, updated.message, off.message, removed.message] {
            assert!(!message.contains("Not saved") && !message.contains("then Save"), "{message}");
        }
    }

    #[test]
    fn open_windows_are_listed_once_without_the_settings_app() {
        let json = br#"[{"class":"firefox"},{"class":"settings"},{"class":""},{"class":"firefox"},{"class":"foot"},{"title":"no class"}]"#;
        assert_eq!(parse_window_classes(json), vec!["firefox".to_string(), "foot".to_string()]);
        assert!(parse_window_classes(b"not json").is_empty());
        assert!(parse_window_classes(b"{}").is_empty());
    }

    #[test]
    fn any_slot_name_round_trips_through_the_shared_grammar() {
        use crate::assign::action::{classify, to_json, ActionKind, Binding};
        let mut d = doc();
        let idx = add_app(&mut d, "org.example.app", SAVE).unwrap().select.unwrap();
        let back = Binding::new(ActionKind::Shortcut, "alt+left");
        let value = to_json(&back).unwrap().expect("a shortcut is stored");
        d.bindings_mut(idx, None).unwrap().set("btn-back", value);

        let stored = |d: &Doc, slot: &str| classify(d.bindings_of(idx, None).and_then(|b| b.get(slot)));
        assert_eq!(stored(&d, "btn-back"), back);
        assert_eq!(stored(&d, "grid-7").kind, ActionKind::Inherit);
        assert!(d.bindings_mut(9, None).is_err());
    }

    #[test]
    fn an_assignment_the_editor_cannot_express_reads_as_advanced_and_is_not_saved_over() {
        use crate::assign::action::{classify, to_json, ActionKind};
        let text = r#"{"profiles":[{"name":"global","bindings":{"btn-fwd":{"command":["notify-send","hi"],"repeat":true}}}]}"#;
        let d = doc_from(text);
        let shown = classify(d.bindings_of(0, None).and_then(|b| b.get("btn-fwd")));
        assert_eq!(shown.kind, ActionKind::Advanced);
        assert!(shown.value.contains(r#""repeat": true"#), "{}", shown.value);
        assert!(to_json(&shown).is_err(), "the editor can't write it back");
    }
}
