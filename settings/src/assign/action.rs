//! One binding: what a button, key or knob does.
//!
//! The control-surface daemon's binding grammar (`"ctrl+c"`, `{"keys": ...}`,
//! `{"mouse": ...}`, `{"command": [...]}`, `"none"` ...) as the Settings app
//! edits it: `classify` reads a JSON value into a `Binding`, `to_json` writes
//! one back. Both the Keypad and the Mouse tab use it.

use super::json::{self, Json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Inherit,
    Disabled,
    Shortcut,
    Media,
    Mouse,
    Command,
    Kdenlive,
    Cheatsheet,
    Advanced,
}

impl ActionKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inherit => "Not set (use Global)",
            Self::Disabled => "Do nothing",
            Self::Shortcut => "Keyboard shortcut",
            Self::Media => "Media key",
            Self::Mouse => "Mouse button",
            Self::Command => "Run command",
            Self::Kdenlive => "Kdenlive action (API)",
            Self::Cheatsheet => "Show the cheatsheet",
            Self::Advanced => "Advanced (edit in file)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub kind: ActionKind,
    /// Shortcut text, media key, mouse button, command line, action id,
    /// cheatsheet mode or, for Advanced, the binding's JSON.
    pub value: String,
    /// The binding's own `"label"`: what the cheatsheet shows for it.
    pub label: String,
    /// The binding's `"icon"`: empty for the daemon's automatic icon,
    /// `ICON_NONE` for none, else an icon name (see `keypad::icons`).
    pub icon: String,
    /// `"ifInstalled"`: programs or desktop ids; while one is missing the
    /// daemon skips the binding (the slot falls through).
    pub needs: Vec<String>,
    /// `"ifInstalled"` was written as a list (kept even for one name).
    pub needs_list: bool,
}

/// `"icon": "none"`: the cheatsheet shows the label alone.
pub const ICON_NONE: &str = "none";

/// Fields any binding may carry beside its action; Settings edits them all.
const META_FIELDS: [&str; 3] = ["label", "icon", "ifInstalled"];

/// An `"ifInstalled"` name: a program or desktop id (the daemon refuses spaces).
fn valid_need(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_whitespace)
}

/// Icon names are kebab-case (Tabler's); the renderer skips names it lacks.
pub fn valid_icon(icon: &str) -> bool {
    !icon.is_empty()
        && icon.split('-').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}

impl Binding {
    pub fn new(kind: ActionKind, value: &str) -> Self {
        Self { kind, value: value.to_string(), label: String::new(), icon: String::new(), needs: Vec::new(), needs_list: false }
    }

    /// Sets `"ifInstalled"` from space-separated names.
    pub fn needing(mut self, names: &str) -> Self {
        self.needs = names.split_whitespace().map(String::from).collect();
        self.needs_list |= self.needs.len() > 1;
        self
    }

    pub fn labelled(mut self, label: &str) -> Self {
        self.label = label.trim().to_string();
        self
    }

    pub fn with_icon(mut self, icon: &str) -> Self {
        self.icon = icon.trim().to_string();
        self
    }

    /// Kinds whose label the cheatsheet can show (not "unset" or "nothing").
    pub fn takes_label(&self) -> bool {
        !matches!(self.kind, ActionKind::Inherit | ActionKind::Disabled | ActionKind::Advanced)
    }

    /// The label if it has one (what the cheatsheet shows), else the summary.
    pub fn short(&self) -> String {
        if self.label.is_empty() || !self.takes_label() {
            self.summary()
        } else {
            self.label.clone()
        }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            ActionKind::Inherit => String::new(),
            ActionKind::Disabled => "nothing".into(),
            ActionKind::Media => choice_label(MEDIA_KEYS, &self.value),
            ActionKind::Mouse => choice_label(MOUSE_BUTTONS, &self.value),
            ActionKind::Kdenlive => choice_label(KDENLIVE_ACTIONS, &self.value),
            ActionKind::Cheatsheet => format!("cheatsheet ({})", self.value),
            ActionKind::Advanced => "advanced".into(),
            ActionKind::Shortcut | ActionKind::Command => self.value.clone(),
        }
    }
}

