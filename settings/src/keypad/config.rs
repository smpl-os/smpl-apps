//! The control-surface daemon's config, as Settings edits it.
//!
//! Settings edits per-profile base bindings in the simple forms (shortcut,
//! media key, mouse, command, Kdenlive action, disabled) and the profile
//! header (app match, Kdenlive API plugin). Everything else (layers, modes,
//! continuous controls, cycles, requests, hardware, settings) is preserved
//! verbatim and shown read-only as "Advanced".

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
            Self::Advanced => "Advanced (edit in file)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub kind: ActionKind,
    /// Shortcut text, media key, mouse button, command line, action id or,
    /// for Advanced, the binding's JSON.
    pub value: String,
}

impl Binding {
    pub fn new(kind: ActionKind, value: &str) -> Self {
        Self { kind, value: value.to_string() }
    }

    pub fn summary(&self) -> String {
        match self.kind {
            ActionKind::Inherit => String::new(),
            ActionKind::Disabled => "nothing".into(),
            ActionKind::Media => choice_label(MEDIA_KEYS, &self.value),
            ActionKind::Mouse => choice_label(MOUSE_BUTTONS, &self.value),
            ActionKind::Kdenlive => choice_label(KDENLIVE_ACTIONS, &self.value),
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

pub const KNOB_EVENTS: &[(&str, &str)] = &[("ccw", "Turn left"), ("cw", "Turn right"), ("press", "Press")];

pub fn choice_label(table: &[(&str, &str)], id: &str) -> String {
    table
        .iter()
        .find(|(k, _)| *k == id)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| id.to_string())
}

/// "key7" -> "Key 7", "knob2.cw" -> "Knob 2 · Turn right".
pub fn slot_label(slot: &str) -> String {
    let (control, event) = slot.split_once('.').unwrap_or((slot, ""));
    let base = if let Some(n) = control.strip_prefix("key") {
        format!("Key {n}")
    } else if let Some(n) = control.strip_prefix("knob") {
        format!("Knob {n}")
    } else {
        control.to_string()
    };
    match event {
        "" => base,
        "turn" => format!("{base} · Turn"),
        e => format!("{base} · {}", choice_label(KNOB_EVENTS, e)),
    }
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
            let only = |key: &str| entries.len() == 1 && entries[0].0 == key;
            if only("keys") {
                match &entries[0].1 {
                    Json::Str(s) => return Binding::new(ActionKind::Shortcut, s),
                    Json::Arr(items) if items.iter().all(|i| i.as_str().is_some()) => {
                        let seq: Vec<&str> = items.iter().filter_map(Json::as_str).collect();
                        return Binding::new(ActionKind::Shortcut, &seq.join(" "));
                    }
                    _ => {}
                }
            }
            if let Some(Json::Str(action)) = value.get("action") {
                // Keep extra fields (fallback, label) intact by treating the
                // binding as API-only for display; editing replaces it.
                return Binding::new(ActionKind::Kdenlive, action);
            }
            if let (true, Some(Json::Arr(argv))) = (only("command"), value.get("command")) {
                let argv: Vec<String> = argv.iter().filter_map(|a| a.as_str().map(String::from)).collect();
                return Binding::new(ActionKind::Command, &join_command(&argv));
            }
            if let (true, Some(Json::Str(button))) = (only("mouse"), value.get("mouse")) {
                return Binding::new(ActionKind::Mouse, button);
            }
            advanced()
        }
        _ => advanced(),
    }
}

/// The JSON for a binding; `None` removes the slot (inherit).
pub fn to_json(binding: &Binding) -> Result<Option<Json>, String> {
    Ok(Some(match binding.kind {
        ActionKind::Inherit => return Ok(None),
        ActionKind::Advanced => return Err("advanced bindings are edited in the config file".into()),
        ActionKind::Disabled => Json::str("none"),
        ActionKind::Shortcut => Json::Str(normalize_shortcut(&binding.value)?),
        ActionKind::Media => {
            if !MEDIA_KEYS.iter().any(|(k, _)| *k == binding.value) {
                return Err(format!("unknown media key '{}'", binding.value));
            }
            Json::str(&binding.value)
        }
        ActionKind::Mouse => {
            if !MOUSE_BUTTONS.iter().any(|(k, _)| *k == binding.value) {
                return Err(format!("unknown mouse button '{}'", binding.value));
            }
            Json::Obj(vec![("mouse".into(), Json::str(&binding.value))])
        }
        ActionKind::Command => {
            let argv = split_command(&binding.value)?;
            Json::Obj(vec![("command".into(), Json::Arr(argv.into_iter().map(Json::Str).collect()))])
        }
        ActionKind::Kdenlive => {
            let id = binding.value.trim();
            if id.is_empty() || id.contains(char::is_whitespace) {
                return Err("choose a Kdenlive action".into());
            }
            Json::Obj(vec![("action".into(), Json::str(id))])
        }
    }))
}

