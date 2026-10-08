//! The control-surface daemon's config, as Settings edits it.
//!
//! Settings edits per-profile base bindings in the simple forms (shortcut,
//! media key, mouse, command, Kdenlive action, cheatsheet, disabled), each
//! with an optional cheatsheet label, the profile header (app match,
//! Kdenlive API plugin), the layout and the cheatsheet options. Everything
//! else (layers, modes, continuous controls, cycles, requests, hardware,
//! settings) is preserved verbatim and shown read-only as "Advanced".

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
    /// The binding's `"icon"`: empty for the keypad app's automatic icon,
    /// `ICON_NONE` for none, else an icon name (see `keypad::icons`).
    pub icon: String,
    /// `"ifInstalled"`: programs or desktop ids; while one is missing the
    /// keypad app skips the binding (the slot falls through).
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

/// Where the cheatsheet overlay appears (the daemon's `position` values).
pub const SHEET_POSITIONS: [&str; 9] = [
    "top-left", "top", "top-right", "left", "center", "right", "bottom-left", "bottom", "bottom-right",
];

/// smplOS's click-through overlay window (eww.yuck); `cheatsheet.eww.window`
/// picks it instead of the unit's default `pad-cheatsheet`.
pub const SHEET_PASSTHROUGH_WINDOW: &str = "pad-cheatsheet-passthrough";

/// What the daemon uses for options the config leaves out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SheetDefaults {
    pub opacity: f64,
    pub auto_hide_ms: u32,
}

impl Default for SheetDefaults {
    /// smplOS's: a see-through background, hidden after 8 s without input.
    fn default() -> Self {
        Self { opacity: 0.35, auto_hide_ms: 8000 }
    }
}

/// `device.input`: how the keypad app reads the keypad. Unset means "auto".
pub const INPUT_MODES: [(&str, &str); 3] =
    [("auto", "Automatic"), ("evdev", "Keymap (compatible)"), ("raw", "Raw (fastest, firmware 2.0.2+)")];

/// The first open firmware whose raw mode is reliable (2.0.1 dropped out of
/// raw mode every 256 ms; the keypad app refuses raw input before 2.0.2).
pub const RAW_MIN_FIRMWARE: [u32; 3] = [2, 0, 2];

/// One engine setting under `"settings"`, edited in Settings > Keypad >
/// Advanced. Defaults are the keypad app's built-in values (control-surface
/// `config.h`, `struct Settings`). Ranges are its `SetOption` ranges for the
/// three it sets in place (accelFactor, accelWindowMs, keyRateHz); the
/// Kdenlive ones keep to sensible values (it bounds gestureIdleMs to 50..590).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub default: f64,
    pub unit: &'static str,
    /// Only matters to the Kdenlive API plugin.
    pub kdenlive: bool,
}

pub const TUNINGS: [Tuning; 6] = [
    Tuning {
        key: "accelFactor",
        label: "Knob acceleration",
        help: "How much farther a fast turn moves continuous controls (Kdenlive jog, zoom, trim). 1 = off. Key and volume bindings stay one step per detent.",
        min: 1.0, max: 10.0, step: 0.5, default: 1.0, unit: "×", kdenlive: false,
    },
    Tuning {
        key: "accelWindowMs",
        label: "Fast turn",
        help: "Detents closer together than this count as a fast turn.",
        min: 5.0, max: 200.0, step: 5.0, default: 40.0, unit: " ms", kdenlive: false,
    },
    Tuning {
        key: "keyRateHz",
        label: "Knob key rate",
        help: "Most key presses per second a knob sends (queued taps beyond 48 are dropped; reversing drops the rest).",
        min: 10.0, max: 1000.0, step: 10.0, default: 120.0, unit: "/s", kdenlive: false,
    },
    Tuning {
        key: "coalesceMs",
        label: "Update spacing",
        help: "Kdenlive: at most one update per control this often; faster turns are summed, never lost.",
        min: 1.0, max: 100.0, step: 1.0, default: 8.0, unit: " ms", kdenlive: true,
    },
    Tuning {
        key: "ackTimeoutMs",
        label: "Answer timeout",
        help: "Kdenlive: stop waiting for an update's acknowledgment after this long.",
        min: 10.0, max: 1000.0, step: 10.0, default: 60.0, unit: " ms", kdenlive: true,
    },
    Tuning {
        key: "gestureIdleMs",
        label: "Edit gesture",
        help: "Kdenlive: a pause this long ends one turning gesture, which is one undo step.",
        min: 50.0, max: 590.0, step: 10.0, default: 500.0, unit: " ms", kdenlive: true,
    },
];

/// `"1"`, `"2.5"`: a number token without needless decimals.
pub fn number(v: f64) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Version strings ("2.0.2", "2.0") to compare with `RAW_MIN_FIRMWARE`.
pub fn version_at_least(version: &str, min: [u32; 3]) -> Option<bool> {
    let parts: Vec<u32> = version.split('.').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    if parts.len() < 3 {
        return None;
    }
    Some(parts[..3] >= min[..])
}

/// Slots that can carry a cheatsheet binding (the daemon refuses turns).
pub fn sheet_slot(slot: &str) -> bool {
    !slot.contains('.') || slot.ends_with(".press")
}

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
fn merge_extras(old: Option<&Json>, new: Json) -> Json {
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

// ── Profiles ─────────────────────────────────────────────────────────────────

/// A layer that applies while pad controls are held down
/// (`"when": {"held": "key1" | ["key1", "key13"] | "key1+knob3"}`, control-
/// surface's held layers). "held" is a condition like any other: layers apply
/// in list order, the first matching one that binds an input wins.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldLayer {
    /// Index in the profile's `"layers"`.
    pub index: usize,
    pub name: String,
    /// Alternatives, each a set of controls held together (`knobN` = its press).
    pub held: Vec<Vec<String>>,
    /// Other `"when"` conditions it also needs (e.g. Kdenlive's focus), by name.
    pub conditions: Vec<String>,
}

impl HeldLayer {
    /// The one control it needs held, when that is all it needs.
    pub fn single(&self) -> Option<&str> {
        match self.held.as_slice() {
            [set] if set.len() == 1 && self.conditions.is_empty() => Some(&set[0]),
            _ => None,
        }
    }

