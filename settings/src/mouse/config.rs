//! The Mouse tab's assignments: one set of profiles per device, kept in
//! `~/.config/smplos/mouse.json`.
//!
//! ```text
//! { "version": 1,
//!   "devices": {
//!     "mmo-12": { "profiles": [
//!       { "name": "Browser", "match": { "class": "^firefox$" },
//!         "bindings": { "btn-back": "alt+left" } },
//!       { "name": "global", "bindings": { "grid-1": "ctrl+c" } } ] } } }
//! ```
//!
//! A device holds the same profile list the keypad daemon reads (app profiles
//! first, `global` last, unset buttons falling through to Global unless
//! `"fallthrough": false`), and its buttons are flat slots named by the input
//! ids of its device profile. Whatever Settings doesn't know about (other
//! devices, other keys) is written back as it was.

use crate::assign::json::{self, Json};
use crate::assign::profile::Profiles;

/// The version of the file layout this Settings reads and writes.
const VERSION: f64 = 1.0;

/// What a device without any assignments yet reads as.
static NOTHING: Json = Json::Obj(Vec::new());

/// The entry `select_device` makes for a device the file doesn't list.
const BLANK_DEVICE: &str = r#"{ "profiles": [{ "name": "global", "bindings": {} }] }"#;

/// The file's document and the device whose profiles are being edited. No
/// device is selected until `select_device`: choose one before editing.
#[derive(Clone, Debug)]
pub struct MouseConfig {
    doc: Json,
    device: String,
    /// Devices `select_device` had to add. The text leaves them out while
    /// they hold nothing, so only looking at a mouse doesn't put it in the file.
    added: Vec<String>,
}

impl MouseConfig {
    /// A file that doesn't exist yet.
    pub fn empty() -> Self {
        let doc = Json::Obj(vec![("version".into(), Json::Num("1".into())), ("devices".into(), Json::obj())]);
        MouseConfig { doc, device: String::new(), added: Vec::new() }
    }

    /// Reads the file's text. A file this Settings can't safely edit (not an
    /// object, a newer version) is refused so it is never overwritten.
    pub fn parse(text: &str) -> Result<Self, String> {
        let doc = json::parse(text)?;
        if !matches!(doc, Json::Obj(_)) {
            return Err("it isn't a JSON object".into());
        }
        match doc.get("version") {
            None => {}
            Some(Json::Num(version)) => match version.parse::<f64>() {
                Ok(number) if number == VERSION => {}
                Ok(number) if number > VERSION => {
                    return Err(format!("it is version {version}, newer than this Settings understands"));
                }
                _ => return Err(format!("\"version\" must be {VERSION}, not {version}")),
            },
            Some(_) => return Err("\"version\" must be a number".into()),
        }
        match doc.get("devices") {
            None => {}
            Some(Json::Obj(devices)) => {
                if let Some((id, _)) = devices.iter().find(|(_, entry)| !matches!(entry, Json::Obj(_))) {
                    return Err(format!("\"devices\" > \"{id}\" must be an object"));
                }
            }
            Some(_) => return Err("\"devices\" must be an object".into()),
        }
        Ok(MouseConfig { doc, device: String::new(), added: Vec::new() })
    }

    /// Switches to a device's profiles; returns the index of its Global
    /// profile, which is created (in memory) when the device has none yet.
    pub fn select_device(&mut self, device: &str) -> usize {
        let listed = self.doc.get("devices").and_then(|devices| devices.get(device)).is_some();
        if !listed && !self.added.iter().any(|id| id == device) {
            self.added.push(device.to_string());
        }
        self.device = device.to_string();
        self.ensure_global()
    }

    /// Whether the file gives `device` an app profile or a button. Reads only:
    /// it doesn't create the device's Global profile.
    pub fn has_assignments(&self, device: &str) -> bool {
        let profiles = self
            .doc
            .get("devices")
            .and_then(|devices| devices.get(device))
            .and_then(|entry| entry.get("profiles"))
            .and_then(Json::as_array);
        profiles.is_some_and(|list| {
            list.iter().any(|profile| {
                profile.get("match").is_some() || profile.get("bindings").is_some_and(|b| !b.is_empty_container())
            })
        })
    }