pub const MEDIA_KEYS: &[(&str, &str)] = &[
    ("playpause", "Play / pause"),
    ("nextsong", "Next track"),
    ("previoussong", "Previous track"),
    ("stopcd", "Stop"),
    ("volumeup", "Volume up"),
    ("volumedown", "Volume down"),
    ("mute", "Mute"),
    ("brightnessup", "Brightness up"),
    ("brightnessdown", "Brightness down"),
];

/// Needs daemon support (smplOS API request R3); offered only when the
/// installed daemon accepts a `{"mouse": ...}` binding.
pub const MOUSE_BUTTONS: &[(&str, &str)] = &[
    ("left", "Left click"),
    ("right", "Right click"),
    ("middle", "Middle click"),
    ("back", "Back"),
    ("forward", "Forward"),
    ("wheel-up", "Scroll up"),
    ("wheel-down", "Scroll down"),
    ("wheel-left", "Scroll left"),
    ("wheel-right", "Scroll right"),
];

/// Kdenlive's curated action list (K23-MR1a) used when no Kdenlive is running
/// to ask; a running Kdenlive's own list replaces it.
pub const KDENLIVE_ACTIONS: &[(&str, &str)] = &[
    ("monitor_play", "Play / pause"),
    ("monitor_pause", "Pause"),
    ("mark_in", "Set in point"),
    ("mark_out", "Set out point"),
    ("insert_to_in_point", "Insert clip at in point"),
    ("overwrite_to_in_point", "Overwrite clip at in point"),
    ("switch_monitor", "Switch monitor"),
    ("cut_timeline_clip", "Cut clip at playhead"),
    ("delete_timeline_clip", "Delete selected clip"),
    ("monitor_seek_snap_backward", "Previous snap point"),
    ("monitor_seek_snap_forward", "Next snap point"),
    ("add_marker_guide_quickly", "Add marker / guide"),
    ("select_tool", "Selection tool"),
    ("razor_tool", "Razor tool"),
    ("edit_undo", "Undo"),
    ("edit_redo", "Redo"),
    ("zoom_fit", "Fit timeline zoom"),
    ("keyframe_add", "Add keyframe"),
    ("keyframe_next", "Next keyframe"),
    ("keyframe_previous", "Previous keyframe"),
    ("monitor_play_zone", "Play zone"),
    ("monitor_loop_clip", "Loop clip"),
    ("monitor_loop_zone", "Loop zone"),
    ("seek_zone_start", "Go to zone start"),
    ("seek_zone_end", "Go to zone end"),
    ("resize_timeline_clip_start", "Trim clip start to playhead"),
    ("resize_timeline_clip_end", "Trim clip end to playhead"),
    ("delete_space", "Remove space"),
    ("extract_clip", "Extract (ripple delete)"),
];

/// Key names the daemon accepts (its keynames table, without the KEY_ prefix).
const KEY_NAMES: &[&str] = &[
    "esc", "escape", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "minus", "equal", "backspace", "tab",
    "q", "w", "e", "r", "t", "y", "u", "i", "o", "p", "leftbrace", "rightbrace", "enter", "return",
    "leftctrl", "a", "s", "d", "f", "g", "h", "j", "k", "l", "semicolon", "apostrophe", "grave",
    "leftshift", "backslash", "z", "x", "c", "v", "b", "n", "m", "comma", "dot", "period", "slash",
    "rightshift", "kpasterisk", "leftalt", "space", "capslock", "f1", "f2", "f3", "f4", "f5", "f6", "f7",
    "f8", "f9", "f10", "numlock", "scrolllock", "kp7", "kp8", "kp9", "kpminus", "kp4", "kp5", "kp6",
    "kpplus", "kp1", "kp2", "kp3", "kp0", "kpdot", "f11", "f12", "kpenter", "rightctrl", "kpslash",
    "sysrq", "print", "rightalt", "home", "up", "pageup", "left", "right", "end", "down", "pagedown",
    "insert", "delete", "mute", "volumedown", "volumeup", "kpequal", "pause", "leftmeta", "rightmeta",
    "compose", "menu", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23", "f24",
    "playpause", "nextsong", "previoussong", "stopcd", "brightnessdown", "brightnessup",
];
const MODIFIERS: &[&str] = &["ctrl", "control", "shift", "alt", "super", "meta", "win", "logo"];