// ── Profiles ─────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct ProfileInfo {
    pub name: String,
    /// `match.class` regex; empty for the global (match-less) profile.
    pub class: String,
    pub global: bool,
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
fn pattern_covers(pattern: &str, class: &str) -> bool {
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

/// The config's `"layout"`: unset (the daemon decides), a board profile id,
/// or a custom grid.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Auto,
    Board(String),
    Custom { keys: usize, knobs: usize, columns: usize },
}

pub const MAX_KEYS: usize = 16;
pub const MAX_KNOBS: usize = 3;
pub const MAX_COLUMNS: usize = 8;

pub struct KeypadConfig {
    pub doc: Json,
}

pub const DEFAULT_CONFIG: &str = r#"{
    "device": { "vendor": "1189", "product": "8890" },
    "hardware": "default:keys-then-knobs",
    "profiles": [
        {
            "name": "global",
            "bindings": {
                "knob1": { "ccw": "volumedown", "cw": "volumeup", "press": "mute" }
            }
        }
    ]
}
"#;

pub const HEADER: &str = "// control-surface configuration, written by smplOS Settings > Keypad.\n\
// Hand edits are fine; Settings keeps unknown entries. Comments are not kept:\n\
// the previous version of this file is in backups/ next to it.\n\
// Check a file with: control-surfaced check-config -c FILE\n";

