//! Mouse device profiles: a mouse's physical inputs, as data.
//!
//! A profile says which inputs a mouse has (an id, a label, where it sits on
//! the drawing, which kinds of action it accepts) so the Mouse tab can draw it
//! and offer the right choices without knowing any mouse in particular. Adding
//! a mouse means adding a JSON file, schema version 1:
//!
//! ```text
//! { "version": 1, "id": "mmo-12", "name": "…", "description": "…",
//!   "preview": true,                       // a mock-up until captured
//!   "match": [{ "vendor": "1532", "product": "0067", "name": "Naga" }],
//!   "aspect": 1.6,                         // drawing width / height
//!   "actions": ["inherit", "shortcut"],    // what inputs accept by default
//!   "panels": [{ "label": "Side grid", "box": [x, y, w, h] }],
//!   "inputs": [{ "id": "grid-1", "label": "Grid button 1", "short": "1",
//!                "box": [x, y, w, h],      // percent of the drawing
//!                "actions": ["inherit"],   // optional override
//!                "code": "KEY_1", "source": "keyboard", "note": "…" }] }
//! ```
//!
//! An input's `id` is the slot its assignments are stored under in
//! `mouse.json`, so ids must stay stable. `code` (the evdev name) and `source`
//! (which interface reports it) are filled in once a button has been captured.
//! Unknown keys are errors, so a typo is found when the file is loaded.

use crate::assign::action::ActionKind;
use crate::assign::json::{self, Json};

/// The files compiled in; a user folder of device files is a follow-up.
const BUILTIN: [(&str, &str); 2] = [
    ("generic.json", include_str!("devices/generic.json")),
    ("mmo-12.json", include_str!("devices/mmo-12.json")),
];

/// The action names a device file may use.
const KINDS: [(&str, ActionKind); 7] = [
    ("inherit", ActionKind::Inherit),
    ("disabled", ActionKind::Disabled),
    ("shortcut", ActionKind::Shortcut),
    ("media", ActionKind::Media),
    ("mouse", ActionKind::Mouse),
    ("command", ActionKind::Command),
    ("kdenlive", ActionKind::Kdenlive),
];

/// Evdev names of the clicks a mouse is unusable without.
const PRIMARY_CLICKS: [&str; 2] = ["BTN_LEFT", "BTN_RIGHT"];

/// How far past the edge of the drawing a box may round to.
const EDGE: f32 = 0.001;

/// A box on the drawing, in percent of its width and height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Which interface of the device reports an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Pointer,
    Keyboard,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Pointer => "mouse interface",
            Source::Keyboard => "keyboard interface",
        }
    }
}

/// How a device is recognised among the connected ones: every key given must
/// match.
#[derive(Clone, Debug, PartialEq)]
pub struct Matcher {
    pub vendor: Option<u16>,
    pub product: Option<u16>,
    /// A case-insensitive part of the device's name.
    pub name: Option<String>,
}

/// A labelled outline on the drawing that groups inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub label: String,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    /// The slot its assignments are stored under.
    pub id: String,
    pub label: String,
    /// What the drawing shows on the button; the label when empty.
    pub short: String,
    pub rect: Rect,
    /// Overrides the device's actions for this input.
    pub actions: Option<Vec<ActionKind>>,
    /// The evdev name, once captured.
    pub code: Option<String>,
    pub source: Option<Source>,
    pub note: String,
}