    /// The file's text: pretty JSON, ending in a newline.
    pub fn render(&self) -> String {
        let blank = json::parse(BLANK_DEVICE).expect("a valid constant");
        let mut doc = self.doc.clone();
        if let Some(Json::Obj(devices)) = doc.get_mut("devices") {
            devices.retain(|(id, entry)| !(self.added.contains(id) && *entry == blank));
        }
        json::to_pretty(&doc)
    }
}

impl Profiles for MouseConfig {
    fn doc(&self) -> &Json {
        self.doc.get("devices").and_then(|devices| devices.get(&self.device)).unwrap_or(&NOTHING)
    }

    fn doc_mut(&mut self) -> &mut Json {
        debug_assert!(!self.device.is_empty(), "select a device before editing");
        self.doc.object_mut("devices").object_mut(&self.device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assign::action::{ActionKind, Binding};

    fn shortcut(text: &str) -> Binding {
        Binding::new(ActionKind::Shortcut, text)
    }

    fn names(config: &MouseConfig) -> Vec<String> {
        config.profiles().into_iter().map(|p| p.name).collect()
    }

    /// A new file with `device` selected.
    fn empty_for(device: &str) -> MouseConfig {
        let mut config = MouseConfig::empty();
        config.select_device(device);
        config
    }

    /// `text` read, with `device` selected.
    fn parsed(text: &str, device: &str) -> MouseConfig {
        let mut config = MouseConfig::parse(text).expect("valid");
        config.select_device(device);
        config
    }

    #[test]
    fn a_new_file_holds_an_empty_global_profile_for_the_device() {
        let mut config = empty_for("mmo-12");
        assert_eq!(names(&config), ["global"]);
        assert_eq!(config.set_flat_binding(0, "grid-1", &shortcut("ctrl+c")), Ok(()));
        assert_eq!(
            config.render(),
            "{\n    \"version\": 1,\n    \"devices\": {\n        \"mmo-12\": { \"profiles\": [{ \"name\": \"global\", \"bindings\": { \"grid-1\": \"ctrl+c\" } }] }\n    }\n}\n"
        );
    }

    #[test]
    fn looking_at_a_device_leaves_nothing_in_the_text() {
        let mut config = empty_for("mmo-12");
        assert_eq!(config.select_device("generic"), 0);
        assert_eq!(names(&config), ["global"], "the editor still has a profile to show");
        assert_eq!(config.render(), "{\n    \"version\": 1,\n    \"devices\": {}\n}\n");

        config.set_flat_binding(0, "btn-back", &shortcut("alt+left")).expect("set");
        let text = config.render();
        assert!(text.contains("\"generic\"") && !text.contains("mmo-12"), "{text}");

        config.set_flat_binding(0, "btn-back", &Binding::new(ActionKind::Inherit, "")).expect("unset");
        assert!(!config.render().contains("generic"), "back to nothing, back out of the file");
    }

    #[test]
    fn an_entry_that_was_in_the_file_is_never_dropped() {
        let text = r#"{"devices":{"generic":{"profiles":[{"name":"global","bindings":{}}]}}}"#;
        let config = parsed(text, "generic");
        assert!(config.render().contains("\"generic\""));
    }

    #[test]
    fn reading_a_file_invents_nothing_until_a_device_is_selected() {
        let text = r#"{"devices":{"mmo-12":{"profiles":[{"name":"global","bindings":{"grid-1":"ctrl+c"}}]}}}"#;
        let config = MouseConfig::parse(text).expect("valid");
        assert!(config.profiles().is_empty());
        assert_eq!(json::to_compact(&json::parse(&config.render()).unwrap()), json::to_compact(&json::parse(text).unwrap()));

        let mut config = config;
        assert_eq!(config.select_device("generic"), 0, "a device the file lacks gets a Global profile");
        assert_eq!(config.profiles().len(), 1);
        assert!(config.render().contains("mmo-12"));
    }

    #[test]
    fn it_tells_which_devices_have_assignments() {
        let text = r#"{"devices":{
            "generic":{"profiles":[{"name":"global","bindings":{}}]},
            "mmo-12":{"profiles":[{"name":"global","bindings":{"grid-1":"ctrl+c"}}]},
            "redragon":{"profiles":[{"name":"Browser","match":{"class":"^firefox$"},"bindings":{}},{"name":"global","bindings":{}}]},
            "broken":{"profiles":"nope"}}}"#;
        let config = MouseConfig::parse(text).expect("valid");
        assert!(!config.has_assignments("generic"), "an empty Global profile isn't an assignment");
        assert!(config.has_assignments("mmo-12"), "a button");
        assert!(config.has_assignments("redragon"), "an app profile");
        assert!(!config.has_assignments("broken"));
        assert!(!config.has_assignments("never-seen"));
        assert!(!MouseConfig::empty().has_assignments("generic"));
    }