impl KeypadConfig {
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = json::parse(text)?;
        if !matches!(doc, Json::Obj(_)) {
            return Err("the config must be a JSON object".into());
        }
        if doc.get("profiles").is_some_and(|p| p.as_array().is_none()) {
            return Err("\"profiles\" must be a list".into());
        }
        Ok(Self { doc })
    }

    pub fn render(&self) -> String {
        format!("{HEADER}{}", json::to_pretty(&self.doc))
    }

    fn profile_list(&self) -> &[Json] {
        self.doc.get("profiles").and_then(Json::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    fn profile_list_mut(&mut self) -> &mut Vec<Json> {
        if self.doc.get("profiles").and_then(Json::as_array).is_none() {
            self.doc.set("profiles", Json::Arr(Vec::new()));
        }
        self.doc.get_mut("profiles").and_then(Json::as_array_mut).expect("profiles list")
    }

    pub fn profiles(&self) -> Vec<ProfileInfo> {
        self.profile_list()
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let class = p.get("match").and_then(|m| m.get("class")).and_then(Json::as_str).unwrap_or("");
                let has_match = p.get("match").is_some_and(|m| !m.entries().is_empty());
                ProfileInfo {
                    name: p.get("name").and_then(Json::as_str).map(String::from).unwrap_or(format!("profile {}", i + 1)),
                    class: class.to_string(),
                    global: !has_match,
                    kdenlive: p.get("kdenlive").and_then(Json::as_bool).unwrap_or(false),
                    key_fallback: p.get("keyFallback").and_then(Json::as_bool).unwrap_or(false),
                    layers: p.get("layers").and_then(Json::as_array).map_or(0, Vec::len),
                }
            })
            .collect()
    }

    /// Makes sure a global profile exists (last, so app profiles match first).
    pub fn ensure_global(&mut self) -> usize {
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

    pub fn binding(&self, profile: usize, slot: &str) -> Binding {
        let Some(bindings) = self.profile_list().get(profile).and_then(|p| p.get("bindings")) else {
            return classify(None);
        };
        if let Some(flat) = bindings.get(slot) {
            return classify(Some(flat));
        }
        match slot.split_once('.') {
            Some((knob, event)) => classify(bindings.get(knob).and_then(|k| k.get(event))),
            None => classify(None),
        }
    }

    /// Base bindings of a profile, flattened to slots, in file order.
    pub fn bound_slots(&self, profile: usize) -> Vec<(String, Binding)> {
        let mut out = Vec::new();
        let Some(bindings) = self.profile_list().get(profile).and_then(|p| p.get("bindings")) else {
            return out;
        };
        for (slot, value) in bindings.entries() {
            let nested = slot.starts_with("knob")
                && !slot.contains('.')
                && matches!(value, Json::Obj(_))
                && ["keys", "action", "control"].iter().all(|k| value.get(k).is_none());
            if nested {
                for (event, inner) in value.entries() {
                    let binding = if event == "shift" {
                        Binding::new(ActionKind::Advanced, &json::to_compact(inner))
                    } else {
                        classify(Some(inner))
                    };
                    out.push((format!("{slot}.{event}"), binding));
                }
            } else {
                out.push((slot.clone(), classify(Some(value))));
            }
        }
        out
    }

    /// Sets a key ("key3") or knob event ("knob1.cw") binding. Returns a note
    /// when another binding had to change for this one to take effect.
    pub fn set_binding(&mut self, profile: usize, slot: &str, binding: &Binding) -> Result<Option<String>, String> {
        let value = to_json(binding)?;
        let bindings = self.profile_mut(profile)?.object_mut("bindings");
        let mut note = None;
        match slot.split_once('.') {
            None => match value {
                Some(v) => bindings.set(slot, v),
                None => {
                    bindings.remove(slot);
                }
            },
            Some((knob, event)) => {
                // A knob's "turn" wins over "ccw"/"cw" at the same level.
                let removes_turn = matches!(event, "ccw" | "cw") && value.is_some();
                if bindings.get(slot).is_some() {
                    match value {
                        Some(v) => bindings.set(slot, v),
                        None => {
                            bindings.remove(slot);
                        }
                    }
                } else {
                    let nested = bindings.object_mut(knob);
                    match value {
                        Some(v) => nested.set(event, v),
                        None => {
                            nested.remove(event);
                        }
                    }
                    if removes_turn && nested.remove("turn").is_some() {
                        note = Some(format!("Replaced the continuous turn binding of {}", slot_label(knob)));
                    }
                    if nested.entries().is_empty() {
                        bindings.remove(knob);
                    }
                }
                let flat_turn = format!("{knob}.turn");
                if removes_turn && bindings.remove(&flat_turn).is_some() {
                    note = Some(format!("Replaced the continuous turn binding of {}", slot_label(knob)));
                }
            }
        }
        Ok(note)
    }

    pub fn layout(&self) -> Layout {
        let num = |v: &Json, k: &str| match v.get(k) {
            Some(Json::Num(n)) => n.parse::<usize>().ok(),
            _ => None,
        };
        match self.doc.get("layout") {
            Some(Json::Str(id)) => Layout::Board(id.clone()),
            Some(o @ Json::Obj(_)) => Layout::Custom {
                keys: num(o, "keys").unwrap_or(0),
                knobs: num(o, "knobs").unwrap_or(0),
                columns: num(o, "columns").unwrap_or(1),
            },
            _ => Layout::Auto,
        }
    }

    /// Sets the config's `"layout"`; `Auto` removes it.
    pub fn set_layout(&mut self, layout: &Layout) -> Result<(), String> {
        match layout {
            Layout::Auto => {
                self.doc.remove("layout");
            }
            Layout::Board(id) => {
                if id.is_empty() || id.contains(char::is_whitespace) {
                    return Err("choose a keypad variant".into());
                }
                self.doc.set("layout", Json::str(id));
            }
            Layout::Custom { keys, knobs, columns } => {
                if !(1..=MAX_KEYS).contains(keys) || *knobs > MAX_KNOBS || !(1..=MAX_COLUMNS).contains(columns) {
                    return Err(format!(
                        "a custom layout needs 1-{MAX_KEYS} keys, 0-{MAX_KNOBS} knobs and 1-{MAX_COLUMNS} columns"
                    ));
                }
                self.doc.set(
                    "layout",
                    Json::Obj(vec![
                        ("keys".into(), Json::Num(keys.to_string())),
                        ("knobs".into(), Json::Num(knobs.to_string())),
                        ("columns".into(), Json::Num(columns.to_string())),
                    ]),
                );
            }
        }
        Ok(())
    }

    /// Adds an app profile before the global one; returns its index.
    pub fn add_app_profile(&mut self, name: &str, class: &str) -> Result<usize, String> {
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

    pub fn remove_profile(&mut self, index: usize) -> Result<(), String> {
        let info = self.profiles().get(index).cloned().ok_or("no such profile")?;
        if info.global {
            return Err("the Global profile can't be removed".into());
        }
        self.profile_list_mut().remove(index);
        Ok(())
    }

    pub fn set_profile_class(&mut self, index: usize, regex: &str) -> Result<(), String> {
        if self.profiles().get(index).is_some_and(|p| p.global) {
            return Err("the Global profile applies to every app".into());
        }
        if regex.trim().is_empty() {
            return Err("the window class pattern can't be empty".into());
        }
        self.profile_mut(index)?.object_mut("match").set("class", Json::str(regex.trim()));
        Ok(())
    }

    pub fn set_profile_flag(&mut self, index: usize, key: &str, on: bool) -> Result<(), String> {
        let profile = self.profile_mut(index)?;
        if on {
            profile.set(key, Json::Bool(true));
        } else {
            profile.remove(key);
        }
        Ok(())
    }

    /// Replaces (or inserts before Global) the Kdenlive profile with `preset`,
    /// a profile from the daemon's example config. Returns its index.
    pub fn apply_kdenlive_preset(&mut self, preset: Json) -> usize {
        if let Some(i) = self.profiles().iter().position(|p| p.kdenlive) {
            self.profile_list_mut()[i] = preset;
            return i;
        }
        let global = self.ensure_global();
        self.profile_list_mut().insert(global, preset);
        global
    }
}

/// The Kdenlive profile from a config text (the daemon's example).
pub fn kdenlive_preset(example: &str) -> Option<Json> {
    let doc = KeypadConfig::parse(example).ok()?.doc;
    doc.get("profiles")?
        .as_array()?
        .iter()
        .find(|p| p.get("kdenlive").and_then(Json::as_bool).unwrap_or(false))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> KeypadConfig {
        KeypadConfig::parse(
            r#"// comment
            {
                "device": { "vendor": "1189", "product": "8890", "serial": "key153" },
                "profiles": [
                    { "name": "kdenlive", "match": { "class": "^org\\.kde\\.kdenlive" }, "kdenlive": true,
                      "layers": [ { "name": "x", "when": {}, "bindings": {} } ],
                      "bindings": {
                        "key1": { "action": "mark_in", "fallback": "i" },
                        "key13": { "cycle": "page", "label": "Page" },
                        "knob1": { "turn": { "control": "playhead.jog" }, "press": { "action": "monitor_play" } },
                      } },
                    { "name": "global", "bindings": {
                        "knob1": { "ccw": "volumedown", "cw": "volumeup", "press": "mute" },
                        "knob2.cw": "right",
                        "key11": "previoussong",
                        "key2": { "keys": ["ctrl+k", "x"] },
                        "key3": { "command": ["notify-send", "hi there"] },
                    } },
                ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn classifies_every_supported_form() {
        let c = config();
        assert_eq!(c.binding(0, "key1"), Binding::new(ActionKind::Kdenlive, "mark_in"));
        assert_eq!(c.binding(0, "key13").kind, ActionKind::Advanced);
        assert_eq!(c.binding(0, "knob1.press"), Binding::new(ActionKind::Kdenlive, "monitor_play"));
        assert_eq!(c.binding(0, "knob1.cw").kind, ActionKind::Inherit);
        assert_eq!(c.binding(1, "knob1.ccw"), Binding::new(ActionKind::Media, "volumedown"));
        assert_eq!(c.binding(1, "knob2.cw"), Binding::new(ActionKind::Shortcut, "right"));
        assert_eq!(c.binding(1, "key11"), Binding::new(ActionKind::Media, "previoussong"));
        assert_eq!(c.binding(1, "key2"), Binding::new(ActionKind::Shortcut, "ctrl+k x"));
        assert_eq!(c.binding(1, "key3"), Binding::new(ActionKind::Command, "notify-send 'hi there'"));
        assert_eq!(classify(Some(&Json::str("none"))).kind, ActionKind::Disabled);
        assert_eq!(classify(Some(&Json::str("cycle:page"))).kind, ActionKind::Advanced);
        assert_eq!(classify(Some(&json::parse(r#"{"mouse":"left"}"#).unwrap())).kind, ActionKind::Mouse);
    }

    #[test]
    fn profiles_report_match_plugin_and_layers() {
        let profiles = config().profiles();
        assert_eq!(profiles.len(), 2);
        assert!(profiles[0].kdenlive && !profiles[0].global && profiles[0].layers == 1);
        assert_eq!(profiles[0].class, r"^org\.kde\.kdenlive");
        assert!(profiles[1].global);
        assert_eq!(profiles[1].title(), "global (all other apps)");
    }

    #[test]
    fn setting_bindings_keeps_everything_else() {
        let mut c = config();
        c.set_binding(1, "key5", &Binding::new(ActionKind::Shortcut, "Ctrl+Shift+S")).unwrap();
        c.set_binding(1, "knob1.press", &Binding::new(ActionKind::Command, "playerctl play-pause")).unwrap();
        c.set_binding(1, "knob2.cw", &Binding::new(ActionKind::Media, "nextsong")).unwrap();
        c.set_binding(1, "key11", &Binding::new(ActionKind::Inherit, "")).unwrap();
        c.set_binding(1, "knob3.cw", &Binding::new(ActionKind::Disabled, "")).unwrap();
        let out = c.render();
        assert!(out.starts_with(HEADER));
        let doc = KeypadConfig::parse(&out).unwrap();
        assert_eq!(doc.binding(1, "key5"), Binding::new(ActionKind::Shortcut, "Ctrl+Shift+S"));
        assert_eq!(doc.binding(1, "knob1.press"), Binding::new(ActionKind::Command, "playerctl play-pause"));
        assert_eq!(doc.binding(1, "knob1.ccw"), Binding::new(ActionKind::Media, "volumedown"));
        assert_eq!(doc.binding(1, "knob2.cw"), Binding::new(ActionKind::Media, "nextsong"));
        assert_eq!(doc.binding(1, "key11").kind, ActionKind::Inherit);
        assert!(out.contains(r#""knob3": { "cw": "none" }"#), "{out}");
        // untouched: device serial, layers, advanced bindings, number/key order
        assert!(out.contains(r#""serial": "key153""#));
        assert!(out.contains(r#""key13": { "cycle": "page", "label": "Page" }"#));
        assert!(out.contains(r#""key1": { "action": "mark_in", "fallback": "i" }"#));
        assert_eq!(doc.profiles()[0].layers, 1);
    }

    #[test]
    fn turning_a_knob_left_or_right_replaces_its_continuous_turn() {
        let mut c = config();
        let note = c.set_binding(0, "knob1.cw", &Binding::new(ActionKind::Shortcut, "right")).unwrap();
        assert!(note.unwrap().contains("Knob 1"));
        assert_eq!(c.binding(0, "knob1.turn").kind, ActionKind::Inherit);
        assert_eq!(c.binding(0, "knob1.press").kind, ActionKind::Kdenlive);
        c.set_binding(0, "knob1.cw", &Binding::new(ActionKind::Inherit, "")).unwrap();
        c.set_binding(0, "knob1.press", &Binding::new(ActionKind::Inherit, "")).unwrap();
        let profile = &c.doc.get("profiles").and_then(Json::as_array).unwrap()[0];
        assert!(profile.get("bindings").unwrap().get("knob1").is_none(), "empty knob objects are removed");
    }

    #[test]
    fn rejects_invalid_values_without_changing_the_config() {
        let mut c = config();
        let before = c.render();
        for bad in [
            Binding::new(ActionKind::Shortcut, "hyper+x"),
            Binding::new(ActionKind::Shortcut, "ctrl+nosuchkey"),
            Binding::new(ActionKind::Shortcut, "  "),
            Binding::new(ActionKind::Command, "echo 'open"),
            Binding::new(ActionKind::Media, "eject"),
            Binding::new(ActionKind::Mouse, "laser"),
            Binding::new(ActionKind::Advanced, "{}"),
        ] {
            assert!(c.set_binding(1, "key4", &bad).is_err(), "{bad:?}");
        }
        assert_eq!(c.render(), before);
        assert_eq!(normalize_shortcut("ctrl+k   x").unwrap(), "ctrl+k x");
        assert!(normalize_shortcut("super").is_ok());
        assert!(normalize_shortcut("F24").is_ok());
        assert!(normalize_shortcut("#183").is_ok());
    }

    #[test]
    fn app_profiles_go_before_global_and_global_is_protected() {
        let mut c = config();
        let i = c.add_app_profile("Firefox", "firefox").unwrap();
        assert_eq!(i, 1);
        assert_eq!(c.profiles()[1].class, "^firefox$");
        assert!(c.profiles()[2].global);
        assert!(c.add_app_profile("Firefox", "firefox").is_err());
        let err = c.add_app_profile("kdenlive", "org.kde.kdenlive").unwrap_err();
        assert!(err.contains("kdenlive profile already matches"), "{err}");
        assert!(pattern_covers(r"^org\.kde\.kdenlive", "org.kde.kdenlive"));
        assert!(!pattern_covers(r"^org\.kde\.kdenlive$", "org.kde.kdenlive2"));
        assert!(!pattern_covers("^(fl64|fl)$", "fl"));
        assert_eq!(class_regex("org.kde.dolphin"), r"^org\.kde\.dolphin$");
        assert!(c.remove_profile(2).is_err());
        c.set_profile_class(1, "^(firefox|librewolf)$").unwrap();
        assert_eq!(c.profiles()[1].class, "^(firefox|librewolf)$");
        c.set_profile_flag(1, "kdenlive", true).unwrap();
        assert!(c.profiles()[1].kdenlive);
        c.set_profile_flag(1, "kdenlive", false).unwrap();
        assert!(!c.render().contains(r#""kdenlive": false"#));
        c.remove_profile(1).unwrap();
        assert_eq!(c.profiles().len(), 2);
    }

    #[test]
    fn kdenlive_preset_replaces_the_existing_profile() {
        let mut c = KeypadConfig::parse(DEFAULT_CONFIG).unwrap();
        let preset = kdenlive_preset(&config().render()).unwrap();
        assert_eq!(c.apply_kdenlive_preset(preset.clone()), 0);
        assert_eq!(c.profiles().len(), 2);
        assert_eq!(c.apply_kdenlive_preset(preset), 0);
        assert_eq!(c.profiles().len(), 2);
    }

    #[test]
    fn layout_round_trips_ids_and_custom_grids() {
        let mut c = config();
        assert_eq!(c.layout(), Layout::Auto);
        c.set_layout(&Layout::Board("generic-12k2e".into())).unwrap();
        assert_eq!(c.layout(), Layout::Board("generic-12k2e".into()));
        assert!(c.render().contains(r#""layout": "generic-12k2e""#));
        let custom = Layout::Custom { keys: 9, knobs: 1, columns: 3 };
        c.set_layout(&custom).unwrap();
        let reread = KeypadConfig::parse(&c.render()).unwrap();
        assert_eq!(reread.layout(), custom);
        let layout = reread.doc.get("layout").unwrap();
        assert_eq!(json::to_compact(layout), r#"{ "keys": 9, "knobs": 1, "columns": 3 }"#);
        for bad in [
            Layout::Custom { keys: 0, knobs: 1, columns: 3 },
            Layout::Custom { keys: 17, knobs: 0, columns: 4 },
            Layout::Custom { keys: 4, knobs: 4, columns: 4 },
            Layout::Custom { keys: 4, knobs: 0, columns: 9 },
            Layout::Board(String::new()),
        ] {
            assert!(c.set_layout(&bad).is_err(), "{bad:?}");
        }
        c.set_layout(&Layout::Auto).unwrap();
        assert!(!c.render().contains("\"layout\""));
    }

    #[test]
    fn commands_split_and_join_like_a_shell() {
        let argv = split_command(r#"notify-send "a \"b\"" 'c d' e\ f"#).unwrap();
        assert_eq!(argv, ["notify-send", "a \"b\"", "c d", "e f"]);
        assert_eq!(split_command(&join_command(&argv)).unwrap(), argv);
        assert!(split_command("   ").is_err());
    }

    #[test]
    fn labels() {
        assert_eq!(slot_label("key7"), "Key 7");
        assert_eq!(slot_label("knob2.cw"), "Knob 2 · Turn right");
        assert_eq!(Binding::new(ActionKind::Media, "playpause").summary(), "Play / pause");
    }
}