impl Input {
    pub fn caption(&self) -> &str {
        if self.short.is_empty() {
            &self.label
        } else {
            &self.short
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub description: String,
    /// A mock-up: the layout is a guess until the buttons have been captured.
    pub preview: bool,
    pub matchers: Vec<Matcher>,
    /// Drawing width divided by its height.
    pub aspect: f32,
    /// What an input accepts unless it says otherwise.
    pub actions: Vec<ActionKind>,
    pub panels: Vec<Panel>,
    pub inputs: Vec<Input>,
}

impl Device {
    pub fn input(&self, id: &str) -> Option<&Input> {
        self.inputs.iter().find(|input| input.id == id)
    }

    /// The kinds of action `input` accepts, in the order the editor lists them.
    pub fn kinds_for<'a>(&'a self, input: &'a Input) -> &'a [ActionKind] {
        input.actions.as_deref().unwrap_or(&self.actions)
    }

    pub fn title(&self) -> String {
        if self.preview {
            format!("{} (preview)", self.name)
        } else {
            self.name.clone()
        }
    }

    /// Whether a connected device (its USB ids and name) is this one.
    #[allow(dead_code)] // matching is also enforced independently by mouse-remapperd
    pub fn matches(&self, vendor: u16, product: u16, name: &str) -> bool {
        let name = name.to_lowercase();
        self.matchers.iter().any(|rule| {
            rule.vendor.is_none_or(|v| v == vendor)
                && rule.product.is_none_or(|p| p == product)
                && rule.name.as_ref().is_none_or(|part| name.contains(&part.to_lowercase()))
        })
    }
}

/// The devices Settings ships with. A broken file is reported and skipped, so
/// one bad profile can't take the tab down.
pub fn builtin() -> Vec<Device> {
    load(&BUILTIN)
}

fn load(files: &[(&str, &str)]) -> Vec<Device> {
    let mut devices: Vec<Device> = Vec::new();
    for (file, text) in files {
        match parse(text) {
            Ok(device) if devices.iter().any(|known| known.id == device.id) => {
                eprintln!("settings: mouse device {file}: \"{}\" is already defined", device.id);
            }
            Ok(device) => devices.push(device),
            Err(error) => eprintln!("settings: mouse device {file}: {error}"),
        }
    }
    devices
}

pub fn parse(text: &str) -> Result<Device, String> {
    let doc = json::parse(text)?;
    let what = "the device";
    only_keys(
        &doc,
        &["version", "id", "name", "description", "preview", "match", "aspect", "actions", "panels", "inputs"],
        what,
    )?;
    if !matches!(doc.get("version"), Some(Json::Num(version)) if version == "1") {
        return Err("\"version\" must be 1".to_string());
    }
    let id = id_of(&doc, what)?;
    let name = text_of(&doc, "name", what)?;
    let preview = match doc.get("preview") {
        None => false,
        Some(Json::Bool(preview)) => *preview,
        Some(_) => return Err("the device: \"preview\" must be true or false".to_string()),
    };
    let aspect = match doc.get("aspect") {
        Some(value) => number(value, what)?,
        None => return Err("the device needs an \"aspect\" (drawing width / height)".to_string()),
    };
    if !(0.5..=4.0).contains(&aspect) {
        return Err("the device: \"aspect\" must be between 0.5 and 4".to_string());
    }
    let actions = match doc.get("actions") {
        Some(value) => kinds(value, what)?,
        None => return Err("the device needs \"actions\"".to_string()),
    };
    let matchers = items(&doc, "match", what)?
        .iter()
        .enumerate()
        .map(|(index, rule)| matcher(rule, index))
        .collect::<Result<Vec<_>, _>>()?;
    let panels = items(&doc, "panels", what)?
        .iter()
        .enumerate()
        .map(|(index, panel)| panel_of(panel, index))
        .collect::<Result<Vec<_>, _>>()?;
    let inputs = items(&doc, "inputs", what)?
        .iter()
        .enumerate()
        .map(|(index, input)| input_of(input, index))
        .collect::<Result<Vec<_>, _>>()?;
    if inputs.is_empty() {
        return Err("the device has no inputs".to_string());
    }
    for (index, input) in inputs.iter().enumerate() {
        if inputs[..index].iter().any(|earlier| earlier.id == input.id) {
            return Err(format!("input \"{}\" appears twice", input.id));
        }
        let primary = input.code.as_deref().is_some_and(|code| PRIMARY_CLICKS.contains(&code));
        let accepted = input.actions.as_ref().unwrap_or(&actions);
        if primary && accepted.iter().any(|kind| !matches!(kind, ActionKind::Inherit | ActionKind::Mouse)) {
            return Err(format!(
                "input \"{}\" is a primary click, so it may only accept \"inherit\" and \"mouse\": \
                 a wrong assignment could make the mouse unclickable",
                input.id
            ));
        }
    }
    Ok(Device {
        id,
        name,
        description: optional_text(&doc, "description", what)?,
        preview,
        matchers,
        aspect,
        actions,
        panels,
        inputs,
    })
}

fn matcher(rule: &Json, index: usize) -> Result<Matcher, String> {
    let what = format!("match {}", index + 1);
    only_keys(rule, &["vendor", "product", "name"], &what)?;
    let vendor = hex_id(rule, "vendor", &what)?;
    let product = hex_id(rule, "product", &what)?;
    let name = match rule.get("name") {
        None => None,
        Some(Json::Str(part)) if !part.trim().is_empty() => Some(part.clone()),
        Some(_) => return Err(format!("{what}: \"name\" must be non-empty text")),
    };
    if vendor.is_none() && product.is_none() && name.is_none() {
        return Err(format!("{what} needs a \"vendor\", \"product\" or \"name\""));
    }
    if product.is_some() && vendor.is_none() {
        return Err(format!("{what}: a \"product\" id only means something with its \"vendor\""));
    }
    Ok(Matcher { vendor, product, name })
}

fn panel_of(panel: &Json, index: usize) -> Result<Panel, String> {
    let what = format!("panel {}", index + 1);
    only_keys(panel, &["label", "box"], &what)?;
    Ok(Panel { label: optional_text(panel, "label", &what)?, rect: rect(panel, &what)? })
}

fn input_of(input: &Json, index: usize) -> Result<Input, String> {
    let what = format!("input {}", index + 1);
    only_keys(input, &["id", "label", "short", "box", "actions", "code", "source", "note"], &what)?;
    let id = id_of(input, &what)?;
    let what = format!("input \"{id}\"");
    let code = match input.get("code") {
        None => None,
        Some(Json::Str(code)) if valid_code(code) => Some(code.clone()),
        Some(_) => return Err(format!("{what}: \"code\" must be an evdev name like KEY_1, BTN_SIDE or REL_WHEEL")),
    };
    let source = match input.get("source") {
        None => None,
        Some(Json::Str(source)) if source == "pointer" => Some(Source::Pointer),
        Some(Json::Str(source)) if source == "keyboard" => Some(Source::Keyboard),
        Some(_) => return Err(format!("{what}: \"source\" must be \"pointer\" or \"keyboard\"")),
    };
    Ok(Input {
        id,
        label: text_of(input, "label", &what)?,
        short: optional_text(input, "short", &what)?,
        rect: rect(input, &what)?,
        actions: input.get("actions").map(|value| kinds(value, &what)).transpose()?,
        code,
        source,
        note: optional_text(input, "note", &what)?,
    })
}

fn kinds(value: &Json, what: &str) -> Result<Vec<ActionKind>, String> {
    let Json::Arr(names) = value else {
        return Err(format!("{what}: \"actions\" must be a list"));
    };
    let mut kinds = Vec::new();
    for name in names {
        let name = name.as_str().ok_or_else(|| format!("{what}: \"actions\" must hold names"))?;
        let kind = KINDS
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, kind)| *kind)
            .ok_or_else(|| format!("{what}: unknown action \"{name}\""))?;
        if kinds.contains(&kind) {
            return Err(format!("{what}: action \"{name}\" is listed twice"));
        }
        kinds.push(kind);
    }
    if !kinds.contains(&ActionKind::Inherit) {
        return Err(format!("{what}: \"actions\" must include \"inherit\", the way back to unassigned"));
    }
    Ok(kinds)
}