/// `{"cheatsheet": mode}`: keys and knob presses only.
pub const CHEATSHEET_MODES: &[(&str, &str)] = &[
    ("toggle", "Toggle: press to show, press again to hide"),
    ("hold", "Hold: shown while held"),
];

pub fn choice_label(table: &[(&str, &str)], id: &str) -> String {
    table
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| id.to_string())
}

// ── Validation ───────────────────────────────────────────────────────────────

fn valid_key(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    let n = n.strip_prefix("key_").unwrap_or(&n);
    KEY_NAMES.contains(&n)
        || n.strip_prefix('#')
            .and_then(|code| code.parse::<u32>().ok())
            .is_some_and(|code| code > 0 && code < 0x2ff)
}

fn check_chord(chord: &str) -> Result<(), String> {
    let parts: Vec<&str> = chord.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((key, mods)) = parts.split_last() else {
        return Err("empty shortcut".into());
    };
    if let Some(bad) = mods.iter().find(|m| !MODIFIERS.contains(&m.to_lowercase().as_str())) {
        return Err(format!("unknown modifier '{bad}' in '{chord}' (use ctrl, shift, alt, super)"));
    }
    if parts.len() == 1 && MODIFIERS.contains(&key.to_lowercase().as_str()) {
        return Ok(());
    }
    if !valid_key(key) {
        return Err(format!("unknown key '{key}' in '{chord}' (e.g. a, F13, space, enter, left)"));
    }
    Ok(())
}

/// Validates a shortcut or a space-separated sequence ("ctrl+k x").
pub fn normalize_shortcut(text: &str) -> Result<String, String> {
    let chords: Vec<&str> = text.split_whitespace().collect();
    if chords.is_empty() {
        return Err("type a shortcut such as ctrl+shift+s or F13".into());
    }
    for chord in &chords {
        check_chord(chord)?;
    }
    Ok(chords.join(" "))
}

/// Splits a command line like a shell would for plain words and quotes.
pub fn split_command(text: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => current.push(chars.next().unwrap_or('\\')),
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, '\\') => {
                current.push(chars.next().unwrap_or('\\'));
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("unterminated quote in command".into());
    }
    if in_word {
        args.push(current);
    }
    if args.is_empty() {
        return Err("type a command, e.g. notify-send hello".into());
    }
    Ok(args)
}