    #[test]
    fn buttons_are_assigned_per_device() {
        let mut config = empty_for("generic");
        assert_eq!(config.set_flat_binding(0, "btn-back", &shortcut("alt+left")), Ok(()));

        assert_eq!(config.select_device("mmo-12"), 0);
        assert_eq!(config.flat_binding(0, "btn-back").kind, ActionKind::Inherit, "another device starts blank");
        assert_eq!(config.set_flat_binding(0, "grid-7", &shortcut("ctrl+c")), Ok(()));

        config.select_device("generic");
        assert_eq!(config.flat_binding(0, "btn-back").value, "alt+left");
        assert_eq!(config.flat_binding(0, "grid-7").kind, ActionKind::Inherit);
    }

    #[test]
    fn global_and_app_assignments_survive_a_round_trip() {
        let mut config = empty_for("mmo-12");
        assert_eq!(config.set_flat_binding(0, "grid-7", &shortcut("ctrl+c")), Ok(()));
        let browser = config.add_app_profile("Browser", "firefox").expect("a new app profile");
        assert_eq!(browser, 0, "app profiles come before Global");
        assert_eq!(config.set_flat_binding(browser, "btn-back", &shortcut("alt+left")), Ok(()));
        assert_eq!(config.set_flat_binding(browser, "grid-7", &Binding::new(ActionKind::Disabled, "")), Ok(()));

        let loaded = parsed(&config.render(), "mmo-12");
        assert_eq!(names(&loaded), ["Browser", "global"]);
        assert_eq!(loaded.profiles()[0].class, "^firefox$");
        assert_eq!(loaded.flat_binding(0, "btn-back").value, "alt+left");
        assert_eq!(loaded.flat_binding(0, "grid-7").kind, ActionKind::Disabled);
        assert_eq!(loaded.flat_binding(1, "grid-7").value, "ctrl+c");
        assert_eq!(loaded.flat_binding(1, "btn-back").kind, ActionKind::Inherit);
        assert_eq!(loaded.flat_bindings(0).len(), 2);
    }

    #[test]
    fn setting_a_button_back_to_not_set_removes_it() {
        let mut config = empty_for("generic");
        config.set_flat_binding(0, "btn-middle", &shortcut("ctrl+v")).expect("set");
        assert!(config.render().contains("btn-middle"));
        config.set_flat_binding(0, "btn-middle", &Binding::new(ActionKind::Inherit, "")).expect("unset");
        assert!(!config.render().contains("btn-middle"));
        assert!(config.flat_bindings(0).is_empty());
    }

    #[test]
    fn what_cannot_be_stored_is_refused_and_changes_nothing() {
        let mut config = empty_for("generic");
        let before = config.render();
        assert!(config.set_flat_binding(0, "btn-back", &Binding::new(ActionKind::Media, "not-a-key")).is_err());
        assert!(config.set_flat_binding(0, "btn-back", &Binding::new(ActionKind::Mouse, "side")).is_err());
        assert_eq!(config.set_flat_binding(5, "btn-back", &shortcut("alt+left")), Err("no such profile".into()));
        assert_eq!(config.render(), before);
    }