fn rect(object: &Json, what: &str) -> Result<Rect, String> {
    let parts = match object.get("box") {
        Some(Json::Arr(parts)) if parts.len() == 4 => parts,
        _ => return Err(format!("{what} needs \"box\": [x, y, width, height]")),
    };
    let mut values = [0.0_f32; 4];
    for (value, part) in values.iter_mut().zip(parts) {
        *value = number(part, what)?;
    }
    let [x, y, w, h] = values;
    if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 || x + w > 100.0 + EDGE || y + h > 100.0 + EDGE {
        return Err(format!("{what}: \"box\" must stay inside the drawing (percent, 0 to 100, size above 0)"));
    }
    Ok(Rect { x, y, w, h })
}

fn number(value: &Json, what: &str) -> Result<f32, String> {
    match value {
        Json::Num(text) => text
            .parse::<f32>()
            .ok()
            .filter(|number| number.is_finite())
            .ok_or_else(|| format!("{what}: \"{text}\" is not a usable number")),
        _ => Err(format!("{what}: expected a number")),
    }
}

fn hex_id(object: &Json, key: &str, what: &str) -> Result<Option<u16>, String> {
    match object.get(key) {
        None => Ok(None),
        Some(Json::Str(digits)) if digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(u16::from_str_radix(digits, 16).ok())
        }
        Some(_) => Err(format!("{what}: \"{key}\" must be four hex digits, like \"1532\"")),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_code(code: &str) -> bool {
    let Some(rest) = code.strip_prefix("KEY_").or_else(|| code.strip_prefix("BTN_")).or_else(|| code.strip_prefix("REL_")) else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn id_of(object: &Json, what: &str) -> Result<String, String> {
    match object.get("id") {
        Some(Json::Str(id)) if valid_id(id) => Ok(id.clone()),
        _ => Err(format!("{what} needs an \"id\" of a-z, 0-9 and \"-\"")),
    }
}

fn text_of(object: &Json, key: &str, what: &str) -> Result<String, String> {
    match object.get(key) {
        Some(Json::Str(text)) if !text.trim().is_empty() => Ok(text.clone()),
        _ => Err(format!("{what} needs \"{key}\" as non-empty text")),
    }
}

fn optional_text(object: &Json, key: &str, what: &str) -> Result<String, String> {
    match object.get(key) {
        None => Ok(String::new()),
        Some(Json::Str(text)) => Ok(text.clone()),
        Some(_) => Err(format!("{what}: \"{key}\" must be text")),
    }
}

fn items<'a>(object: &'a Json, key: &str, what: &str) -> Result<&'a [Json], String> {
    match object.get(key) {
        None => Ok(&[]),
        Some(Json::Arr(items)) => Ok(items),
        Some(_) => Err(format!("{what}: \"{key}\" must be a list")),
    }
}