    /// Every control it names.
    pub fn controls(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in self.held.iter().flatten() {
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
        out
    }

    /// "While holding key 1", "… key 1 or key 13", "… key 1 + knob 3 (pressed)".
    pub fn title(&self) -> String {
        let name = |c: &str| {
            if c.starts_with("knob") {
                format!("{} (pressed)", slot_label(c).to_lowercase())
            } else {
                slot_label(c).to_lowercase()
            }
        };
        let sets: Vec<String> = self.held.iter().map(|set| set.iter().map(|c| name(c)).collect::<Vec<_>>().join(" + ")).collect();
        let when = if self.conditions.is_empty() { String::new() } else { format!(", when {}", self.conditions.join(", ")) };
        format!("While holding {}{when}", sets.join(" or "))
    }
}

/// `key1`..`key16` or `knob1`..`knob3` (a knob held means its press).
pub fn held_control(name: &str) -> bool {
    let number = |rest: &str, max: usize| {
        !rest.starts_with('0') && rest.chars().all(|c| c.is_ascii_digit()) && rest.parse::<usize>().is_ok_and(|n| (1..=max).contains(&n))
    };
    name.strip_prefix("key").is_some_and(|r| number(r, 16)) || name.strip_prefix("knob").is_some_and(|r| number(r, 3))
}

/// The alternatives of a `"held"` value, or `None` if it isn't one.
fn parse_held(v: &Json) -> Option<Vec<Vec<String>>> {
    let alternatives: Vec<&str> = match v {
        Json::Str(s) => vec![s.as_str()],
        Json::Arr(items) if !items.is_empty() => items.iter().map(Json::as_str).collect::<Option<_>>()?,
        _ => return None,
    };
    alternatives
        .into_iter()
        .map(|a| {
            let mut set: Vec<String> = Vec::new();
            for c in a.split('+').map(str::trim) {
                if !held_control(c) {
                    return None;
                }
                if !set.iter().any(|s| s == c) {
                    set.push(c.to_string());
                }
            }
            Some(set)
        })
        .collect()
}

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

#[derive(Clone, Debug, PartialEq)]
pub struct SheetOptions {
    pub opacity: f64,
    /// 0: shown until hidden.
    pub auto_hide_ms: u32,
    pub position: String,
    /// Clicks go through the overlay (SHEET_PASSTHROUGH_WINDOW), so it can't
    /// be clicked away and must hide by itself.
    pub click_through: bool,
    /// The keypad app draws the overlay through the bar (`cheatsheet.eww`
    /// not false / not `{"enabled": false}`).
    pub overlay: bool,
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

/// The config's `"layout"`: unset (the daemon decides), a board profile id,
/// or a custom grid.
#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Auto,
    Board(String),
    Custom { keys: usize, knobs: usize, columns: usize },
}

/// The daemon's default when a custom layout omits "columns" (config.cpp).
pub fn default_layout_columns(keys: usize) -> usize {
    if keys >= 10 {
        5
    } else {
        keys.clamp(1, 4)
    }
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

    #[cfg(test)]
    fn raw_binding(&self, profile: usize, slot: &str) -> Option<&Json> {
        self.raw_binding_at(profile, None, slot)
    }

    fn raw_binding_at(&self, profile: usize, layer: Option<usize>, slot: &str) -> Option<&Json> {
        let bindings = self.bindings_of(profile, layer)?;
        if let Some(flat) = bindings.get(slot) {
            return Some(flat);
        }
        let (knob, event) = slot.split_once('.')?;
        bindings.get(knob)?.get(event)
    }

    #[cfg(test)]
    pub fn binding(&self, profile: usize, slot: &str) -> Binding {
        self.binding_at(profile, None, slot)
    }

    /// A binding of a profile's base bindings (`layer: None`) or of a layer.
    pub fn binding_at(&self, profile: usize, layer: Option<usize>, slot: &str) -> Binding {
        let Some(bindings) = self.bindings_of(profile, layer) else {
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
    #[cfg(test)]
    pub fn bound_slots(&self, profile: usize) -> Vec<(String, Binding)> {
        self.bound_slots_at(profile, None)
    }

    /// Bindings of a profile (`layer: None`) or a layer, flattened, in file order.
    pub fn bound_slots_at(&self, profile: usize, layer: Option<usize>) -> Vec<(String, Binding)> {
        let mut out = Vec::new();
        let Some(bindings) = self.bindings_of(profile, layer) else {
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

    #[cfg(test)]
    pub fn set_binding(&mut self, profile: usize, slot: &str, binding: &Binding) -> Result<Option<String>, String> {
        self.set_binding_at(profile, None, slot, binding)
    }

    /// Sets a key ("key3") or knob event ("knob1.cw") binding of a profile
    /// (`layer: None`) or of one of its layers. Returns a note when another
    /// binding had to change for this one to take effect. A held layer can't
    /// map the control held for it.
    pub fn set_binding_at(&mut self, profile: usize, layer: Option<usize>, slot: &str, binding: &Binding) -> Result<Option<String>, String> {
        if binding.kind == ActionKind::Cheatsheet && !sheet_slot(slot) {
            return Err("the cheatsheet can be shown by a key or a knob press, not a turn".into());
        }
        if let Some(held) = layer.and_then(|i| self.held_layers(profile).into_iter().find(|h| h.index == i)) {
            let control = slot.split('.').next().unwrap_or(slot);
            if binding.kind != ActionKind::Inherit && held.controls().iter().any(|c| c == control) && !slot.contains(".cw") && !slot.contains(".ccw") {
                return Err(format!("{} is the control you hold for this layer", slot_label(control)));
            }
        }
        let value = to_json(binding)?;
        let old = self.raw_binding_at(profile, layer, slot).cloned();
        let value = value.map(|v| merge_extras(old.as_ref(), v));
        let bindings = self.bindings_mut(profile, layer)?;
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
            Some(o @ Json::Obj(_)) => {
                let keys = num(o, "keys").unwrap_or(0);
                Layout::Custom {
                    keys,
                    knobs: num(o, "knobs").unwrap_or(0),
                    columns: num(o, "columns").unwrap_or_else(|| default_layout_columns(keys)),
                }
            }
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
                if *keys > MAX_KEYS || *knobs > MAX_KNOBS || keys + knobs == 0 || !(1..=MAX_COLUMNS).contains(columns) {
                    return Err(format!(
                        "a custom layout needs 0-{MAX_KEYS} keys, 0-{MAX_KNOBS} knobs (at least one input) and 1-{MAX_COLUMNS} columns"
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

    /// The options in effect: the config's, else the daemon's `defaults`.
    pub fn sheet_options(&self, defaults: &SheetDefaults) -> SheetOptions {
        let o = self.doc.get("cheatsheet");
        let num = |k: &str| match o.and_then(|o| o.get(k)) {
            Some(Json::Num(n)) => n.parse::<f64>().ok(),
            _ => None,
        };
        SheetOptions {
            opacity: num("opacity").unwrap_or(defaults.opacity),
            auto_hide_ms: num("autoHideMs").map_or(defaults.auto_hide_ms, |v| v.max(0.0) as u32),
            position: o
                .and_then(|o| o.get("position"))
                .and_then(Json::as_str)
                .unwrap_or("center")
                .to_string(),
            click_through: o
                .and_then(|o| o.get("eww"))
                .and_then(|e| e.get("window"))
                .and_then(Json::as_str)
                == Some(SHEET_PASSTHROUGH_WINDOW),
            overlay: match o.and_then(|o| o.get("eww")) {
                Some(Json::Bool(on)) => *on,
                Some(e @ Json::Obj(_)) => e.get("enabled").and_then(Json::as_bool).unwrap_or(true),
                _ => true,
            },
        }
    }

    /// Sets the cheatsheet options; anything else under "cheatsheet" is kept,
    /// including the other "eww" fields.
    pub fn set_sheet_options(&mut self, o: &SheetOptions) -> Result<(), String> {
        if !(0.05..=1.0).contains(&o.opacity) {
            return Err("opacity is 5% to 100%".into());
        }
        if o.auto_hide_ms > 600_000 {
            return Err("hide after at most 10 minutes".into());
        }
        if o.click_through && o.auto_hide_ms == 0 {
            return Err("a click-through cheatsheet can't be clicked away: choose when it hides".into());
        }
        if !SHEET_POSITIONS.contains(&o.position.as_str()) {
            return Err(format!("unknown position '{}'", o.position));
        }
        let sheet = self.doc.object_mut("cheatsheet");
        let opacity = format!("{:.2}", o.opacity);
        let opacity = opacity.trim_end_matches('0').trim_end_matches('.');
        sheet.set("opacity", Json::Num(opacity.to_string()));
        sheet.set("autoHideMs", Json::Num(o.auto_hide_ms.to_string()));
        sheet.set("position", Json::str(&o.position));
        let window = sheet.get("eww").and_then(|e| e.get("window")).and_then(Json::as_str);
        let through = window == Some(SHEET_PASSTHROUGH_WINDOW);
        if o.click_through && !through {
            // `"eww": false` stays off: {"enabled": false, "window": ...}.
            let mut eww = match sheet.get("eww") {
                Some(e @ Json::Obj(_)) => e.clone(),
                Some(Json::Bool(on)) => Json::Obj(vec![("enabled".into(), Json::Bool(*on))]),
                _ => Json::obj(),
            };
            eww.set("window", Json::str(SHEET_PASSTHROUGH_WINDOW));
            sheet.set("eww", eww);
        } else if !o.click_through && through {
            let eww = sheet.get_mut("eww").expect("has a window");
            eww.remove("window");
            if *eww == Json::obj() {
                sheet.remove("eww");
            }
        }
        let overlay = match sheet.get("eww") {
            Some(Json::Bool(on)) => *on,
            Some(e @ Json::Obj(_)) => e.get("enabled").and_then(Json::as_bool).unwrap_or(true),
            _ => true,
        };
        if o.overlay != overlay {
            match sheet.get_mut("eww") {
                Some(eww @ Json::Obj(_)) => {
                    if o.overlay {
                        eww.remove("enabled");
                    } else {
                        eww.set("enabled", Json::Bool(false));
                    }
                    if *eww == Json::obj() {
                        sheet.remove("eww");
                    }
                }
                // On is the unit's default (--eww-window); off is `false`.
                _ if o.overlay => {
                    sheet.remove("eww");
                }
                _ => sheet.set("eww", Json::Bool(false)),
            }
        }
        Ok(())
    }

    /// `device.input`, "auto" when unset.
    pub fn input_mode(&self) -> String {
        self.doc.get("device").and_then(|d| d.get("input")).and_then(Json::as_str).unwrap_or("auto").to_string()
    }

    pub fn set_input_mode(&mut self, mode: &str) -> Result<(), String> {
        if !INPUT_MODES.iter().any(|(m, _)| *m == mode) {
            return Err(format!("unknown input mode '{mode}'"));
        }
        if mode == self.input_mode() {
            return Ok(());
        }
        if mode == "auto" && self.doc.get("device").and_then(|d| d.get("input")).is_none() {
            return Ok(());
        }
        self.doc.object_mut("device").set("input", Json::str(mode));
        Ok(())
    }

    /// An engine setting's value in the config, if set.
    pub fn tuning(&self, key: &str) -> Option<f64> {
        match self.doc.get("settings").and_then(|s| s.get(key)) {
            Some(Json::Num(n)) => n.parse().ok(),
            _ => None,
        }
    }

    /// Sets an engine setting (snapped to its step and range); `None` removes it.
    pub fn set_tuning(&mut self, key: &str, value: Option<f64>) -> Result<(), String> {
        let t = TUNINGS.iter().find(|t| t.key == key).ok_or_else(|| format!("unknown setting '{key}'"))?;
        match value {
            Some(v) => {
                if !v.is_finite() {
                    return Err(format!("{} needs a number", t.label));
                }
                let v = ((v.clamp(t.min, t.max) - t.min) / t.step).round() * t.step + t.min;
                if self.tuning(key).is_some_and(|old| (old - v).abs() < 1e-9) {
                    return Ok(());
                }
                self.doc.object_mut("settings").set(key, Json::Num(number(v)));
            }
            None => {
                if let Some(settings) = self.doc.get_mut("settings") {
                    settings.remove(key);
                    if *settings == Json::obj() {
                        self.doc.remove("settings");
                    }
                }
            }
        }
        Ok(())
    }

    /// The profile's held layers ("while holding key N"), in file order.
    pub fn held_layers(&self, profile: usize) -> Vec<HeldLayer> {
        let Some(layers) = self.profile_list().get(profile).and_then(|p| p.get("layers")).and_then(Json::as_array) else {
            return Vec::new();
        };
        layers
            .iter()
            .enumerate()
            .filter_map(|(index, l)| {
                let when = l.get("when")?;
                let held = parse_held(when.get("held")?)?;
                Some(HeldLayer {
                    index,
                    name: l.get("name").and_then(Json::as_str).unwrap_or("").to_string(),
                    held,
                    conditions: when.entries().iter().map(|(k, _)| k.clone()).filter(|k| k != "held").collect(),
                })
            })
            .collect()
    }

    /// A profile's other layers (Kdenlive contexts and the like).
    pub fn context_layer_count(&self, profile: usize) -> usize {
        self.profiles().get(profile).map_or(0, |p| p.layers) - self.held_layers(profile).len()
    }

    fn layers_mut(&mut self, profile: usize) -> Result<&mut Vec<Json>, String> {
        let p = self.profile_mut(profile)?;
        if !matches!(p.get("layers"), Some(Json::Arr(_))) {
            p.set("layers", Json::Arr(Vec::new()));
        }
        match p.get_mut("layers") {
            Some(Json::Arr(layers)) => Ok(layers),
            _ => unreachable!("just set"),
        }
    }

    /// A layer name not yet used in the profile.
    fn free_layer_name(&self, profile: usize, base: &str) -> String {
        let used: Vec<String> = self
            .profile_list()
            .get(profile)
            .and_then(|p| p.get("layers"))
            .and_then(Json::as_array)
            .map(|ls| ls.iter().filter_map(|l| l.get("name").and_then(Json::as_str).map(String::from)).collect())
            .unwrap_or_default();
        (1..).map(|n| if n == 1 { base.to_string() } else { format!("{base}-{n}") }).find(|n| !used.contains(n)).expect("unbounded")
    }

    /// The held layer for exactly `control` and exactly the `extra` conditions.
    fn find_held_layer(&self, profile: usize, control: &str, extra: &[(String, Json)]) -> Option<usize> {
        let layers = self.profile_list().get(profile)?.get("layers")?.as_array()?;
        self.held_layers(profile)
            .into_iter()
            .find(|h| {
                let when = layers[h.index].get("when");
                h.held == [vec![control.to_string()]]
                    && h.conditions.len() == extra.len()
                    && extra.iter().all(|(k, v)| when.and_then(|w| w.get(k)) == Some(v))
            })
            .map(|h| h.index)
    }

    /// Finds or adds the layer that applies while `control` is held
    /// (`extra`: further `"when"` conditions, e.g. a Kdenlive layer's).
    /// Layers apply in list order, so a new one goes before the profile's
    /// context layers, to win over them while held: one with conditions goes
    /// first of all (it is the more specific), a plain one after the held
    /// layers already at the top (a "key1+knob3" chord stays ahead of
    /// "key1"). Returns its index.
    fn held_layer_for(&mut self, profile: usize, control: &str, extra: &[(String, Json)], name: &str) -> Result<usize, String> {
        if !held_control(control) {
            return Err(format!("'{control}' is not a key or knob"));
        }
        if let Some(index) = self.find_held_layer(profile, control, extra) {
            return Ok(index);
        }
        let mut when = vec![("held".to_string(), Json::str(control))];
        when.extend(extra.iter().cloned());
        let layer = Json::Obj(vec![
            ("name".into(), Json::str(&self.free_layer_name(profile, name))),
            ("when".into(), Json::Obj(when)),
            ("bindings".into(), Json::obj()),
        ]);
        let at = if extra.is_empty() {
            let held = self.held_layers(profile);
            let layers = self.profile_list()[profile].get("layers").and_then(Json::as_array).map_or(0, Vec::len);
            // After the leading held layers.
            (0..layers).find(|i| !held.iter().any(|h| h.index == *i)).unwrap_or(layers)
        } else {
            0
        };
        self.layers_mut(profile)?.insert(at, layer);
        Ok(at)
    }

    /// Adds a "while holding `control`" layer (or finds the one there is);
    /// returns its index in `"layers"`.
    pub fn add_held_layer(&mut self, profile: usize, control: &str) -> Result<usize, String> {
        self.held_layer_for(profile, control, &[], &format!("hold-{control}"))
    }

    /// The layers of a profile, by name, in the order they apply.
    pub fn layer_names(&self, profile: usize) -> Vec<String> {
        self.profile_list()
            .get(profile)
            .and_then(|p| p.get("layers"))
            .and_then(Json::as_array)
            .map(|ls| ls.iter().enumerate().map(|(i, l)| l.get("name").and_then(Json::as_str).map_or(format!("layer {}", i + 1), String::from)).collect())
            .unwrap_or_default()
    }

    /// Moves a layer to the top of the list (layers apply in list order, so it
    /// then wins wherever it binds an input). Returns its new index (0).
    pub fn move_layer_first(&mut self, profile: usize, index: usize) -> Result<usize, String> {
        let layers = self.layers_mut(profile)?;
        if index >= layers.len() {
            return Err("no such layer".into());
        }
        let layer = layers.remove(index);
        layers.insert(0, layer);
        Ok(0)
    }

    pub fn remove_layer(&mut self, profile: usize, index: usize) -> Result<(), String> {
        let layers = self.layers_mut(profile)?;
        if index >= layers.len() {
            return Err("no such layer".into());
        }
        layers.remove(index);
        if layers.is_empty() {
            self.profile_mut(profile)?.remove("layers");
        }
        Ok(())
    }

    /// Moves a profile's "shift" (turn while pressed) bindings into "while
    /// holding `control`" layers: the profile's own into the plain held layer,
    /// a context layer's into a held layer with that layer's conditions too.
    /// A knob the target layer maps already keeps its shift binding. Returns
    /// (moved, kept) knob counts.
    pub fn convert_shift_to_held(&mut self, profile: usize, control: &str) -> Result<(usize, usize), String> {
        if !held_control(control) {
            return Err(format!("'{control}' is not a key or knob"));
        }
        struct Move {
            source: Option<usize>,
            knob: String,
            events: Vec<(String, Json)>,
            extra: Vec<(String, Json)>,
            name: String,
        }
        let held: Vec<usize> = self.held_layers(profile).iter().map(|h| h.index).collect();
        let layer_count = self.profile_list().get(profile).and_then(|p| p.get("layers")).and_then(Json::as_array).map_or(0, Vec::len);
        let mut moves: Vec<Move> = Vec::new();
        for source in std::iter::once(None).chain((0..layer_count).filter(|i| !held.contains(i)).map(Some)) {
            let (extra, name) = match source {
                None => (Vec::new(), format!("hold-{control}")),
                Some(i) => {
                    let layer = &self.profile_list()[profile].get("layers").and_then(Json::as_array).expect("counted")[i];
                    let when: Vec<(String, Json)> = layer.get("when").map(|w| w.entries().to_vec()).unwrap_or_default();
                    let name = layer.get("name").and_then(Json::as_str).unwrap_or("layer");
                    (when, format!("{name}-hold-{control}"))
                }
            };
            for (slot, value) in self.bindings_of(profile, source).map(Json::entries).unwrap_or_default() {
                let mut parts = slot.splitn(3, '.');
                let knob = parts.next().unwrap_or_default();
                let events: Vec<(String, Json)> = match (parts.next(), parts.next()) {
                    (Some("shift"), Some(event)) => vec![(event.to_string(), value.clone())],
                    (None, None) => value.get("shift").map(|s| s.entries().to_vec()).unwrap_or_default(),
                    _ => Vec::new(),
                };
                if !knob.starts_with("knob") || events.is_empty() {
                    continue;
                }
                match moves.iter_mut().find(|m| m.source == source && m.knob == knob) {
                    Some(m) => m.events.extend(events),
                    None => moves.push(Move { source, knob: knob.to_string(), events, extra: extra.clone(), name: name.clone() }),
                }
            }
        }
        // A knob the target already maps (or another move fills) stays as is.
        let mut kept = 0;
        let mut taken: Vec<(Vec<(String, Json)>, String)> = Vec::new();
        moves.retain(|m| {
            let target = self.find_held_layer(profile, control, &m.extra);
            let mapped = target.and_then(|t| self.bindings_of(profile, Some(t))).is_some_and(|b| {
                b.entries().iter().any(|(k, _)| k == &m.knob || k.starts_with(&format!("{}.", m.knob)))
            });
            let duplicate = taken.iter().any(|(e, k)| *e == m.extra && *k == m.knob);
            if mapped || duplicate {
                kept += 1;
                return false;
            }
            taken.push((m.extra.clone(), m.knob.clone()));
            true
        });
        // Take them out of their source first: layer indices are still valid.
        for m in &moves {
            if let Json::Obj(entries) = self.bindings_mut(profile, m.source)? {
                let dotted = format!("{}.shift.", m.knob);
                entries.retain(|(slot, _)| !slot.starts_with(&dotted));
                for (slot, value) in entries.iter_mut() {
                    if *slot == m.knob {
                        value.remove("shift");
                    }
                }
                entries.retain(|(slot, value)| !(*slot == m.knob && value.is_empty_container()));
            }
        }
        let moved = moves.len();
        for m in moves {
            let target = self.held_layer_for(profile, control, &m.extra, &m.name)?;
            self.bindings_mut(profile, Some(target))?.set(&m.knob, Json::Obj(m.events));
        }
        Ok((moved, kept))
    }

    /// Where a profile has "shift" (turn while pressed) bindings: "knob1" for
    /// its own bindings, "knob1 in the timeline layer" for a layer's.
    pub fn shift_bindings(&self, profile: usize) -> Vec<String> {
        let Some(p) = self.profile_list().get(profile) else { return Vec::new() };
        let mut out = Vec::new();
        let mut scan = |bindings: Option<&Json>, layer: Option<&str>| {
            for (slot, value) in bindings.map(Json::entries).unwrap_or_default() {
                let knob = slot.split('.').next().unwrap_or(slot);
                let shift = slot.split('.').nth(1) == Some("shift") || (!slot.contains('.') && value.get("shift").is_some());
                if knob.starts_with("knob") && shift {
                    let place = layer.map_or(knob.to_string(), |l| format!("{knob} in the {l} layer"));
                    if !out.contains(&place) {
                        out.push(place);
                    }
                }
            }
        };
        scan(p.get("bindings"), None);
        for layer in p.get("layers").and_then(Json::as_array).into_iter().flatten() {
            scan(layer.get("bindings"), Some(layer.get("name").and_then(Json::as_str).unwrap_or("unnamed")));
        }
        out
    }

    /// Removes a profile's "shift" bindings (its own and its layers'); returns
    /// how many knobs had them.
    pub fn remove_shift_bindings(&mut self, profile: usize) -> usize {
        let Ok(p) = self.profile_mut(profile) else { return 0 };
        let mut removed = 0;
        let mut strip = |bindings: Option<&mut Json>| {
            let Some(Json::Obj(entries)) = bindings else { return };
            let before = entries.len();
            entries.retain(|(slot, _)| !(slot.starts_with("knob") && slot.split('.').nth(1) == Some("shift")));
            removed += before - entries.len();
            for (slot, value) in entries.iter_mut() {
                if slot.starts_with("knob") && !slot.contains('.') && value.remove("shift").is_some() {
                    removed += 1;
                }
            }
        };
        strip(p.get_mut("bindings"));
        if let Some(Json::Arr(layers)) = p.get_mut("layers") {
            for layer in layers.iter_mut() {
                strip(layer.get_mut("bindings"));
            }
        }
        removed
    }

    /// Renames a profile (names show in the profile list and the cheatsheet).
    pub fn set_profile_name(&mut self, index: usize, name: &str) -> Result<(), String> {
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
    pub fn set_profile_title(&mut self, index: usize, regex: &str) -> Result<(), String> {
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
    pub fn set_profile_fallthrough(&mut self, index: usize, on: bool) -> Result<(), String> {
        let profile = self.profile_mut(index)?;
        if on {
            profile.remove("fallthrough");
        } else {
            profile.set("fallthrough", Json::Bool(false));
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
        let knobs_only = Layout::Custom { keys: 0, knobs: 2, columns: 1 };
        c.set_layout(&knobs_only).unwrap();
        assert_eq!(KeypadConfig::parse(&c.render()).unwrap().layout(), knobs_only);
        let no_columns = KeypadConfig::parse(r#"{"layout": {"keys": 12, "knobs": 2}}"#).unwrap();
        assert_eq!(no_columns.layout(), Layout::Custom { keys: 12, knobs: 2, columns: 5 }, "the daemon's default");
        let small = KeypadConfig::parse(r#"{"layout": {"keys": 3}}"#).unwrap();
        assert_eq!(small.layout(), Layout::Custom { keys: 3, knobs: 0, columns: 3 });
        for bad in [
            Layout::Custom { keys: 0, knobs: 0, columns: 3 },
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
    fn labels_round_trip_on_every_simple_form_and_short_forms_become_objects() {
        let mut c = config();
        let cases = [
            ("key4", Binding::new(ActionKind::Shortcut, "ctrl+z").labelled("Undo"), r#"{ "keys": "ctrl+z", "label": "Undo" }"#),
            ("key5", Binding::new(ActionKind::Media, "playpause").labelled("Music"), r#"{ "keys": "playpause", "label": "Music" }"#),
            ("key6", Binding::new(ActionKind::Command, "notify-send hi").labelled("Hi"), r#"{ "command": ["notify-send", "hi"], "label": "Hi" }"#),
            ("key7", Binding::new(ActionKind::Mouse, "left").labelled("Click"), r#"{ "mouse": "left", "label": "Click" }"#),
            ("key8", Binding::new(ActionKind::Cheatsheet, "hold").labelled("Help"), r#"{ "cheatsheet": "hold", "label": "Help" }"#),
        ];
        for (slot, binding, json_text) in cases {
            c.set_binding(1, slot, &binding).unwrap();
            let raw = c.raw_binding(1, slot).unwrap();
            assert_eq!(json::to_compact(raw), json_text);
            assert_eq!(c.binding(1, slot), binding, "{slot}");
        }
        // Without a label the short forms stay short.
        c.set_binding(1, "key4", &Binding::new(ActionKind::Shortcut, "ctrl+z")).unwrap();
        assert_eq!(c.raw_binding(1, "key4"), Some(&Json::str("ctrl+z")));
        assert!(to_json(&Binding::new(ActionKind::Shortcut, "a").labelled(&"x".repeat(41))).is_err());
    }

    #[test]
    fn a_label_edit_keeps_the_fallback_but_a_new_action_drops_it() {
        let mut c = config();
        c.set_binding(0, "key1", &Binding::new(ActionKind::Kdenlive, "mark_in").labelled("In")).unwrap();
        assert_eq!(
            json::to_compact(c.raw_binding(0, "key1").unwrap()),
            r#"{ "action": "mark_in", "fallback": "i", "label": "In" }"#,
            "the file's keys stay in place; new ones go last"
        );
        // The fallback "i" types mark_in's shortcut: it must not follow a new action.
        c.set_binding(0, "key1", &Binding::new(ActionKind::Kdenlive, "mark_out").labelled("Out")).unwrap();
        assert_eq!(json::to_compact(c.raw_binding(0, "key1").unwrap()), r#"{ "action": "mark_out", "label": "Out" }"#);
        c.set_binding(0, "key1", &Binding::new(ActionKind::Shortcut, "x")).unwrap();
        assert_eq!(c.raw_binding(0, "key1"), Some(&Json::str("x")), "a different form starts clean");
    }

    #[test]
    fn cheatsheet_bindings_only_on_keys_and_knob_presses() {
        let mut c = config();
        let sheet = Binding::new(ActionKind::Cheatsheet, "toggle");
        c.set_binding(1, "key9", &sheet).unwrap();
        c.set_binding(1, "knob2.press", &sheet).unwrap();
        assert!(c.set_binding(1, "knob2.cw", &sheet).is_err());
        assert!(c.set_binding(1, "key9", &Binding::new(ActionKind::Cheatsheet, "blink")).is_err());
        assert_eq!(c.binding(1, "key9"), sheet);
        assert_eq!(classify(Some(&json::parse(r#"{"cheatsheet":"hold","x":1}"#).unwrap())).kind, ActionKind::Advanced);
    }

    #[test]
    fn sheet_options_round_trip_and_keep_the_eww_block() {
        let d = SheetDefaults::default();
        let mut c = KeypadConfig::parse(r#"{"cheatsheet": {"eww": {"window": "pad-cheatsheet"}}, "profiles": []}"#).unwrap();
        let unset = SheetOptions { opacity: 0.35, auto_hide_ms: 8000, position: "center".into(), click_through: false, overlay: true };
        assert_eq!(c.sheet_options(&d), unset, "unset options are the daemon's defaults");
        let old_daemon = SheetDefaults { opacity: 0.85, auto_hide_ms: 0 };
        assert_eq!(c.sheet_options(&old_daemon), SheetOptions { opacity: 0.85, auto_hide_ms: 0, ..unset.clone() });
        let o = SheetOptions { opacity: 0.6, auto_hide_ms: 5000, position: "top-right".into(), click_through: false, overlay: true };
        c.set_sheet_options(&o).unwrap();
        assert_eq!(c.sheet_options(&d), o);
        let out = c.render();
        assert!(out.contains(r#""opacity": 0.6"#) && out.contains(r#""eww": { "window": "pad-cheatsheet" }"#), "{out}");
        for bad in [
            SheetOptions { opacity: 0.01, ..o.clone() },
            SheetOptions { auto_hide_ms: 700_000, ..o.clone() },
            SheetOptions { position: "middle".into(), ..o.clone() },
            SheetOptions { click_through: true, auto_hide_ms: 0, ..o.clone() },
        ] {
            assert!(c.set_sheet_options(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn hand_edited_configs_round_trip_unchanged() {
        let text = include_str!("testdata/live-like.jsonc");
        let original = KeypadConfig::parse(text).unwrap();
        let mut c = KeypadConfig::parse(text).unwrap();
        let (kdenlive, brave, global) = (0, 1, 2);
        let b = |kind, value: &str, label: &str| Binding::new(kind, value).labelled(label);
        assert_eq!(c.binding(global, "key1"), b(ActionKind::Cheatsheet, "hold", "Cheatsheet"));
        assert_eq!(c.binding(kdenlive, "key1"), b(ActionKind::Cheatsheet, "hold", "Cheatsheet"));
        assert_eq!(c.binding(global, "key2"), b(ActionKind::Command, "gtk-launch grafium", "Grafium"));
        assert_eq!(c.binding(global, "key3"), b(ActionKind::Command, "focus-or-launch brave-browser brave", "Brave"));
        assert_eq!(c.binding(global, "key8"), b(ActionKind::Command, "smplos-settings", "Settings").with_icon("settings"));
        assert_eq!(c.binding(global, "key12"), b(ActionKind::Media, "playpause", "Play/Pause"));
        assert_eq!(c.binding(global, "key13"), Binding::new(ActionKind::Media, "nextsong"));
        assert_eq!(c.binding(global, "knob1.cw"), b(ActionKind::Media, "volumeup", "Volume up"));
        assert_eq!(c.binding(global, "knob2.ccw"), b(ActionKind::Mouse, "wheel-up", "Scroll up"));
        assert_eq!(c.binding(global, "knob2.press"), b(ActionKind::Mouse, "middle", "Middle click").with_icon(ICON_NONE));
        assert_eq!(c.binding(brave, "knob2.press"), b(ActionKind::Shortcut, "ctrl+shift+t", "Reopen tab"));
        assert_eq!(c.binding(kdenlive, "key2"), Binding::new(ActionKind::Kdenlive, "mark_in"));

        // Applying every binding Settings can edit, unchanged, changes nothing.
        let mut applied = 0;
        for p in 0..c.profiles().len() {
            for (slot, binding) in c.bound_slots(p) {
                if binding.kind != ActionKind::Advanced {
                    c.set_binding(p, &slot, &binding).unwrap_or_else(|e| panic!("{slot}: {e}"));
                    applied += 1;
                }
            }
        }
        assert!(applied >= 20, "{applied}");
        assert_eq!(c.doc, original.doc);

        // An icon comes and goes without touching the rest (a Kdenlive fallback included).
        for (p, slot) in [(kdenlive, "key2"), (kdenlive, "knob1.press"), (global, "key13"), (global, "knob1.cw"),
                          (global, "key3"), (brave, "knob2.ccw"), (global, "key1")] {
            let before = c.binding(p, slot);
            c.set_binding(p, slot, &before.clone().with_icon("star")).unwrap();
            assert_eq!(c.binding(p, slot), before.clone().with_icon("star"), "{slot}");
            c.set_binding(p, slot, &before).unwrap();
        }
        assert_eq!(c.doc, original.doc);
        assert_eq!(KeypadConfig::parse(&c.render()).unwrap().doc, original.doc, "the saved file reads back the same");
    }

    #[test]
    fn if_installed_round_trips_in_its_own_form() {
        let text = r#"{"profiles": [{"name": "global", "bindings": {
            "key2": { "command": ["gtk-launch", "grafium"], "ifInstalled": ["gtk-launch", "grafium"], "label": "Grafium" },
            "key6": { "command": ["terminal"], "ifInstalled": "terminal", "label": "Terminal" },
            "key7": { "command": ["xdg-open", "~"], "ifInstalled": ["xdg-open"], "label": "Files", "icon": "folder" },
            "key9": { "keys": "ctrl+z", "ifInstalled": "has space" }
        }}]}"#;
        let original = KeypadConfig::parse(text).unwrap();
        let mut c = KeypadConfig::parse(text).unwrap();
        let grafium = c.binding(0, "key2");
        assert_eq!((grafium.kind, grafium.needs.clone(), grafium.label.as_str()),
                   (ActionKind::Command, vec!["gtk-launch".to_string(), "grafium".to_string()], "Grafium"));
        assert_eq!(c.binding(0, "key6").needs, ["terminal"]);
        assert!(c.binding(0, "key7").needs_list, "a one-name list stays a list");
        assert_eq!(c.binding(0, "key9").kind, ActionKind::Advanced, "the daemon refuses names with spaces");
        for slot in ["key2", "key6", "key7"] {
            let b = c.binding(0, slot);
            c.set_binding(0, slot, &b).unwrap();
        }
        assert_eq!(c.doc, original.doc, "same keys in the same order");
        let edited = c.binding(0, "key6").needing("terminal st");
        c.set_binding(0, "key6", &edited).unwrap();
        assert_eq!(json::to_compact(c.doc.get("profiles").unwrap().as_array().unwrap()[0].get("bindings").unwrap().get("key6").unwrap()),
                   r#"{ "command": ["terminal"], "ifInstalled": ["terminal", "st"], "label": "Terminal" }"#);
        let cleared = c.binding(0, "key6").needing("");
        c.set_binding(0, "key6", &cleared).unwrap();
        assert!(c.binding(0, "key6").needs.is_empty());
        let shortcut = Binding::new(ActionKind::Shortcut, "ctrl+z").needing("gimp");
        assert_eq!(json::to_compact(&to_json(&shortcut).unwrap().unwrap()), r#"{ "keys": "ctrl+z", "ifInstalled": "gimp" }"#);
    }

    #[test]
    fn input_mode_is_device_input() {
        let mut c = KeypadConfig::parse(r#"{"profiles": []}"#).unwrap();
        assert_eq!(c.input_mode(), "auto");
        c.set_input_mode("auto").unwrap();
        assert!(c.doc.get("device").is_none(), "automatic needs no key");
        c.set_input_mode("raw").unwrap();
        assert_eq!(json::to_compact(c.doc.get("device").unwrap()), r#"{ "input": "raw" }"#);
        c.set_input_mode("auto").unwrap();
        assert_eq!(c.input_mode(), "auto");
        assert!(c.set_input_mode("fast").is_err());
        let mut live = KeypadConfig::parse(r#"{"device": { "vendor": "1189", "product": "8890", "serial": "", "input": "auto" }, "profiles": []}"#).unwrap();
        live.set_input_mode("evdev").unwrap();
        assert_eq!(json::to_compact(live.doc.get("device").unwrap()),
                   r#"{ "vendor": "1189", "product": "8890", "serial": "", "input": "evdev" }"#, "the rest of device is kept, in place");
    }

    #[test]
    fn engine_tuning_snaps_to_its_range_and_reset_removes_it() {
        let text = r#"{"settings": { "coalesceMs": 8, "ackTimeoutMs": 60, "accelWindowMs": 35, "accelFactor": 3, "keyRateHz": 120 }, "profiles": []}"#;
        let mut c = KeypadConfig::parse(text).unwrap();
        assert_eq!(c.tuning("accelFactor"), Some(3.0));
        assert_eq!(c.tuning("gestureIdleMs"), None);
        c.set_tuning("accelFactor", Some(3.0)).unwrap();
        assert_eq!(c.doc, KeypadConfig::parse(text).unwrap().doc, "an unchanged value isn't rewritten");
        c.set_tuning("accelFactor", Some(2.6)).unwrap();
        assert_eq!(c.tuning("accelFactor"), Some(2.5));
        c.set_tuning("keyRateHz", Some(9999.0)).unwrap();
        assert_eq!(c.tuning("keyRateHz"), Some(1000.0));
        c.set_tuning("gestureIdleMs", Some(10.0)).unwrap();
        assert_eq!(c.tuning("gestureIdleMs"), Some(50.0), "the daemon's own bound");
        assert!(c.set_tuning("speed", Some(1.0)).is_err());
        let out = c.render();
        assert!(out.contains(r#""accelFactor": 2.5"#) && out.contains(r#""keyRateHz": 1000"#), "{out}");
        for t in TUNINGS {
            c.set_tuning(t.key, None).unwrap();
            assert!(t.min <= t.default && t.default <= t.max, "{}", t.key);
        }
        assert!(c.doc.get("settings").is_none(), "an empty settings block goes away");
    }

    #[test]
    fn profile_name_title_and_fallthrough() {
        let mut c = KeypadConfig::parse(r#"{"profiles": [
            {"name": "brave", "match": {"class": "^brave-browser$"}, "bindings": {}},
            {"name": "global", "bindings": {}}]}"#).unwrap();
        let p = &c.profiles()[0];
        assert_eq!((p.title.as_str(), p.fallthrough), ("", true));
        c.set_profile_title(0, " - YouTube ").unwrap();
        c.set_profile_fallthrough(0, false).unwrap();
        c.set_profile_name(0, "Brave video").unwrap();
        let p = &c.profiles()[0];
        assert_eq!((p.name.as_str(), p.title.as_str(), p.fallthrough), ("Brave video", "- YouTube", false));
        assert_eq!(json::to_compact(c.doc.get("profiles").unwrap().as_array().unwrap()[0].get("match").unwrap()),
                   r#"{ "class": "^brave-browser$", "title": "- YouTube" }"#);
        c.set_profile_title(0, "").unwrap();
        c.set_profile_fallthrough(0, true).unwrap();
        let brave = &c.doc.get("profiles").unwrap().as_array().unwrap()[0];
        assert!(brave.get("fallthrough").is_none() && brave.get("match").unwrap().get("title").is_none());
        assert!(c.set_profile_name(0, "global").is_err(), "names stay unique");
        assert!(c.set_profile_name(0, " ").is_err());
        assert!(c.set_profile_title(1, "x").is_err(), "Global matches every window");
    }

    #[test]
    fn the_overlay_switch_keeps_the_other_eww_fields() {
        let d = SheetDefaults::default();
        let eww = |c: &KeypadConfig| c.doc.get("cheatsheet").and_then(|s| s.get("eww")).map(json::to_compact);
        let set = |text: &str, change: &dyn Fn(&mut SheetOptions)| {
            let mut c = KeypadConfig::parse(text).unwrap();
            let mut o = c.sheet_options(&d);
            change(&mut o);
            c.set_sheet_options(&o).unwrap();
            assert_eq!(c.sheet_options(&d), o);
            c
        };
        let plain = r#"{"profiles": []}"#;
        assert!(KeypadConfig::parse(plain).unwrap().sheet_options(&d).overlay);
        let off = set(plain, &|o| o.overlay = false);
        assert_eq!(eww(&off).as_deref(), Some("false"));
        assert_eq!(eww(&set(&off.render(), &|o| o.overlay = true)), None, "on is the unit's default");
        let through = r#"{"cheatsheet": {"eww": {"window": "pad-cheatsheet-passthrough"}}, "profiles": []}"#;
        let off = set(through, &|o| o.overlay = false);
        assert_eq!(eww(&off).as_deref(), Some(r#"{ "window": "pad-cheatsheet-passthrough", "enabled": false }"#));
        assert_eq!(eww(&set(&off.render(), &|o| o.overlay = true)).as_deref(), Some(r#"{ "window": "pad-cheatsheet-passthrough" }"#));
        let both = set(plain, &|o| {
            o.overlay = false;
            o.click_through = true;
        });
        assert!(!both.sheet_options(&d).overlay && both.sheet_options(&d).click_through);
    }

    #[test]
    fn shift_bindings_are_found_and_removed_in_profiles_and_layers() {
        let text = r#"{"profiles": [{"name": "kdenlive", "match": {"class": "^org\\.kde\\.kdenlive"},
            "layers": [{"name": "timeline", "when": {"focus": "timeline"}, "bindings": {
                "knob1": {"turn": {"control": "playhead.jog"}, "shift": {"turn": {"control": "timeline.scroll"}}, "press": "space"}}},
                       {"name": "wheels", "bindings": {"knob2.shift.turn": {"control": "colorwheel.nudge"}, "key6": "x"}}],
            "bindings": {"knob3": {"ccw": "left", "cw": "right", "shift": {"ccw": "up", "cw": "down"}}, "key1": "a"}},
            {"name": "global", "bindings": {"knob1": {"ccw": "volumedown"}}}]}"#;
        let mut c = KeypadConfig::parse(text).unwrap();
        assert_eq!(c.shift_bindings(0), ["knob3", "knob1 in the timeline layer", "knob2 in the wheels layer"]);
        assert!(c.shift_bindings(1).is_empty());
        assert_eq!(c.remove_shift_bindings(0), 3);
        assert!(c.shift_bindings(0).is_empty());
        let out = c.render();
        assert!(out.contains(r#""turn": { "control": "playhead.jog" }"#) && out.contains(r#""press": "space""#), "the rest stays: {out}");
        assert!(out.contains(r#""key6": "x""#) && out.contains(r#""ccw": "left""#));
        assert_eq!(c.remove_shift_bindings(0), 0);
    }

    #[test]
    fn tuning_ranges_match_the_keypad_apps_set_option() {
        // control-surface c266f02 features.options (settings.*).
        for (key, min, max) in [("accelFactor", 1.0, 10.0), ("accelWindowMs", 5.0, 200.0), ("keyRateHz", 10.0, 1000.0)] {
            let t = TUNINGS.iter().find(|t| t.key == key).unwrap();
            assert_eq!((t.min, t.max), (min, max), "{key}");
            assert!(((t.max - t.min) / t.step).fract() == 0.0, "{key}: the range is whole steps");
        }
    }

    #[test]
    fn held_layers_are_read_in_every_form() {
        let c = KeypadConfig::parse(r#"{"profiles": [{"name": "global", "layers": [
            {"name": "hold-key1", "when": {"held": "key1"}, "bindings": {"knob1": {"ccw": "left"}}},
            {"name": "either", "when": {"held": ["key1", "key13"]}, "bindings": {}},
            {"name": "chord", "when": {"held": "key1+knob3", "focus": "timeline"}, "bindings": {}},
            {"name": "timeline", "when": {"focus": "timeline"}, "bindings": {}},
            {"name": "bad", "when": {"held": "key99"}, "bindings": {}}],
            "bindings": {}}]}"#).unwrap();
        let held = c.held_layers(0);
        assert_eq!(held.iter().map(|h| h.index).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(held[0].single(), Some("key1"));
        assert_eq!(held[0].title(), "While holding key 1");
        assert_eq!(held[1].title(), "While holding key 1 or key 13");
        assert_eq!(held[1].single(), None);
        assert_eq!(held[2].title(), "While holding key 1 + knob 3 (pressed), when focus");
        assert_eq!(held[2].controls(), ["key1", "knob3"]);
        assert_eq!(c.context_layer_count(0), 2, "timeline and the unreadable one");
        assert_eq!(c.binding_at(0, Some(0), "knob1.ccw"), Binding::new(ActionKind::Shortcut, "left"));
        assert_eq!(c.binding_at(0, None, "knob1.ccw").kind, ActionKind::Inherit);
        for (name, ok) in [("key1", true), ("key16", true), ("key17", false), ("key0", false), ("key01", false), ("knob3", true), ("knob4", false), ("knob", false), ("", false)] {
            assert_eq!(held_control(name), ok, "{name}");
        }
    }

    #[test]
    fn a_held_layer_is_added_once_and_edited_like_the_base() {
        let mut c = KeypadConfig::parse(r#"{"profiles": [{"name": "global",
            "layers": [{"name": "timeline", "when": {"focus": "timeline"}, "bindings": {}}],
            "bindings": {"key1": {"cheatsheet": "hold"}, "knob1": {"ccw": "volumedown"}}}]}"#).unwrap();
        let i = c.add_held_layer(0, "key1").unwrap();
        assert_eq!(i, 0, "before the context layers");
        assert_eq!(c.add_held_layer(0, "key1").unwrap(), i, "found, not added twice");
        c.set_binding_at(0, Some(i), "knob1.ccw", &Binding::new(ActionKind::Media, "previoussong").labelled("Previous")).unwrap();
        c.set_binding_at(0, Some(i), "key2", &Binding::new(ActionKind::Shortcut, "ctrl+c")).unwrap();
        assert!(c.set_binding_at(0, Some(i), "key1", &Binding::new(ActionKind::Shortcut, "x")).is_err(), "the held key itself");
        assert_eq!(c.binding(0, "knob1.ccw").value, "volumedown", "the base is untouched");
        assert_eq!(c.bound_slots_at(0, Some(i)).len(), 2);
        let layer = json::to_compact(&c.doc.get("profiles").unwrap().as_array().unwrap()[0].get("layers").unwrap().as_array().unwrap()[0]);
        assert_eq!(layer, r#"{ "name": "hold-key1", "when": { "held": "key1" }, "bindings": { "knob1": { "ccw": { "keys": "previoussong", "label": "Previous" } }, "key2": "ctrl+c" } }"#);
        let j = c.add_held_layer(0, "key13").unwrap();
        assert_eq!(j, 1, "after the other held layers");
        assert!(c.add_held_layer(0, "key99").is_err());
        c.remove_layer(0, j).unwrap();
        c.remove_layer(0, i).unwrap();
        assert_eq!(c.held_layers(0).len(), 0);
        assert_eq!(c.context_layer_count(0), 1);
    }

    #[test]
    fn held_layers_go_before_context_layers_and_can_move_first() {
        // Layers apply in list order ("held" is a condition like any other).
        let mut c = KeypadConfig::parse(r#"{"profiles": [{"name": "global", "layers": [
            {"name": "chord", "when": {"held": "key1+knob3"}, "bindings": {}},
            {"name": "workspace", "when": {"$mode.ws": "1"}, "bindings": {}},
            {"name": "hub", "when": {"held": "key15"}, "bindings": {}}],
            "bindings": {}}]}"#).unwrap();
        let i = c.add_held_layer(0, "key1").unwrap();
        assert_eq!(c.layer_names(0), ["chord", "hold-key1", "workspace", "hub"], "after the chord, before the context layer");
        assert_eq!(i, 1);
        assert_eq!(c.add_held_layer(0, "key15").unwrap(), 3, "a hand-placed held layer stays where it is");
        assert_eq!(c.move_layer_first(0, 3).unwrap(), 0);
        assert_eq!(c.layer_names(0), ["hub", "chord", "hold-key1", "workspace"]);
        assert!(c.move_layer_first(0, 9).is_err());
    }

    #[test]
    fn shift_bindings_convert_to_held_layers() {
        let text = r#"{"profiles": [{"name": "kdenlive", "match": {"class": "^org\\.kde\\.kdenlive"},
            "layers": [
                {"name": "timeline", "when": {"focus": "timeline"}, "bindings": {
                    "knob1": {"turn": {"control": "playhead.jog"}, "shift": {"turn": {"control": "timeline.scroll"}}, "press": "space"}}},
                {"name": "wheels", "when": {"colorWheels": true}, "bindings": {"knob2.shift.turn": {"control": "colorwheel.nudge", "options": {"step": "fine"}}}}],
            "bindings": {"knob3": {"ccw": "left", "cw": "right", "shift": {"ccw": "up", "cw": "down"}}, "key1": {"cheatsheet": "hold"}}},
            {"name": "global", "bindings": {}}]}"#;
        let mut c = KeypadConfig::parse(text).unwrap();
        assert_eq!(c.convert_shift_to_held(0, "key1").unwrap(), (3, 0));
        assert!(c.shift_bindings(0).is_empty());
        let held = c.held_layers(0);
        assert_eq!(held.len(), 3);
        let by_name = |n: &str| held.iter().find(|h| h.name == n).cloned().unwrap();
        let plain = by_name("hold-key1");
        assert_eq!(plain.single(), Some("key1"));
        assert_eq!(c.binding_at(0, Some(plain.index), "knob3.cw"), Binding::new(ActionKind::Shortcut, "down"));
        let timeline = by_name("timeline-hold-key1");
        assert_eq!(timeline.conditions, ["focus"]);
        assert!(timeline.index < plain.index, "the one with conditions wins, so it goes first");
        let out = c.render();
        assert!(out.contains(r#""when": { "held": "key1", "focus": "timeline" }"#), "{out}");
        assert!(out.contains(r#""knob1": { "turn": { "control": "timeline.scroll" } }"#), "{out}");
        assert!(out.contains(r#""knob2": { "turn": { "control": "colorwheel.nudge", "options": { "step": "fine" } } }"#), "{out}");
        assert!(out.contains(r#""press": "space""#) && out.contains(r#""ccw": "left""#), "the rest stays");
        assert!(!out.contains("shift"), "{out}");
        assert_eq!(c.convert_shift_to_held(0, "key1").unwrap(), (0, 0), "nothing left");

        // A knob the held layer maps already keeps its shift binding.
        let mut c = KeypadConfig::parse(r#"{"profiles": [{"name": "global",
            "layers": [{"name": "mine", "when": {"held": "key2"}, "bindings": {"knob1": {"ccw": "a"}}}],
            "bindings": {"knob1": {"shift": {"cw": "b"}}, "knob2": {"shift": {"cw": "c"}}}}]}"#).unwrap();
        assert_eq!(c.convert_shift_to_held(0, "key2").unwrap(), (1, 1));
        assert_eq!(c.shift_bindings(0), ["knob1"]);
        assert_eq!(c.binding_at(0, Some(0), "knob2.cw"), Binding::new(ActionKind::Shortcut, "c"));
        assert!(c.convert_shift_to_held(0, "key0").is_err());
    }

    #[test]
    fn firmware_versions_compare_for_raw_mode() {
        assert_eq!(version_at_least("2.0.2", RAW_MIN_FIRMWARE), Some(true));
        assert_eq!(version_at_least("2.1.0", RAW_MIN_FIRMWARE), Some(true));
        assert_eq!(version_at_least("2.0.1", RAW_MIN_FIRMWARE), Some(false));
        assert_eq!(version_at_least("2.0", RAW_MIN_FIRMWARE), None, "bcdDevice alone can't tell");
        assert_eq!(version_at_least("", RAW_MIN_FIRMWARE), None);
    }

    #[test]
    fn icons_are_written_beside_the_label() {
        let json = |b: &Binding| json::to_compact(&to_json(b).unwrap().unwrap());
        let media = Binding::new(ActionKind::Media, "playpause");
        assert_eq!(json(&media), r#""playpause""#);
        assert_eq!(json(&media.clone().with_icon("player-play")), r#"{ "keys": "playpause", "icon": "player-play" }"#);
        let undo = Binding::new(ActionKind::Shortcut, "ctrl+z").labelled("Undo").with_icon(ICON_NONE);
        assert_eq!(json(&undo), r#"{ "keys": "ctrl+z", "label": "Undo", "icon": "none" }"#);
        assert!(to_json(&media.clone().with_icon("Bad Name")).is_err());
        // A hand-written icon Settings can't read keeps the binding out of the editor.
        let odd = Json::Obj(vec![("keys".into(), Json::str("a")), ("icon".into(), Json::Num("3".into()))]);
        assert_eq!(classify(Some(&odd)).kind, ActionKind::Advanced);
        for (name, ok) in [("volume-3", true), ("brand-github", true), ("none", true), ("", false), ("-x", false), ("a--b", false), ("Vol", false)] {
            assert_eq!(valid_icon(name), ok, "{name}");
        }
    }

    #[test]
    fn click_through_picks_the_passthrough_window_and_keeps_other_eww_fields() {
        let d = SheetDefaults::default();
        let eww = |c: &KeypadConfig| c.doc.get("cheatsheet").and_then(|s| s.get("eww")).cloned();
        let through = |c: &KeypadConfig, on: bool| {
            let mut c = KeypadConfig { doc: c.doc.clone() };
            let o = SheetOptions { click_through: on, ..c.sheet_options(&d) };
            c.set_sheet_options(&o).unwrap();
            assert_eq!(c.sheet_options(&d).click_through, on);
            c
        };
        let window = Json::str(SHEET_PASSTHROUGH_WINDOW);

        let plain = KeypadConfig::parse(r#"{"profiles": []}"#).unwrap();
        let on = through(&plain, true);
        assert_eq!(eww(&on), Some(Json::Obj(vec![("window".into(), window.clone())])));
        assert_eq!(eww(&through(&on, false)), None, "back to the unit's window");

        let off = KeypadConfig::parse(r#"{"cheatsheet": {"eww": false}, "profiles": []}"#).unwrap();
        let on = through(&off, true);
        assert_eq!(eww(&on), Some(Json::Obj(vec![("enabled".into(), Json::Bool(false)), ("window".into(), window.clone())])));
        assert_eq!(eww(&through(&on, false)), Some(Json::Obj(vec![("enabled".into(), Json::Bool(false))])));

        let custom = KeypadConfig::parse(r#"{"cheatsheet": {"eww": {"binary": "/opt/eww", "window": "pad-cheatsheet"}}, "profiles": []}"#).unwrap();
        let on = through(&custom, true);
        assert_eq!(eww(&on), Some(Json::Obj(vec![("binary".into(), Json::str("/opt/eww")), ("window".into(), window)])));
        assert_eq!(eww(&through(&on, false)), Some(Json::Obj(vec![("binary".into(), Json::str("/opt/eww"))])));
    }

    #[test]
    fn example_classes_for_previews() {
        assert_eq!(example_class(r"^org\.kde\.kdenlive"), "org.kde.kdenlive");
        assert_eq!(example_class("^(fl64\\.exe|fl\\.exe)$"), "fl64.exe");
        assert_eq!(example_class("^firefox$"), "firefox");
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