pub fn join_command(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c)) {
                a.clone()
            } else {
                format!("'{}'", a.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Classification ───────────────────────────────────────────────────────────

pub fn classify(value: Option<&Json>) -> Binding {
    let Some(value) = value else {
        return Binding::new(ActionKind::Inherit, "");
    };
    let advanced = || Binding::new(ActionKind::Advanced, &json::to_compact(value));
    match value {
        Json::Null => Binding::new(ActionKind::Disabled, ""),
        Json::Str(s) if s == "none" => Binding::new(ActionKind::Disabled, ""),
        Json::Str(s) => {
            if let Some(action) = s.strip_prefix("action:") {
                return Binding::new(ActionKind::Kdenlive, action);
            }
            if s.contains(':') {
                return advanced();
            }
            let lower = s.trim().to_lowercase();
            if MEDIA_KEYS.iter().any(|(k, _)| *k == lower) {
                return Binding::new(ActionKind::Media, &lower);
            }
            Binding::new(ActionKind::Shortcut, s)
        }
        Json::Obj(entries) => {
            // "label" and "icon" are the cheatsheet's and may accompany any form.
            let meta = |key: &str| match value.get(key) {
                Some(Json::Str(v)) => Some(v.as_str()),
                Some(_) => None,
                None => Some(""),
            };
            let (Some(label), Some(icon)) = (meta("label"), meta("icon")) else {
                return advanced();
            };
            if !icon.is_empty() && !valid_icon(icon) {
                return advanced();
            }
            let (needs, needs_list) = match value.get("ifInstalled") {
                None => (Vec::new(), false),
                Some(Json::Str(n)) if valid_need(n) => (vec![n.clone()], false),
                Some(Json::Arr(items)) if !items.is_empty() && items.iter().all(|i| i.as_str().is_some_and(valid_need)) => {
                    (items.iter().filter_map(Json::as_str).map(String::from).collect(), true)
                }
                Some(_) => return advanced(),
            };
            let labelled = |mut b: Binding| {
                b.needs = needs.clone();
                b.needs_list = needs_list;
                b.labelled(label).with_icon(icon)
            };
            let rest: Vec<&(String, Json)> = entries.iter().filter(|(k, _)| !META_FIELDS.contains(&k.as_str())).collect();
            let only = |key: &str| rest.len() == 1 && rest[0].0 == key;
            if only("keys") {
                match &rest[0].1 {
                    Json::Str(s) => {
                        let lower = s.trim().to_lowercase();
                        if MEDIA_KEYS.iter().any(|(k, _)| *k == lower) {
                            return labelled(Binding::new(ActionKind::Media, &lower));
                        }
                        return labelled(Binding::new(ActionKind::Shortcut, s));
                    }
                    Json::Arr(items) if items.iter().all(|i| i.as_str().is_some()) => {
                        let seq: Vec<&str> = items.iter().filter_map(Json::as_str).collect();
                        return labelled(Binding::new(ActionKind::Shortcut, &seq.join(" ")));
                    }
                    _ => {}
                }
            }
            if let Some(Json::Str(action)) = value.get("action") {
                // Extra fields (fallback, options) are kept when it is edited.
                return labelled(Binding::new(ActionKind::Kdenlive, action));
            }
            if let (true, Some(Json::Arr(argv))) = (only("command"), value.get("command")) {
                let argv: Vec<String> = argv.iter().filter_map(|a| a.as_str().map(String::from)).collect();
                return labelled(Binding::new(ActionKind::Command, &join_command(&argv)));
            }
            if let (true, Some(Json::Str(button))) = (only("mouse"), value.get("mouse")) {
                return labelled(Binding::new(ActionKind::Mouse, button));
            }
            if let (true, Some(Json::Str(mode))) = (only("cheatsheet"), value.get("cheatsheet")) {
                return labelled(Binding::new(ActionKind::Cheatsheet, mode));
            }
            advanced()
        }
        _ => advanced(),
    }
}

/// The JSON for a binding; `None` removes the slot (inherit). A label or icon
/// turns the short string forms into objects (`{"keys": "ctrl+z", "label": "Undo"}`).
pub fn to_json(binding: &Binding) -> Result<Option<Json>, String> {
    let label = binding.label.trim();
    if label.chars().count() > 40 {
        return Err("keep the cheatsheet label under 40 characters".into());
    }
    let icon = binding.icon.trim();
    if !icon.is_empty() && !valid_icon(icon) {
        return Err(format!("'{icon}' is not an icon name"));
    }
    if let Some(bad) = binding.needs.iter().find(|n| !valid_need(n)) {
        return Err(format!("'{bad}' is not a program or app id"));
    }
    let needs = match binding.needs.as_slice() {
        [] => None,
        [one] if !binding.needs_list => Some(Json::str(one)),
        many => Some(Json::Arr(many.iter().map(|n| Json::str(n)).collect())),
    };
    let with_label = |mut entries: Vec<(String, Json)>| {
        if let Some(needs) = &needs {
            entries.push(("ifInstalled".into(), needs.clone()));
        }
        if !label.is_empty() {
            entries.push(("label".into(), Json::str(label)));
        }
        if !icon.is_empty() {
            entries.push(("icon".into(), Json::str(icon)));
        }
        Json::Obj(entries)
    };
    let keys = |text: &str| {
        if label.is_empty() && icon.is_empty() && needs.is_none() {
            Json::str(text)
        } else {
            with_label(vec![("keys".into(), Json::str(text))])
        }
    };
    Ok(Some(match binding.kind {
        ActionKind::Inherit => return Ok(None),
        ActionKind::Advanced => return Err("advanced bindings are edited in the config file".into()),
        ActionKind::Disabled => Json::str("none"),
        ActionKind::Shortcut => keys(&normalize_shortcut(&binding.value)?),
        ActionKind::Media => {
            if !MEDIA_KEYS.iter().any(|(k, _)| *k == binding.value) {
                return Err(format!("unknown media key '{}'", binding.value));
            }
            keys(&binding.value)
        }
        ActionKind::Mouse => {
            if !MOUSE_BUTTONS.iter().any(|(k, _)| *k == binding.value) {
                return Err(format!("unknown mouse button '{}'", binding.value));
            }
            with_label(vec![("mouse".into(), Json::str(&binding.value))])
        }
        ActionKind::Command => {
            let argv = split_command(&binding.value)?;
            with_label(vec![("command".into(), Json::Arr(argv.into_iter().map(Json::Str).collect()))])
        }
        ActionKind::Kdenlive => {
            let id = binding.value.trim();
            if id.is_empty() || id.contains(char::is_whitespace) {
                return Err("choose a Kdenlive action".into());
            }
            with_label(vec![("action".into(), Json::str(id))])
        }
        ActionKind::Cheatsheet => {
            if !CHEATSHEET_MODES.iter().any(|(m, _)| *m == binding.value) {
                return Err("choose toggle or hold for the cheatsheet".into());
            }
            with_label(vec![("cheatsheet".into(), Json::str(&binding.value))])
        }
    }))
}

/// Keeps what Settings doesn't edit (e.g. a Kdenlive action's "fallback")
/// when only the label, icon or ifInstalled changed. Those fields belong to
/// one specific action or command, so any other edit starts clean. Keys keep
/// the order they had in the file, so applying an unchanged binding changes
/// nothing.
pub(crate) fn merge_extras(old: Option<&Json>, new: Json) -> Json {
    const FORMS: [&str; 5] = ["keys", "action", "command", "mouse", "cheatsheet"];
    let (Some(Json::Obj(old)), Json::Obj(fresh)) = (old, &new) else {
        return new;
    };
    let main = |e: &[(String, Json)]| e.iter().find(|(k, _)| FORMS.contains(&k.as_str())).cloned();
    let same_action = main(old).is_some() && main(old) == main(fresh);
    let mut entries: Vec<(String, Json)> = Vec::new();
    for (k, v) in old {
        if let Some((_, value)) = fresh.iter().find(|(f, _)| f == k) {
            entries.push((k.clone(), value.clone()));
        } else if same_action && !META_FIELDS.contains(&k.as_str()) && !FORMS.contains(&k.as_str()) {
            entries.push((k.clone(), v.clone()));
        }
    }
    for (k, v) in fresh {
        if !entries.iter().any(|(e, _)| e == k) {
            entries.push((k.clone(), v.clone()));
        }
    }
    Json::Obj(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Json {
        json::parse(text).unwrap()
    }

    fn stored(b: &Binding) -> Json {
        to_json(b).unwrap().expect("stored")
    }

    #[test]
    fn every_kind_a_mouse_button_can_have_survives_a_round_trip() {
        let cases = [
            Binding::new(ActionKind::Disabled, ""),
            Binding::new(ActionKind::Shortcut, "alt+left"),
            Binding::new(ActionKind::Shortcut, "ctrl+k x"),
            Binding::new(ActionKind::Media, "playpause"),
            Binding::new(ActionKind::Mouse, "middle"),
            Binding::new(ActionKind::Command, "notify-send 'hello world'"),
            Binding::new(ActionKind::Shortcut, "super+1").labelled("Workspace 1").with_icon("home").needing("hyprctl"),
            Binding::new(ActionKind::Command, "gtk-launch nemo").labelled("Files").needing("nemo thunar"),
        ];
        for b in cases {
            assert_eq!(classify(Some(&stored(&b))), b, "{b:?}");
        }
    }

    #[test]
    fn unset_is_not_stored_and_advanced_is_never_written() {
        assert_eq!(to_json(&Binding::new(ActionKind::Inherit, "")), Ok(None));
        assert_eq!(classify(None).kind, ActionKind::Inherit);
        assert_eq!(
            to_json(&Binding::new(ActionKind::Advanced, "{}")),
            Err("advanced bindings are edited in the config file".to_string())
        );
    }

    #[test]
    fn the_short_forms_stay_short_until_a_label_icon_or_requirement_is_added() {
        let text = |b: Binding| json::to_compact(&stored(&b));
        let plain = Binding::new(ActionKind::Shortcut, "alt+left");
        assert_eq!(text(plain.clone()), r#""alt+left""#);
        assert_eq!(text(plain.clone().labelled("Back")), r#"{ "keys": "alt+left", "label": "Back" }"#);
        assert_eq!(
            text(plain.clone().labelled("Back").with_icon("arrow-left").needing("firefox")),
            r#"{ "keys": "alt+left", "ifInstalled": "firefox", "label": "Back", "icon": "arrow-left" }"#
        );
        assert_eq!(text(Binding::new(ActionKind::Mouse, "back")), r#"{ "mouse": "back" }"#);
        assert_eq!(text(Binding::new(ActionKind::Disabled, "")), r#""none""#);
        assert_eq!(
            text(Binding::new(ActionKind::Command, "notify-send hi").needing("a b")),
            r#"{ "command": ["notify-send", "hi"], "ifInstalled": ["a", "b"] }"#
        );
    }

    #[test]
    fn media_keys_are_recognised_in_either_form() {
        assert_eq!(classify(Some(&parse(r#""mute""#))), Binding::new(ActionKind::Media, "mute"));
        assert_eq!(classify(Some(&parse(r#"{"keys":"VolumeUp"}"#))), Binding::new(ActionKind::Media, "volumeup"));
        assert_eq!(classify(Some(&parse(r#"{"keys":["ctrl","c"]}"#))), Binding::new(ActionKind::Shortcut, "ctrl c"));
        assert_eq!(classify(Some(&parse("null"))), Binding::new(ActionKind::Disabled, ""));
    }

    #[test]
    fn forms_the_editor_cannot_express_stay_advanced() {
        for text in [
            r#"{"command":["a"],"repeat":true}"#,
            r#"{"mouse":"left","label":5}"#,
            r#"{"keys":"ctrl+c","icon":"Bad Icon"}"#,
            r#"{"keys":"x","ifInstalled":["two words"]}"#,
            r#"{"keys":"x","ifInstalled":[]}"#,
            r#"{"mouse":"left","keys":"x"}"#,
            r#""ctrl:shift""#,
            "42",
            "[1]",
        ] {
            let shown = classify(Some(&parse(text)));
            assert_eq!(shown.kind, ActionKind::Advanced, "{text}");
            assert_eq!(shown.value, json::to_compact(&parse(text)), "{text}");
        }
    }

    #[test]
    fn only_the_cheatsheet_capable_kinds_take_a_label() {
        for kind in [ActionKind::Inherit, ActionKind::Disabled, ActionKind::Advanced] {
            assert!(!Binding::new(kind, "").takes_label(), "{kind:?}");
        }
        for kind in [ActionKind::Shortcut, ActionKind::Media, ActionKind::Mouse, ActionKind::Command] {
            assert!(Binding::new(kind, "x").takes_label(), "{kind:?}");
        }
        let b = Binding::new(ActionKind::Shortcut, "alt+left").labelled(" Back ");
        assert_eq!((b.short().as_str(), b.summary().as_str()), ("Back", "alt+left"));
        assert_eq!(Binding::new(ActionKind::Mouse, "back").summary(), "Back");
        assert_eq!(Binding::new(ActionKind::Mouse, "side-9").summary(), "side-9", "an unknown id is shown as it is");
        assert_eq!(Binding::new(ActionKind::Disabled, "").summary(), "nothing");
        assert_eq!(Binding::new(ActionKind::Inherit, "").summary(), "");
    }

    #[test]
    fn invalid_values_are_refused_with_a_reason() {
        let err = |b: Binding| to_json(&b).unwrap_err();
        assert_eq!(err(Binding::new(ActionKind::Media, "rewind")), "unknown media key 'rewind'");
        assert_eq!(err(Binding::new(ActionKind::Mouse, "side-9")), "unknown mouse button 'side-9'");
        assert_eq!(err(Binding::new(ActionKind::Command, "  ")), "type a command, e.g. notify-send hello");
        assert_eq!(err(Binding::new(ActionKind::Cheatsheet, "flash")), "choose toggle or hold for the cheatsheet");
        assert_eq!(err(Binding::new(ActionKind::Kdenlive, "two words")), "choose a Kdenlive action");
        assert_eq!(
            err(Binding::new(ActionKind::Shortcut, "ctrl+banana")),
            "unknown key 'banana' in 'ctrl+banana' (e.g. a, F13, space, enter, left)"
        );
        assert_eq!(
            err(Binding::new(ActionKind::Shortcut, "alt+left").labelled(&"x".repeat(41))),
            "keep the cheatsheet label under 40 characters"
        );
        assert_eq!(
            err(Binding::new(ActionKind::Shortcut, "alt+left").with_icon("Not An Icon")),
            "'Not An Icon' is not an icon name"
        );

        let mut spaced = Binding::new(ActionKind::Shortcut, "alt+left");
        spaced.needs = vec!["two words".into()];
        assert_eq!(err(spaced), "'two words' is not a program or app id");
    }

    #[test]
    fn shortcuts_are_checked_against_the_keys_the_daemon_knows() {
        assert_eq!(normalize_shortcut("  ctrl+shift+s "), Ok("ctrl+shift+s".to_string()));
        assert_eq!(normalize_shortcut("ctrl+k   x"), Ok("ctrl+k x".to_string()));
        assert_eq!(normalize_shortcut("F13"), Ok("F13".to_string()));
        assert_eq!(normalize_shortcut("super"), Ok("super".to_string()), "a lone modifier is allowed");
        assert_eq!(normalize_shortcut("#190"), Ok("#190".to_string()), "a raw key code is allowed");
        assert_eq!(normalize_shortcut("KEY_VOLUMEUP"), Ok("KEY_VOLUMEUP".to_string()));
        assert!(normalize_shortcut("").is_err());
        assert!(normalize_shortcut("#0").is_err());
        assert!(normalize_shortcut("hyper+a").unwrap_err().contains("unknown modifier 'hyper'"));
        assert!(normalize_shortcut("ctrl+banana").unwrap_err().contains("unknown key 'banana'"));
    }

    #[test]
    fn commands_split_on_words_and_quotes_and_join_back() {
        let argv = split_command(r#"notify-send "hello world" 'it'\''s' plain\ space"#).unwrap();
        assert_eq!(argv, ["notify-send", "hello world", "it's", "plain space"]);
        assert_eq!(split_command(&join_command(&argv)).unwrap(), argv);
        assert_eq!(join_command(&["a".to_string(), "b c".to_string(), String::new()]), "a 'b c' ''");
        assert_eq!(split_command("echo ''").unwrap(), ["echo", ""]);
        assert_eq!(split_command("echo 'oops"), Err("unterminated quote in command".to_string()));
        assert!(split_command("   ").is_err());
    }

    #[test]
    fn a_choice_shows_its_label_or_the_id_when_the_table_lacks_it() {
        assert_eq!(choice_label(MOUSE_BUTTONS, "wheel-left"), "Scroll left");
        assert_eq!(choice_label(MOUSE_BUTTONS, "tilt-9"), "tilt-9");
        assert!(valid_icon("arrow-left") && valid_icon("player-play-2"));
        assert!(!valid_icon("") && !valid_icon("Arrow") && !valid_icon("a--b") && !valid_icon("-a"));
    }

    #[test]
    fn what_the_editor_does_not_touch_survives_an_unchanged_action() {
        let old = parse(r#"{"action":"mark_in","fallback":"ctrl+i","label":"In"}"#);
        let relabelled = merge_extras(Some(&old), parse(r#"{"action":"mark_in","label":"Mark in"}"#));
        assert_eq!(json::to_compact(&relabelled), r#"{ "action": "mark_in", "fallback": "ctrl+i", "label": "Mark in" }"#);

        let other = merge_extras(Some(&old), parse(r#"{"action":"mark_out"}"#));
        assert_eq!(json::to_compact(&other), r#"{ "action": "mark_out" }"#, "another action starts clean");

        let fresh = parse(r#"{"mouse":"back"}"#);
        assert_eq!(merge_extras(None, fresh.clone()), fresh);
        assert_eq!(merge_extras(Some(&Json::str("alt+left")), fresh.clone()), fresh);
    }
}