/// Refuses keys the schema does not know, so a typo is an error and not a
/// silently ignored setting. (A key written twice never gets this far: the
/// parser keeps the last one, as the daemon's does.)
fn only_keys(object: &Json, allowed: &[&str], what: &str) -> Result<(), String> {
    let Json::Obj(entries) = object else {
        return Err(format!("{what} must be an object"));
    };
    match entries.iter().find(|(key, _)| !allowed.contains(&key.as_str())) {
        Some((key, _)) => Err(format!("{what}: unknown key \"{key}\"")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
        "version": 1, "id": "test-mouse", "name": "Test mouse", "aspect": 1.5,
        "actions": ["inherit", "shortcut", "mouse"],
        "inputs": [
            { "id": "a", "label": "A", "box": [10, 10, 20, 20] },
            { "id": "b", "label": "B", "box": [40, 10, 20, 20], "code": "KEY_1", "source": "keyboard" }
        ]
    }"#;

    /// An imagined second MMO mouse whose side keys report as keyboard keys:
    /// supporting it should take nothing but this file.
    const KEYBOARD_GRID: &str = r#"{
        "version": 1, "id": "mmo-keys", "name": "MMO mouse with side keys",
        "match": [{ "vendor": "1234", "name": "Gamer" }],
        "aspect": 1.4, "actions": ["inherit", "disabled", "shortcut", "command"],
        "panels": [{ "label": "Thumb keys", "box": [2, 20, 30, 60] }],
        "inputs": [
            { "id": "thumb-1", "label": "Thumb key 1", "short": "1", "box": [4, 30, 12, 12], "code": "KEY_1", "source": "keyboard" },
            { "id": "thumb-2", "label": "Thumb key 2", "short": "2", "box": [18, 30, 12, 12], "code": "KEY_2", "source": "keyboard" },
            { "id": "btn-left", "label": "Left button", "box": [40, 5, 25, 25], "actions": ["inherit", "mouse"], "code": "BTN_LEFT", "source": "pointer" }
        ]
    }"#;

    fn built_in(id: &str) -> Device {
        builtin().into_iter().find(|device| device.id == id).expect("a built-in device")
    }

    fn ids(device: &Device) -> Vec<&str> {
        device.inputs.iter().map(|input| input.id.as_str()).collect()
    }

    fn refused(text: &str) -> String {
        parse(text).expect_err("the file should be refused")
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    fn inside(inner: &Rect, outer: &Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.w <= outer.x + outer.w
            && inner.y + inner.h <= outer.y + outer.h
    }

    #[test]
    fn every_built_in_file_parses_and_only_supported_hardware_is_live() {
        for (file, text) in BUILTIN {
            parse(text).unwrap_or_else(|error| panic!("{file}: {error}"));
        }
        let devices = builtin();
        assert_eq!(devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["generic", "mmo-12"]);
        assert!(devices[0].preview);
        assert!(!devices[1].preview);
        assert_eq!(devices[0].title(), "Generic mouse (preview)");
    }

    #[test]
    fn the_generic_mouse_has_the_usual_buttons() {
        let device = built_in("generic");
        let mut got = ids(&device);
        got.sort_unstable();
        let mut want = vec![
            "btn-back",
            "btn-dpi",
            "btn-forward",
            "btn-left",
            "btn-middle",
            "btn-right",
            "wheel-tilt-left",
            "wheel-tilt-right",
        ];
        want.sort_unstable();
        assert_eq!(got, want);
        assert_eq!(device.input("btn-back").and_then(|i| i.code.as_deref()), Some("BTN_SIDE"));
        assert_eq!(device.input("btn-forward").and_then(|i| i.code.as_deref()), Some("BTN_EXTRA"));
        assert!(device.matchers.is_empty(), "a generic mouse is chosen by hand");
    }

    #[test]
    fn the_mmo_mouse_adds_a_twelve_button_side_grid() {
        let device = built_in("mmo-12");
        for number in 1..=12 {
            let id = format!("grid-{number}");
            let input = device.input(&id).unwrap_or_else(|| panic!("{id} is missing"));
            assert_eq!(input.short, number.to_string());
            assert_eq!(input.code, None, "grid codes are not guessed before capture");
        }
        assert_eq!(device.inputs.iter().filter(|i| i.id.starts_with("grid-")).count(), 12);
        for id in built_in("generic").inputs.iter().map(|i| i.id.as_str()) {
            assert!(device.input(id).is_some(), "{id} should also be on the MMO mouse");
        }
        let grid = &device.panels[0];
        assert_eq!(grid.label, "Side grid");
        for input in device.inputs.iter().filter(|i| i.id.starts_with("grid-")) {
            assert!(inside(&input.rect, &grid.rect), "{} leaves the Side grid panel", input.id);
        }
    }

    #[test]
    fn no_two_inputs_overlap_and_all_fit_the_drawing() {
        for device in builtin() {
            for (index, a) in device.inputs.iter().enumerate() {
                assert!(inside(&a.rect, &Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 }), "{} is off the drawing", a.id);
                assert!(a.caption().chars().count() <= 10, "{} has a caption too long for its button", a.id);
                for b in &device.inputs[index + 1..] {
                    assert!(!overlap(&a.rect, &b.rect), "{} and {} overlap on {}", a.id, b.id, device.id);
                }
            }
        }
    }

    #[test]
    fn the_primary_clicks_only_take_mouse_buttons() {
        for device in builtin() {
            for id in ["btn-left", "btn-right"] {
                let input = device.input(id).expect("a primary click");
                assert_eq!(
                    device.kinds_for(input),
                    [ActionKind::Inherit, ActionKind::Mouse],
                    "{id} on {}",
                    device.id
                );
            }
            let middle = device.input("btn-middle").expect("the wheel click");
            assert!(device.kinds_for(middle).contains(&ActionKind::Shortcut));
        }
    }

    #[test]
    fn a_primary_click_cannot_be_given_other_actions_by_a_file() {
        let click = VALID.replace(r#""code": "KEY_1", "source": "keyboard""#, r#""code": "BTN_LEFT", "source": "pointer""#);
        assert!(refused(&click).contains("primary click"));

        let restricted = click.replace(r#""code": "BTN_LEFT""#, r#""actions": ["inherit", "mouse"], "code": "BTN_LEFT""#);
        assert!(parse(&restricted).is_ok());

        let widened = restricted.replace(r#"["inherit", "mouse"]"#, r#"["inherit", "mouse", "command"]"#);
        assert!(refused(&widened).contains("primary click"));
    }

    #[test]
    fn a_minimal_file_parses_with_defaults() {
        let device = parse(VALID).expect("valid");
        assert_eq!((device.id.as_str(), device.name.as_str()), ("test-mouse", "Test mouse"));
        assert!(!device.preview && device.description.is_empty() && device.matchers.is_empty());
        assert_eq!(device.aspect, 1.5);
        assert_eq!(ids(&device), ["a", "b"]);
        let a = device.input("a").expect("input a");
        assert_eq!(a.rect, Rect { x: 10.0, y: 10.0, w: 20.0, h: 20.0 });
        assert_eq!((a.code.clone(), a.source), (None, None));
        let b = device.input("b").expect("input b");
        assert_eq!((b.code.as_deref(), b.source), (Some("KEY_1"), Some(Source::Keyboard)));
        assert_eq!(Source::Keyboard.label(), "keyboard interface");
    }

    #[test]
    fn an_input_takes_the_device_actions_unless_it_names_its_own() {
        let text = VALID.replace(r#""label": "A","#, r#""label": "A", "actions": ["inherit", "disabled"],"#);
        let device = parse(&text).expect("valid");
        let a = device.input("a").expect("input a");
        let b = device.input("b").expect("input b");
        assert_eq!(device.kinds_for(a), [ActionKind::Inherit, ActionKind::Disabled]);
        assert_eq!(device.kinds_for(b), [ActionKind::Inherit, ActionKind::Shortcut, ActionKind::Mouse]);
    }

    #[test]
    fn captions_fall_back_to_the_label() {
        let device = parse(VALID).expect("valid");
        assert_eq!(device.input("a").expect("input a").caption(), "A");
        let grid = built_in("mmo-12");
        assert_eq!(grid.input("grid-7").expect("grid 7").caption(), "7");
    }

    #[test]
    fn a_second_mmo_mouse_is_only_data() {
        let device = parse(KEYBOARD_GRID).expect("valid");
        assert_eq!(ids(&device), ["thumb-1", "thumb-2", "btn-left"]);
        let thumb = device.input("thumb-1").expect("thumb 1");
        assert_eq!((thumb.code.as_deref(), thumb.source), (Some("KEY_1"), Some(Source::Keyboard)));
        assert!(!device.kinds_for(thumb).contains(&ActionKind::Mouse));
        assert_eq!(device.kinds_for(device.input("btn-left").expect("left")), [ActionKind::Inherit, ActionKind::Mouse]);
        assert_eq!(device.panels[0].label, "Thumb keys");
        assert!(device.matches(0x1234, 0x0042, "Gamer MMO Mouse"));
    }

    #[test]
    fn devices_are_recognised_by_their_ids_and_name() {
        let naga = built_in("mmo-12");
        assert!(naga.matches(0x1532, 0x00b4, "Razer Razer Naga V2 HyperSpeed"));
        assert!(naga.matches(0x1532, 0x00b4, "razer naga v2 hyperspeed"), "names match in any case");
        assert!(!naga.matches(0x1532, 0x0067, "Razer Razer Naga Trinity"), "other Naga models are not silently supported");
        assert!(!naga.matches(0x1532, 0x0043, "Razer DeathAdder"), "the name has to match too");
        assert!(!naga.matches(0x046d, 0xc08b, "Naga clone"), "so does the vendor");
        assert!(!built_in("generic").matches(0x1532, 0x0067, "Razer Naga"), "a generic mouse is never auto-picked");

        let by_product = parse(&VALID.replace(
            r#""aspect": 1.5,"#,
            r#""aspect": 1.5, "match": [{ "vendor": "046d", "product": "c08b" }],"#,
        ))
        .expect("valid");
        assert!(by_product.matches(0x046d, 0xc08b, "anything"));
        assert!(!by_product.matches(0x046d, 0xc08c, "anything"));
    }

    #[test]
    fn files_with_mistakes_are_refused_with_a_reason() {
        let cases: Vec<(String, &str)> = vec![
            (VALID.replace(r#""version": 1"#, r#""version": 2"#), "\"version\" must be 1"),
            (VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "colour": "red","#), "unknown key \"colour\""),
            (VALID.replace(r#""aspect": 1.5,"#, ""), "needs an \"aspect\""),
            (VALID.replace(r#""aspect": 1.5"#, r#""aspect": 9"#), "between 0.5 and 4"),
            (VALID.replace(r#""id": "test-mouse""#, r#""id": "Test Mouse""#), "a-z, 0-9"),
            (VALID.replace(r#""name": "Test mouse","#, ""), "needs \"name\""),
            (VALID.replace(r#""id": "b""#, r#""id": "a""#), "input \"a\" appears twice"),
            (VALID.replace("[40, 10, 20, 20]", "[90, 10, 20, 20]"), "inside the drawing"),
            (VALID.replace("[40, 10, 20, 20]", "[40, 10, 0, 20]"), "inside the drawing"),
            (VALID.replace("[40, 10, 20, 20]", "[-1, 10, 20, 20]"), "inside the drawing"),
            (VALID.replace("[40, 10, 20, 20]", "[40, 10, 20]"), "\"box\": [x, y, width, height]"),
            (VALID.replace(r#""inherit", "shortcut""#, r#""inherit", "macro""#), "unknown action \"macro\""),
            (VALID.replace(r#""inherit", "shortcut""#, r#""inherit", "inherit""#), "listed twice"),
            (VALID.replace(r#""inherit", "#, ""), "must include \"inherit\""),
            (VALID.replace("KEY_1", "key one"), "evdev name"),
            (VALID.replace("KEY_1", "ABS_X"), "evdev name"),
            (VALID.replace(r#""keyboard""#, r#""usb""#), "\"pointer\" or \"keyboard\""),
            (VALID.replace(r#""label": "A""#, r#""label": 7"#), "needs \"label\""),
            (VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "match": [{}],"#), "needs a \"vendor\""),
            (
                VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "match": [{ "vendor": "15" }],"#),
                "four hex digits",
            ),
            (
                VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "match": [{ "vendor": "zzzz" }],"#),
                "four hex digits",
            ),
            (
                VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "match": [{ "product": "0067" }],"#),
                "only means something with its \"vendor\"",
            ),
            (VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "preview": "yes","#), "\"preview\" must be true or false"),
            (VALID.replace(r#""aspect": 1.5,"#, r#""aspect": 1.5, "panels": [{ "label": "x" }],"#), "panel 1 needs \"box\""),
            (VALID.replace(r#""id": "a","#, r#""id": "a", "colour": "red","#), "input 1: unknown key \"colour\""),
            (r#"{ "version": 1, "id": "x", "name": "X", "aspect": 1.5, "actions": ["inherit"], "inputs": [] }"#.to_string(), "no inputs"),
            ("[]".to_string(), "must be an object"),
            ("{ nope".to_string(), ""),
        ];
        for (text, reason) in cases {
            let error = refused(&text);
            assert!(error.contains(reason), "expected \"{reason}\" in \"{error}\" for:\n{text}");
        }
    }

    #[test]
    fn loading_skips_a_broken_or_repeated_file_and_keeps_the_rest() {
        let devices = load(&[
            ("one.json", VALID),
            ("broken.json", "{ nope"),
            ("again.json", VALID),
            ("two.json", KEYBOARD_GRID),
        ]);
        assert_eq!(devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["test-mouse", "mmo-keys"]);
    }
}
