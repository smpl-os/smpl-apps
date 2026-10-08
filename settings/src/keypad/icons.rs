//! Keypad icons: "smplOS Keypad Icons", the outline Tabler Icons subset
//! (MIT) that smplOS's cheatsheet overlay draws. `ui/assets/keypad-icons.*`
//! come from smplOS's `src/shared/fonts/keypad-icons/build.py`; regenerate
//! both repositories together.

use std::sync::OnceLock;

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Icon {
    pub name: String,
    pub glyph: String,
    pub category: String,
    pub tags: Vec<String>,
}

fn parse(json: &str) -> Vec<Icon> {
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    let text = |i: &Value, k: &str| i.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    v.get("icons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let glyph = char::from_u32(u32::from_str_radix(&text(i, "codepoint"), 16).ok()?)?;
            Some(Icon {
                name: text(i, "name"),
                glyph: glyph.to_string(),
                category: text(i, "category"),
                tags: i
                    .get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.as_str().map(str::to_lowercase))
                    .collect(),
            })
        })
        .collect()
}

/// Every icon, grouped by category in the set's order.
pub fn all() -> &'static [Icon] {
    static ICONS: OnceLock<Vec<Icon>> = OnceLock::new();
    ICONS.get_or_init(|| parse(include_str!("../../ui/assets/keypad-icons.json")))
}

pub fn find(name: &str) -> Option<&'static Icon> {
    all().iter().find(|i| i.name == name)
}

/// The glyph for an icon name; empty for none or a name the set lacks.
pub fn glyph(name: &str) -> &'static str {
    find(name).map_or("", |i| i.glyph.as_str())
}

/// Icons matching every word of `query`: in the name, or at the start of a
/// tag or the category ("git" finds brand-github, not "digit"). Name matches
/// first; an empty query lists all.
pub fn search(query: &str) -> Vec<&'static Icon> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return all().iter().collect();
    }
    let hits = |i: &Icon, in_name: bool| {
        words.iter().all(|w| {
            i.name.contains(w.as_str())
                || (!in_name && (i.category.to_lowercase().starts_with(w.as_str()) || i.tags.iter().any(|t| t.starts_with(w.as_str()))))
        })
    };
    let mut out: Vec<&Icon> = all().iter().filter(|i| hits(i, true)).collect();
    out.extend(all().iter().filter(|i| !hits(i, true) && hits(i, false)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_set_has_the_overlay_and_launcher_icons() {
        assert!(all().len() > 200);
        for name in [
            "help-circle", "player-play", "player-track-next", "volume", "volume-2", "volume-3", "mouse", "arrow-up",
            "arrow-down", "world", "brand-github", "movie", "chart-dots-3", "terminal-2", "folder", "settings",
            "rotate", "rotate-clockwise", "circle-dot", "brackets-contain-start", "blade", "scissors",
            // Added for the keypad app's automatic Kdenlive icons (a920ddd).
            "arrow-bar-to-left", "arrow-bar-to-right", "arrow-left-bar", "arrow-right-bar", "arrows-left-right",
            "bookmark-off", "brackets-contain", "column-remove", "list-check", "ripple", "select-all", "space",
            "space-off", "square", "square-check",
            // The Kdenlive catalog (c271522).
            "unlink", "wave-sine", "layout-board", "tag",
        ] {
            assert!(!glyph(name).is_empty(), "{name}");
        }
        assert_eq!(glyph("player-play"), "\u{ed46}");
        assert_eq!(glyph("none"), "");
        assert_eq!(glyph("not-an-icon"), "");
        let names: std::collections::HashSet<&str> = all().iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names.len(), all().len(), "names are unique");
        assert!(all().iter().all(|i| crate::keypad::config::valid_icon(&i.name)));
    }

    #[test]
    fn the_ui_uses_the_embedded_fonts_family() {
        let json: Value = serde_json::from_str(include_str!("../../ui/assets/keypad-icons.json")).unwrap();
        let family = json["family"].as_str().unwrap();
        assert_eq!(family, "smplOS Keypad Icons", "smplOS's eww.scss names the same family");
        let ui = include_str!("../../ui/main.slint");
        assert!(ui.contains(&format!("out property <string> family: \"{family}\";")));
        assert!(ui.contains("import \"assets/keypad-icons.ttf\";"));
        let license = include_str!("../../ui/assets/TABLER-ICONS-LICENSE.txt");
        assert!(license.starts_with("MIT License") && license.contains("Paweł Kuna"));
    }

    #[test]
    fn search_finds_by_name_first_then_by_tag() {
        let names = |q: &str| search(q).iter().map(|i| i.name.clone()).collect::<Vec<_>>();
        assert_eq!(names("").len(), all().len());
        let volume = names("volume");
        assert!(volume.starts_with(&["volume".into(), "volume-2".into(), "volume-3".into(), "volume-off".into()]), "{volume:?}");
        assert!(names("octocat").contains(&"brand-github".to_string()), "tags count");
        assert!(names("PLAYER next").contains(&"player-track-next".to_string()), "every word, any case");
        assert!(names("zzzz").is_empty());
        let git = names("git");
        assert!(git.contains(&"brand-github".to_string()) && !git.contains(&"number-1".to_string()), "{git:?}");
    }
}
