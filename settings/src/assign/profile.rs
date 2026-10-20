//! Profiles: one for every app plus a global one, each holding bindings.
//!
//! A config document keeps its profiles in the control-surface layout:
//!
//! ```json
//! { "profiles": [
//!     { "name": "Firefox", "match": { "class": "^firefox$" }, "bindings": { "btn-back": "alt+left" } },
//!     { "name": "global", "bindings": {} } ] }
//! ```
//!
//! App profiles come first (the first match wins) and the global profile,
//! which has no `match`, comes last. `Profiles` carries the editing rules for
//! that list; a tab implements it for its own document type and gets the
//! rules (unique names, no duplicate app, the global profile can't be removed
//! or re-matched) for free.

use super::json::Json;

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileInfo {
    pub name: String,
    /// `match.class` regex; empty for the global (match-less) profile.
    pub class: String,
    /// `match.title` regex; empty for any title.
    pub title: String,
    /// `"fallthrough"` (default true): unset controls use the Global profile.
    pub fallthrough: bool,
    pub global: bool,
    // The rest is Keypad's (a Kdenlive API profile, its key fallback, held
    // layers); other tabs leave them off.
    pub kdenlive: bool,
    pub key_fallback: bool,
    pub layers: usize,
}

impl ProfileInfo {
    pub fn title(&self) -> String {
        if self.global {
            format!("{} (all other apps)", self.name)
        } else if self.kdenlive {
            format!("{} (Kdenlive API)", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// `^Class$` for a window class typed or picked by the user.
pub fn class_regex(class: &str) -> String {
    let escaped: String = class
        .trim()
        .chars()
        .map(|c| if "\\^$.|?*+()[]{}".contains(c) { format!("\\{c}") } else { c.to_string() })
        .collect();
    format!("^{escaped}$")
}

/// Whether a simple class pattern (`^literal` or `^literal$`) matches `class`.
/// Patterns with other regex syntax are not interpreted (returns false).
pub fn pattern_covers(pattern: &str, class: &str) -> bool {
    let Some(body) = pattern.strip_prefix('^') else {
        return false;
    };
    let (body, exact) = match body.strip_suffix('$') {
        Some(b) => (b, true),
        None => (body, false),
    };
    let mut literal = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(e) if !e.is_alphanumeric() => literal.push(e),
                _ => return false,
            },
            c if "^$.|?*+()[]{}".contains(c) => return false,
            c => literal.push(c),
        }
    }
    !literal.is_empty() && if exact { class == literal } else { class.starts_with(&literal) }
}

/// A literal window class that a simple `match.class` pattern matches, for
/// previews (`^org\.kde\.kdenlive` -> `org.kde.kdenlive`, `^(a|b)$` -> `a`).
pub fn example_class(pattern: &str) -> String {
    let body = pattern.trim_start_matches('^').trim_end_matches('$');
    let body = body
        .strip_prefix('(')
        .and_then(|b| b.strip_suffix(')'))
        .map_or(body, |b| b.split('|').next().unwrap_or(b));
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(e) = chars.next() {
                    out.push(e);
                }
            }
            '.' | '*' | '+' | '?' | '[' | ']' | '{' | '}' | '(' | ')' | '|' | '^' | '$' => {}
            c => out.push(c),
        }
    }
    out
}

/// A config document with a profile list. Implement `doc` and `doc_mut`; the
/// rest is provided.
pub trait Profiles {
    fn doc(&self) -> &Json;
    fn doc_mut(&mut self) -> &mut Json;