    #[test]
    fn an_advanced_binding_is_shown_but_never_overwritten() {
        let text = r#"{"devices":{"generic":{"profiles":[
            {"name":"global","bindings":{"btn-back":{"macro":["a","b"]}}}]}}}"#;
        let mut config = parsed(text, "generic");
        let shown = config.flat_binding(0, "btn-back");
        assert_eq!(shown.kind, ActionKind::Advanced);
        assert!(shown.value.contains("macro"));

        assert!(config.set_flat_binding(0, "btn-back", &shown).is_err(), "Settings can't write it back");
        assert!(config.render().contains(r#""macro": ["a", "b"]"#));
        assert_eq!(config.set_flat_binding(0, "btn-back", &Binding::new(ActionKind::Inherit, "")), Ok(()));
        assert!(!config.render().contains("macro"), "clearing it is an explicit choice");
    }

    #[test]
    fn everything_settings_does_not_edit_is_kept() {
        let text = r#"{
            // written by hand
            "version": 1,
            "note": "mine",
            "devices": {
                "redragon-x": { "profiles": [{ "name": "global", "bindings": { "thumb-1": "ctrl+z" } }], "extra": 3 },
                "mmo-12": { "dpi": 1600, "profiles": [
                    { "name": "Browser", "match": { "class": "^firefox$" }, "fallthrough": false,
                      "bindings": { "btn-back": "alt+left" }, "custom": true },
                    { "name": "global", "bindings": {} } ] }
            }
        }"#;
        let mut config = parsed(text, "mmo-12");
        assert_eq!(config.set_flat_binding(1, "grid-1", &shortcut("ctrl+c")), Ok(()));

        let saved = json::parse(&config.render()).expect("still JSON");
        assert_eq!(saved.get("note"), Some(&Json::str("mine")));
        let devices = saved.get("devices").expect("devices");
        assert_eq!(
            json::to_compact(devices.get("redragon-x").expect("the other device")),
            r#"{ "profiles": [{ "name": "global", "bindings": { "thumb-1": "ctrl+z" } }], "extra": 3 }"#
        );
        let mmo = devices.get("mmo-12").expect("this device");
        assert_eq!(mmo.get("dpi"), Some(&Json::Num("1600".into())));
        let browser = &mmo.get("profiles").and_then(Json::as_array).expect("profiles")[0];
        assert_eq!(browser.get("custom"), Some(&Json::Bool(true)));
        assert_eq!(browser.get("fallthrough"), Some(&Json::Bool(false)));
        assert!(!config.profiles()[0].fallthrough);
        assert_eq!(config.flat_binding(1, "grid-1").value, "ctrl+c");
    }

    #[test]
    fn a_device_without_assignments_reads_as_unassigned() {
        let mut config = MouseConfig::parse(r#"{"devices":{}}"#).expect("valid");
        assert_eq!(config.select_device("never-seen"), 0);
        assert_eq!(config.flat_binding(0, "btn-left").kind, ActionKind::Inherit);
        assert!(config.profiles()[0].global);
    }

    #[test]
    fn a_file_that_cannot_be_edited_safely_is_refused() {
        let cases = [
            ("[]", "isn't a JSON object"),
            ("{ nope", ""),
            (r#"{"version": 2}"#, "newer than this Settings"),
            (r#"{"version": 0}"#, "must be 1"),
            (r#"{"version": "1"}"#, "must be a number"),
            (r#"{"devices": []}"#, "\"devices\" must be an object"),
            (r#"{"devices": {"generic": 5}}"#, "\"generic\" must be an object"),
        ];
        for (text, reason) in cases {
            let error = MouseConfig::parse(text).expect_err(text);
            assert!(error.contains(reason), "expected \"{reason}\" in \"{error}\" for {text}");
        }
        assert!(MouseConfig::parse("{}").is_ok(), "an empty object is a fresh file");
        assert!(MouseConfig::parse(r#"{"version": 1.0}"#).is_ok());
    }
}