    fn profile_list(&self) -> &[Json] {
        self.doc().get("profiles").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    fn profile_list_mut(&mut self) -> &mut Vec<Json> {
        if self.doc().get("profiles").and_then(Json::as_array).is_none() {
            self.doc_mut().set("profiles", Json::Arr(Vec::new()));
        }
        self.doc_mut().get_mut("profiles").and_then(Json::as_array_mut).expect("profiles list")
    }

    fn profiles(&self) -> Vec<ProfileInfo> {
        self.profile_list()
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let class = p.get("match").and_then(|m| m.get("class")).and_then(Json::as_str).unwrap_or("");
                let has_match = p.get("match").is_some_and(|m| !m.entries().is_empty());
                ProfileInfo {
                    name: p.get("name").and_then(Json::as_str).map(String::from).unwrap_or(format!("profile {}", i + 1)),
                    class: class.to_string(),
                    title: p.get("match").and_then(|m| m.get("title")).and_then(Json::as_str).unwrap_or("").to_string(),
                    fallthrough: p.get("fallthrough").and_then(Json::as_bool).unwrap_or(true),
                    global: !has_match,
                    kdenlive: p.get("kdenlive").and_then(Json::as_bool).unwrap_or(false),
                    key_fallback: p.get("keyFallback").and_then(Json::as_bool).unwrap_or(false),
                    layers: p.get("layers").and_then(Json::as_array).map_or(0, Vec::len),
                }
            })
            .collect()
    }

    /// Makes sure a global profile exists (last, so app profiles match first).
    fn ensure_global(&mut self) -> usize {
        if let Some(i) = self.profiles().iter().position(|p| p.global) {
            return i;
        }
        let list = self.profile_list_mut();
        list.push(Json::Obj(vec![("name".into(), Json::str("global")), ("bindings".into(), Json::obj())]));
        list.len() - 1
    }

    fn profile_mut(&mut self, index: usize) -> Result<&mut Json, String> {
        self.profile_list_mut().get_mut(index).ok_or_else(|| "no such profile".to_string())
    }

    /// The `"bindings"` of a profile (`layer: None`) or of one of its layers.
    fn bindings_of(&self, profile: usize, layer: Option<usize>) -> Option<&Json> {
        let p = self.profile_list().get(profile)?;
        match layer {
            None => p.get("bindings"),
            Some(i) => p.get("layers")?.as_array()?.get(i)?.get("bindings"),
        }
    }

    fn bindings_mut(&mut self, profile: usize, layer: Option<usize>) -> Result<&mut Json, String> {
        let p = self.profile_mut(profile)?;
        match layer {
            None => Ok(p.object_mut("bindings")),
            Some(i) => match p.get_mut("layers") {
                Some(Json::Arr(layers)) => layers.get_mut(i).map(|l| l.object_mut("bindings")).ok_or_else(|| "no such layer".to_string()),
                _ => Err("no such layer".into()),
            },
        }
    }

    /// Renames a profile (names show in the profile list and the cheatsheet).
    fn set_profile_name(&mut self, index: usize, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a profile needs a name".into());
        }
        if self.profiles().iter().enumerate().any(|(i, p)| i != index && p.name == name) {
            return Err(format!("there's already a profile named {name}"));
        }
        self.profile_mut(index)?.set("name", Json::str(name));
        Ok(())
    }

    /// `match.title`: the profile applies only while the window title matches.
    fn set_profile_title(&mut self, index: usize, regex: &str) -> Result<(), String> {
        if self.profiles().get(index).is_some_and(|p| p.global) {
            return Err("the Global profile applies to every window".into());
        }
        let regex = regex.trim();
        let profile = self.profile_mut(index)?;
        if regex.is_empty() {
            if let Some(m) = profile.get_mut("match") {
                m.remove("title");
            }
        } else {
            profile.object_mut("match").set("title", Json::str(regex));
        }
        Ok(())
    }

    /// `"fallthrough"`: whether controls this profile leaves unset use Global
    /// (the default; stored only when off).
    fn set_profile_fallthrough(&mut self, index: usize, on: bool) -> Result<(), String> {
        let profile = self.profile_mut(index)?;
        if on {
            profile.remove("fallthrough");
        } else {
            profile.set("fallthrough", Json::Bool(false));
        }
        Ok(())
    }

    /// Adds an app profile before the global one; returns its index.
    fn add_app_profile(&mut self, name: &str, class: &str) -> Result<usize, String> {
        let name = name.trim();
        if name.is_empty() || class.trim().is_empty() {
            return Err("choose an app (window class)".into());
        }
        let regex = class_regex(class);
        if let Some(p) = self.profiles().iter().find(|p| p.class == regex || pattern_covers(&p.class, class.trim())) {
            return Err(format!("the {} profile already matches {}", p.name, class.trim()));
        }
        let global = self.ensure_global();
        let profile = Json::Obj(vec![
            ("name".into(), Json::str(name)),
            ("match".into(), Json::Obj(vec![("class".into(), Json::Str(regex))])),
            ("bindings".into(), Json::obj()),
        ]);
        self.profile_list_mut().insert(global, profile);
        Ok(global)
    }

    fn remove_profile(&mut self, index: usize) -> Result<(), String> {
        let info = self.profiles().get(index).cloned().ok_or("no such profile")?;
        if info.global {
            return Err("the Global profile can't be removed".into());
        }
        self.profile_list_mut().remove(index);
        Ok(())
    }

    fn set_profile_class(&mut self, index: usize, regex: &str) -> Result<(), String> {
        if self.profiles().get(index).is_some_and(|p| p.global) {
            return Err("the Global profile applies to every app".into());
        }
        if regex.trim().is_empty() {
            return Err("the window class pattern can't be empty".into());
        }
        self.profile_mut(index)?.object_mut("match").set("class", Json::str(regex.trim()));
        Ok(())
    }

    fn set_profile_flag(&mut self, index: usize, key: &str, on: bool) -> Result<(), String> {
        let profile = self.profile_mut(index)?;
        if on {
            profile.set(key, Json::Bool(true));
        } else {
            profile.remove(key);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assign::testing::{doc_from, global_only};

    const TWO_APPS: &str = r#"{"profiles":[
        {"name":"Browser","match":{"class":"^firefox$","title":"Inbox"},"fallthrough":false,"bindings":{"btn-back":"alt+left"}},
        {"match":{"class":"^steam_app_"},"bindings":{}},
        {"name":"global","bindings":{"btn-fwd":"alt+right"}}]}"#;

    #[test]
    fn profiles_read_their_match_and_flags() {
        let profiles = doc_from(TWO_APPS).profiles();
        assert_eq!(profiles.len(), 3);

        let browser = &profiles[0];
        assert_eq!((browser.name.as_str(), browser.class.as_str(), browser.title.as_str()), ("Browser", "^firefox$", "Inbox"));
        assert!(!browser.global && !browser.fallthrough);
        assert_eq!(browser.title(), "Browser");

        assert_eq!(profiles[1].name, "profile 2", "a profile without a name is numbered");
        assert!(profiles[1].fallthrough, "falling through to Global is the default");

        assert!(profiles[2].global);
        assert_eq!(profiles[2].title(), "global (all other apps)");
        assert!(!profiles[2].kdenlive && !profiles[2].key_fallback && profiles[2].layers == 0);
    }

    #[test]
    fn a_profile_with_an_empty_match_is_the_global_one() {
        let d = doc_from(r#"{"profiles":[{"name":"all","match":{},"bindings":{}}]}"#);
        assert!(d.profiles()[0].global);
        assert!(doc_from("{}").profiles().is_empty());
    }

    #[test]
    fn the_global_profile_is_created_once_and_last() {
        let mut d = doc_from(r#"{"profiles":[{"name":"Game","match":{"class":"^game$"},"bindings":{}}]}"#);
        assert_eq!(d.ensure_global(), 1);
        assert_eq!(d.ensure_global(), 1, "asking again finds it");
        assert_eq!(d.profiles().len(), 2);

        let mut empty = doc_from("{}");
        assert_eq!(empty.ensure_global(), 0);
        assert!(empty.profiles()[0].global);
    }

    #[test]
    fn app_profiles_are_added_before_global_and_never_twice() {
        let mut d = global_only();
        assert_eq!(d.add_app_profile("Browser", "org.mozilla.firefox"), Ok(0));
        assert_eq!(d.add_app_profile("Editor", "org.kde.kate"), Ok(1));
        let names: Vec<String> = d.profiles().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Browser", "Editor", "global"]);

        let again = d.add_app_profile("Other", "org.mozilla.firefox").unwrap_err();
        assert_eq!(again, "the Browser profile already matches org.mozilla.firefox");
        assert!(d.add_app_profile("", "a.b").is_err());
        assert!(d.add_app_profile("Name", "   ").is_err());
    }

    #[test]
    fn a_broader_pattern_blocks_a_second_profile_for_an_app_it_covers() {
        let mut d = doc_from(TWO_APPS);
        let err = d.add_app_profile("Some game", "steam_app_730").unwrap_err();
        assert!(err.contains("already matches steam_app_730"), "{err}");
    }

    #[test]
    fn names_stay_unique_but_a_profile_may_keep_its_own() {
        let mut d = doc_from(TWO_APPS);
        assert_eq!(d.set_profile_name(0, "Browser"), Ok(()));
        assert_eq!(d.set_profile_name(0, "  Web  "), Ok(()));
        assert_eq!(d.profiles()[0].name, "Web");
        assert_eq!(d.set_profile_name(0, "global"), Err("there's already a profile named global".into()));
        assert_eq!(d.set_profile_name(0, "   "), Err("a profile needs a name".into()));
        assert_eq!(d.set_profile_name(7, "Nobody"), Err("no such profile".into()));
    }

    #[test]
    fn the_global_profile_cannot_be_removed_or_given_a_match() {
        let mut d = doc_from(TWO_APPS);
        assert_eq!(d.remove_profile(2), Err("the Global profile can't be removed".into()));
        assert_eq!(d.set_profile_class(2, "^x$"), Err("the Global profile applies to every app".into()));
        assert_eq!(d.set_profile_title(2, "x"), Err("the Global profile applies to every window".into()));
        assert_eq!(d.remove_profile(9), Err("no such profile".into()));
        assert_eq!(d.remove_profile(1), Ok(()));
        assert_eq!(d.profiles().len(), 2);
    }

    #[test]
    fn class_and_title_edits_touch_only_their_own_match_field() {
        let mut d = doc_from(TWO_APPS);
        assert_eq!(d.set_profile_class(0, " ^chromium$ "), Ok(()));
        assert_eq!(d.profiles()[0].class, "^chromium$");
        assert_eq!(d.profiles()[0].title, "Inbox");
        assert_eq!(d.set_profile_class(0, "  "), Err("the window class pattern can't be empty".into()));

        assert_eq!(d.set_profile_title(0, ""), Ok(()));
        assert_eq!(d.profiles()[0].title, "");
        assert_eq!(d.profiles()[0].class, "^chromium$", "clearing the title keeps the class");
        assert_eq!(d.set_profile_title(1, "Match me"), Ok(()));
        assert_eq!(d.profiles()[1].title, "Match me");
    }

    #[test]
    fn fallthrough_and_flags_are_stored_only_when_they_differ_from_the_default() {
        let mut d = doc_from(TWO_APPS);
        assert_eq!(d.set_profile_fallthrough(0, true), Ok(()));
        assert!(d.profile_list()[0].get("fallthrough").is_none());
        assert_eq!(d.set_profile_fallthrough(0, false), Ok(()));
        assert_eq!(d.profile_list()[0].get("fallthrough"), Some(&Json::Bool(false)));

        assert_eq!(d.set_profile_flag(1, "keyFallback", true), Ok(()));
        assert!(d.profiles()[1].key_fallback);
        assert_eq!(d.set_profile_flag(1, "keyFallback", false), Ok(()));
        assert!(d.profile_list()[1].get("keyFallback").is_none());
        assert_eq!(d.set_profile_fallthrough(9, true), Err("no such profile".into()));
    }

    #[test]
    fn bindings_are_found_by_profile_and_layer() {
        let mut d = doc_from(
            r#"{"profiles":[{"name":"global","bindings":{"a":"x"},"layers":[{"name":"hold","bindings":{"b":"y"}}]}]}"#,
        );
        assert_eq!(d.bindings_of(0, None).and_then(|b| b.get("a")), Some(&Json::str("x")));
        assert_eq!(d.bindings_of(0, Some(0)).and_then(|b| b.get("b")), Some(&Json::str("y")));
        assert!(d.bindings_of(0, Some(1)).is_none());
        assert!(d.bindings_of(3, None).is_none());

        d.bindings_mut(0, None).unwrap().set("c", Json::str("z"));
        d.bindings_mut(0, Some(0)).unwrap().set("d", Json::str("w"));
        assert_eq!(d.bindings_of(0, None).and_then(|b| b.get("c")), Some(&Json::str("z")));
        assert_eq!(d.bindings_of(0, Some(0)).and_then(|b| b.get("d")), Some(&Json::str("w")));
        assert_eq!(d.bindings_mut(0, Some(4)).err(), Some("no such layer".to_string()));
        assert_eq!(d.bindings_mut(5, None).err(), Some("no such profile".to_string()));
    }

    #[test]
    fn a_profile_without_bindings_gets_them_when_first_edited() {
        let mut d = doc_from(r#"{"profiles":[{"name":"global"}]}"#);
        assert!(d.bindings_of(0, None).is_none());
        d.bindings_mut(0, None).unwrap().set("grid-1", Json::str("1"));
        assert!(d.bindings_of(0, None).is_some());
    }

    #[test]
    fn a_window_class_becomes_an_anchored_escaped_pattern() {
        assert_eq!(class_regex("org.mozilla.firefox"), r"^org\.mozilla\.firefox$");
        assert_eq!(class_regex("  steam_app_730 "), "^steam_app_730$");
        assert_eq!(class_regex("a+b"), r"^a\+b$");
    }

    #[test]
    fn simple_patterns_are_matched_and_regex_syntax_is_not_guessed() {
        assert!(pattern_covers(r"^org\.mozilla\.firefox$", "org.mozilla.firefox"));
        assert!(!pattern_covers(r"^org\.mozilla\.firefox$", "org.mozilla.firefox-esr"));
        assert!(pattern_covers("^steam_app_", "steam_app_730"));
        assert!(!pattern_covers("^steam_app_", "other"));
        assert!(!pattern_covers("firefox", "firefox"), "unanchored patterns are not interpreted");
        assert!(!pattern_covers("^(a|b)$", "a"));
        assert!(!pattern_covers("^$", ""));
    }

    #[test]
    fn a_pattern_previews_as_a_literal_window_class() {
        assert_eq!(example_class(r"^org\.kde\.kdenlive$"), "org.kde.kdenlive");
        assert_eq!(example_class("^(kate|kwrite)$"), "kate");
        assert_eq!(example_class("^steam_app_"), "steam_app_");
    }
}
