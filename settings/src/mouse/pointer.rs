//! Pointer options: speed, acceleration and scrolling.
//!
//! They live in `~/.config/hypr/pointer.conf` and reach the running Hyprland
//! as they change.
//!
//! The file is hyprlang, like `monitors.conf`: `hyprland.conf` can source it
//! and a Lua-provider config can read its `input { }` block into
//! `hl.config` with a small loader. Settings manages four options in that
//! block and keeps every other line as it found it.
//!
//! The running compositor is changed with `hyprctl eval "hl.config(...)"`
//! under Lua (that provider refuses `hyprctl keyword`) and with
//! `hyprctl keyword` under hyprlang.

use crate::assign::json::{self, Json};
use crate::display::hyprctl::{hyprctl, hyprctl_ok, lua_string, parse_provider, Provider, Runner};

/// Hyprland clamps `input:sensitivity` to this range.
pub const SENSITIVITY_MIN: f64 = -1.0;
pub const SENSITIVITY_MAX: f64 = 1.0;
/// The page's scroll speed slider covers 0.25 to 4. A value read from a file
/// or from Hyprland is held to this wider range, so one written by hand
/// outside the slider still shows.
const SCROLL_FACTOR_FLOOR: f64 = 0.1;
const SCROLL_FACTOR_CEILING: f64 = 10.0;

/// The options Settings writes in the `input` block.
const MANAGED: [&str; 4] = ["sensitivity", "accel_profile", "natural_scroll", "scroll_factor"];

/// How the pointer's speed follows the hand's speed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accel {
    /// Leave it to libinput.
    Default,
    Flat,
    Adaptive,
    /// A profile Settings doesn't offer (`custom …`) that the file or
    /// Hyprland already has; kept as it is until another one is chosen.
    Custom(String),
}

impl Accel {
    pub fn label(&self) -> String {
        match self {
            Self::Default => "System default".into(),
            Self::Flat => "Flat (no acceleration)".into(),
            Self::Adaptive => "Adaptive (faster when you move faster)".into(),
            Self::Custom(_) => "Custom (set in the file)".into(),
        }
    }

    /// The `accel_profile` value; empty leaves libinput's own choice.
    pub fn option(&self) -> &str {
        match self {
            Self::Default => "",
            Self::Flat => "flat",
            Self::Adaptive => "adaptive",
            Self::Custom(value) => value,
        }
    }

    pub fn from_option(value: &str) -> Self {
        match value.trim() {
            "" | "[[EMPTY]]" => Self::Default,
            "flat" => Self::Flat,
            "adaptive" => Self::Adaptive,
            other => Self::Custom(other.to_string()),
        }
    }

    /// What the dropdown offers: the three profiles, and the current one if
    /// it is a custom profile.
    pub fn choices(current: &Accel) -> Vec<Accel> {
        let mut choices = vec![Self::Default, Self::Flat, Self::Adaptive];
        if matches!(current, Self::Custom(_)) {
            choices.push(current.clone());
        }
        choices
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pointer {
    /// `input:sensitivity`: -1 slowest, 0 as the mouse reports, 1 fastest.
    pub sensitivity: f64,
    pub accel: Accel,
    /// `input:natural_scroll`: content follows the wheel, like a touchscreen.
    pub natural_scroll: bool,
    /// `input:scroll_factor`: a multiplier on every wheel step.
    pub scroll_factor: f64,
}

impl Default for Pointer {
    fn default() -> Self {
        Self { sensitivity: 0.0, accel: Accel::Default, natural_scroll: false, scroll_factor: 1.0 }
    }
}

impl Pointer {
    /// In the ranges Hyprland accepts and rounded to the page's steps, so a
    /// slider's float noise never reaches a file.
    pub fn normalized(mut self) -> Self {
        self.sensitivity = tidy(self.sensitivity, SENSITIVITY_MIN, SENSITIVITY_MAX, 0.0);
        self.scroll_factor = tidy(self.scroll_factor, SCROLL_FACTOR_FLOOR, SCROLL_FACTOR_CEILING, 1.0);
        self
    }
}

fn tidy(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        (value.clamp(min, max) * 100.0).round() / 100.0
    } else {
        fallback
    }
}

/// A number as written in a config: two decimals at most, no trailing zeros.
pub fn number(value: f64) -> String {
    let text = format!("{value:.2}");
    let text = if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.') } else { &text };
    match text {
        "" | "-0" => "0".into(),
        text => text.into(),
    }
}

/// The speed as the page shows it.
pub fn sensitivity_text(value: f64) -> String {
    match number(value).as_str() {
        "0" => "0 (default)".into(),
        text if value > 0.0 => format!("+{text}"),
        text => text.into(),
    }
}

/// The scroll multiplier as the page shows it.
pub fn scroll_factor_text(value: f64) -> String {
    match number(value).as_str() {
        "1" => "1× (default)".into(),
        text => format!("{text}×"),
    }
}

// ── pointer.conf ─────────────────────────────────────────────────────────────

/// One line of a hyprlang file, as far as Settings needs to tell.
enum Line<'a> {
    /// `name {`
    Open(&'a str),
    /// `}`
    Close,
    /// `key = value`
    Option(&'a str, &'a str),
    Other,
}

fn classify(raw: &str) -> Line<'_> {
    let line = raw.split('#').next().unwrap_or("").trim();
    if line == "}" {
        Line::Close
    } else if let Some((key, value)) = line.split_once('=') {
        Line::Option(key.trim(), value.trim())
    } else if let Some(name) = line.strip_suffix('{') {
        Line::Open(name.trim())
    } else {
        Line::Other
    }
}

fn is_managed(key: &str) -> bool {
    MANAGED.contains(&key)
}

fn truth(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The options a file sets in its top-level `input` block over the defaults;
/// `None` when it sets none of them (a missing file, or one that is empty).
pub fn parse(text: &str) -> Option<Pointer> {
    let mut pointer = Pointer::default();
    let mut found = false;
    let mut path: Vec<&str> = Vec::new();
    for raw in text.lines() {
        match classify(raw) {
            Line::Open(name) => path.push(name),
            Line::Close => {
                path.pop();
            }
            Line::Option(key, value) if path == ["input"] && is_managed(key) => {
                found = true;
                match key {
                    "sensitivity" => pointer.sensitivity = value.parse().unwrap_or(pointer.sensitivity),
                    "accel_profile" => pointer.accel = Accel::from_option(value),
                    "natural_scroll" => pointer.natural_scroll = truth(value).unwrap_or(pointer.natural_scroll),
                    _ => pointer.scroll_factor = value.parse().unwrap_or(pointer.scroll_factor),
                }
            }
            _ => {}
        }
    }
    found.then(|| pointer.normalized())
}

const HEADER: &str = "\
# Pointer options, written by smplOS Settings (Mouse tab).
# Settings changes only the options in the input block below and keeps
# every other line. Hyprland reads the file where its config loads it.
";

fn managed_lines(pointer: &Pointer) -> String {
    let mut lines = format!("    sensitivity = {}\n", number(pointer.sensitivity));
    if pointer.accel != Accel::Default {
        lines.push_str(&format!("    accel_profile = {}\n", pointer.accel.option()));
    }
    lines.push_str(&format!("    natural_scroll = {}\n", pointer.natural_scroll));
    lines.push_str(&format!("    scroll_factor = {}\n", number(pointer.scroll_factor)));
    lines
}

/// The file's new text: `existing` with the managed options set from
/// `pointer`. Everything else in it stays. A managed option written
/// anywhere in an `input` block is dropped, so the value that counts is
/// the one Settings writes in the first block.
pub fn render(existing: Option<&str>, pointer: &Pointer) -> String {
    let existing = existing.unwrap_or("");
    let mut out = String::new();
    let mut path: Vec<&str> = Vec::new();
    let mut written = false;
    for raw in existing.lines() {
        match classify(raw) {
            Line::Open(name) => path.push(name),
            Line::Close => {
                if path == ["input"] && !written {
                    out.push_str(&managed_lines(pointer));
                    written = true;
                }
                path.pop();
            }
            Line::Option(key, _) if path == ["input"] && is_managed(key) => continue,
            _ => {}
        }
        out.push_str(raw);
        out.push('\n');
    }
    if !written {
        if existing.trim().is_empty() {
            out = HEADER.to_string();
        } else {
            out.push('\n');
        }
        out.push_str("input {\n");
        out.push_str(&managed_lines(pointer));
        out.push_str("}\n");
    }
    out
}

// ── The running Hyprland ─────────────────────────────────────────────────────

fn lua_config(pointer: &Pointer) -> String {
    format!(
        "hl.config({{ input = {{ sensitivity = {}, accel_profile = {}, natural_scroll = {}, scroll_factor = {} }} }})",
        number(pointer.sensitivity),
        lua_string(pointer.accel.option()),
        pointer.natural_scroll,
        number(pointer.scroll_factor)
    )
}

/// `hyprctl keyword` writes an empty string as `[[EMPTY]]`.
fn keyword_options(pointer: &Pointer) -> [(&'static str, String); 4] {
    let accel = match pointer.accel.option() {
        "" => "[[EMPTY]]".to_string(),
        option => option.to_string(),
    };
    [
        ("sensitivity", number(pointer.sensitivity)),
        ("accel_profile", accel),
        ("natural_scroll", pointer.natural_scroll.to_string()),
        ("scroll_factor", number(pointer.scroll_factor)),
    ]
}

/// Sets the four options on the running compositor. Only an `ok` reply counts.
pub fn apply(runner: &dyn Runner, pointer: &Pointer) -> Result<(), String> {
    match parse_provider(&hyprctl(runner, &["systeminfo"])?)? {
        Provider::Lua => hyprctl_ok(runner, &["eval", &lua_config(pointer)]),
        Provider::Legacy => keyword_options(pointer)
            .iter()
            .try_for_each(|(name, value)| hyprctl_ok(runner, &["keyword", &format!("input:{name}"), value])),
    }
}

fn live_option(runner: &dyn Runner, name: &str) -> Result<Json, String> {
    let reply = hyprctl(runner, &["getoption", &format!("input:{name}"), "-j"])?;
    json::parse(&reply).map_err(|e| format!("Hyprland's answer for input:{name} isn't readable ({e})"))
}

fn live_number(runner: &dyn Runner, name: &str) -> Result<f64, String> {
    let option = live_option(runner, name)?;
    ["float", "int"]
        .iter()
        .find_map(|kind| match option.get(kind) {
            Some(Json::Num(text)) => text.parse().ok(),
            _ => None,
        })
        .ok_or_else(|| format!("Hyprland has no number for input:{name}"))
}

fn live_bool(runner: &dyn Runner, name: &str) -> Result<bool, String> {
    let option = live_option(runner, name)?;
    match (option.get("bool"), option.get("int")) {
        (Some(Json::Bool(on)), _) => Ok(*on),
        (_, Some(Json::Num(text))) => Ok(text != "0"),
        _ => Err(format!("Hyprland has no on/off value for input:{name}")),
    }
}

/// What the running Hyprland has now (a read-only query).
pub fn read_live(runner: &dyn Runner) -> Result<Pointer, String> {
    let accel = live_option(runner, "accel_profile")?;
    Ok(Pointer {
        sensitivity: live_number(runner, "sensitivity")?,
        accel: Accel::from_option(accel.get("str").and_then(Json::as_str).unwrap_or("")),
        natural_scroll: live_bool(runner, "natural_scroll")?,
        scroll_factor: live_number(runner, "scroll_factor")?,
    }
    .normalized())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::hyprctl::tests::{failure, reply, MockRunner};

    fn pointer(sensitivity: f64, accel: Accel, natural_scroll: bool, scroll_factor: f64) -> Pointer {
        Pointer { sensitivity, accel, natural_scroll, scroll_factor }
    }

    #[test]
    fn numbers_are_written_short_and_never_as_negative_zero() {
        assert_eq!(number(0.3), "0.3");
        assert_eq!(number(1.0), "1");
        assert_eq!(number(-0.05), "-0.05");
        assert_eq!(number(10.0), "10");
        assert_eq!(number(0.0), "0");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(-0.001), "0", "rounds to zero");
        assert_eq!(number(0.1 + 0.2), "0.3", "float noise doesn't show");
    }

    #[test]
    fn values_are_held_to_what_hyprland_accepts() {
        let wild = pointer(7.0, Accel::Flat, true, 0.0).normalized();
        assert_eq!((wild.sensitivity, wild.scroll_factor), (1.0, 0.1));
        let low = pointer(-3.0, Accel::Flat, true, 99.0).normalized();
        assert_eq!((low.sensitivity, low.scroll_factor), (-1.0, 10.0));
        let noise = pointer(0.1 + 0.2, Accel::Default, false, 1.0 / 3.0).normalized();
        assert_eq!((noise.sensitivity, noise.scroll_factor), (0.3, 0.33));
        let nan = pointer(f64::NAN, Accel::Default, false, f64::INFINITY).normalized();
        assert_eq!((nan.sensitivity, nan.scroll_factor), (0.0, 1.0), "not a number: the default");
    }

    #[test]
    fn the_page_describes_the_values_in_words() {
        assert_eq!(sensitivity_text(0.0), "0 (default)");
        assert_eq!(sensitivity_text(0.35), "+0.35");
        assert_eq!(sensitivity_text(-0.5), "-0.5");
        assert_eq!(scroll_factor_text(1.0), "1× (default)");
        assert_eq!(scroll_factor_text(1.5), "1.5×");
    }

    #[test]
    fn an_acceleration_profile_round_trips_through_its_option_text() {
        for accel in [Accel::Default, Accel::Flat, Accel::Adaptive, Accel::Custom("custom 0.2 0.0 1.0 2.0".into())] {
            assert_eq!(Accel::from_option(accel.option()), accel);
        }
        assert_eq!(Accel::from_option("[[EMPTY]]"), Accel::Default, "how hyprctl spells unset");
        assert_eq!(Accel::choices(&Accel::Flat).len(), 3);
        let custom = Accel::Custom("custom 1 2".into());
        assert_eq!(Accel::choices(&custom).last(), Some(&custom), "a custom profile is offered so it can stay");
    }

    // ── pointer.conf ─────────────────────────────────────────────────────────

    #[test]
    fn a_fresh_file_has_a_header_and_one_input_block() {
        let text = render(None, &pointer(0.3, Accel::Flat, true, 1.5));
        assert_eq!(
            text,
            format!(
                "{HEADER}input {{\n    sensitivity = 0.3\n    accel_profile = flat\n    natural_scroll = true\n    scroll_factor = 1.5\n}}\n"
            )
        );
    }

    #[test]
    fn the_default_acceleration_leaves_the_option_out() {
        let text = render(None, &Pointer::default());
        assert!(!text.contains("accel_profile"), "{text}");
        assert!(text.contains("natural_scroll = false"));
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        for p in [
            Pointer::default(),
            pointer(0.3, Accel::Flat, true, 1.5),
            pointer(-1.0, Accel::Adaptive, false, 0.25),
            pointer(1.0, Accel::Custom("custom 0.2 0.0 1.0".into()), true, 4.0),
            pointer(-0.05, Accel::Default, true, 0.1),
        ] {
            assert_eq!(parse(&render(None, &p)), Some(p.clone()), "{p:?}");
        }
    }

    #[test]
    fn the_options_are_replaced_in_place_and_the_rest_of_the_file_is_kept() {
        let before = "\
# my pointer
input {
    # slow it down
    sensitivity = -0.4
    left_handed = true
    accel_profile = adaptive
    follow_mouse = 2
    touchpad {
        natural_scroll = true
    }
}

device {
    name = some-mouse
    sensitivity = 0.9
}
";
        let after = render(Some(before), &pointer(0.3, Accel::Default, true, 2.0));
        assert_eq!(
            after,
            "\
# my pointer
input {
    # slow it down
    left_handed = true
    follow_mouse = 2
    touchpad {
        natural_scroll = true
    }
    sensitivity = 0.3
    natural_scroll = true
    scroll_factor = 2
}

device {
    name = some-mouse
    sensitivity = 0.9
}
"
        );
        assert_eq!(render(Some(&after), &pointer(0.3, Accel::Default, true, 2.0)), after, "writing the same values again changes nothing");
    }

    #[test]
    fn a_touchpad_option_is_not_mistaken_for_the_mouse_option() {
        let text = "input {\n    touchpad {\n        natural_scroll = true\n    }\n}\n";
        assert_eq!(parse(text), None, "only the input block's own options count");
        let out = render(Some(text), &Pointer::default());
        assert!(out.contains("        natural_scroll = true\n"), "the touchpad's stays");
        assert_eq!(parse(&out), Some(Pointer::default()));
    }

    #[test]
    fn a_file_without_an_input_block_gets_one_after_its_content() {
        let out = render(Some("# mine\nsource = other.conf\n"), &pointer(0.5, Accel::Flat, false, 1.0));
        assert_eq!(
            out,
            "# mine\nsource = other.conf\n\ninput {\n    sensitivity = 0.5\n    accel_profile = flat\n    natural_scroll = false\n    scroll_factor = 1\n}\n"
        );
        let empty = render(Some("  \n"), &Pointer::default());
        assert!(empty.starts_with("# Pointer options"), "a blank file is treated as a new one");
    }

    #[test]
    fn a_managed_option_in_a_second_input_block_cannot_override_the_first() {
        let before = "input {\n    sensitivity = 0.1\n}\ninput {\n    sensitivity = 0.9\n    kb_layout = us\n}\n";
        let out = render(Some(before), &pointer(0.3, Accel::Default, false, 1.0));
        assert_eq!(out.matches("sensitivity").count(), 1, "{out}");
        assert!(out.contains("kb_layout = us"));
        assert_eq!(parse(&out).map(|p| p.sensitivity), Some(0.3));
    }

    #[test]
    fn parsing_reads_comments_spellings_and_odd_values() {
        let text = "\
input {
    sensitivity = -0.25   # a little slower
    accel_profile = flat
    natural_scroll = yes
    scroll_factor = 99
}
";
        assert_eq!(parse(text), Some(pointer(-0.25, Accel::Flat, true, 10.0)), "99 is held to the ceiling");
        let broken = "input {\n    sensitivity = fast\n    natural_scroll = maybe\n}\n";
        assert_eq!(parse(broken), Some(Pointer::default()), "values that can't be read leave the default");
        assert_eq!(parse(""), None);
        assert_eq!(parse("# only a comment\n"), None);
        assert_eq!(parse("input {\n    kb_layout = us\n}\n"), None, "an input block without our options");
    }

    // ── the running compositor ───────────────────────────────────────────────

    fn compositor(provider: &'static str, answer: &'static str) -> MockRunner {
        MockRunner::new(move |_, args| match args {
            ["systeminfo"] => reply(&format!("version: 0.56.2\nconfigProvider: {provider}\n")),
            _ => reply(answer),
        })
    }

    #[test]
    fn under_lua_the_options_are_set_with_one_eval() {
        let runner = compositor("lua", "ok\n");
        apply(&runner, &pointer(0.3, Accel::Flat, true, 1.5)).unwrap();
        assert_eq!(
            runner.calls(),
            [
                "hyprctl systeminfo",
                "hyprctl eval hl.config({ input = { sensitivity = 0.3, accel_profile = \"flat\", natural_scroll = true, scroll_factor = 1.5 } })"
            ]
        );
    }

    #[test]
    fn the_default_acceleration_is_sent_as_an_empty_profile() {
        let runner = compositor("lua", "ok");
        apply(&runner, &Pointer::default()).unwrap();
        assert!(runner.calls()[1].contains("accel_profile = \"\""), "{:?}", runner.calls());
    }

    #[test]
    fn a_custom_profile_is_quoted_for_lua() {
        let runner = compositor("lua", "ok");
        apply(&runner, &pointer(0.0, Accel::Custom("a\"b".into()), false, 1.0)).unwrap();
        assert!(runner.calls()[1].contains(r#"accel_profile = "a\"b""#), "{:?}", runner.calls());
    }

    #[test]
    fn under_hyprlang_each_option_is_a_keyword() {
        let runner = compositor("hyprlang", "ok");
        apply(&runner, &pointer(-0.5, Accel::Default, false, 2.0)).unwrap();
        assert_eq!(
            runner.calls(),
            [
                "hyprctl systeminfo",
                "hyprctl keyword input:sensitivity -0.5",
                "hyprctl keyword input:accel_profile [[EMPTY]]",
                "hyprctl keyword input:natural_scroll false",
                "hyprctl keyword input:scroll_factor 2",
            ]
        );
    }

    #[test]
    fn an_answer_other_than_ok_is_an_error_and_stops_the_keywords() {
        let lua = compositor("lua", "error: bad option\n");
        let err = apply(&lua, &Pointer::default()).unwrap_err();
        assert!(err.contains("refused") && err.contains("bad option"), "{err}");

        let legacy = compositor("hyprlang", "no such option\n");
        assert!(apply(&legacy, &Pointer::default()).is_err());
        assert_eq!(legacy.calls().len(), 2, "no further keyword after the first refusal: {:?}", legacy.calls());
    }

    #[test]
    fn hyprland_not_running_is_reported_not_swallowed() {
        let runner = MockRunner::new(|_, _| failure("Couldn't connect to the Hyprland socket"));
        let err = apply(&runner, &Pointer::default()).unwrap_err();
        assert!(err.contains("Hyprland socket"), "{err}");
        assert_eq!(runner.calls(), ["hyprctl systeminfo"], "nothing is sent after the first failure");
    }

    #[test]
    fn the_live_values_are_read_from_getoption() {
        let runner = MockRunner::new(|_, args| match args {
            ["getoption", "input:sensitivity", "-j"] => reply("{\"option\": \"input:sensitivity\", \"float\": -0.300000, \"set\": true }"),
            ["getoption", "input:accel_profile", "-j"] => reply("{\"option\": \"input:accel_profile\", \"str\": \"flat\", \"set\": true }"),
            ["getoption", "input:natural_scroll", "-j"] => reply("{\"option\": \"input:natural_scroll\", \"bool\": true, \"set\": true }"),
            ["getoption", "input:scroll_factor", "-j"] => reply("{\"option\": \"input:scroll_factor\", \"float\": 2.500000, \"set\": true }"),
            other => failure(&format!("unexpected {other:?}")),
        });
        assert_eq!(read_live(&runner), Ok(pointer(-0.3, Accel::Flat, true, 2.5)));
    }

    #[test]
    fn unset_live_options_read_as_the_defaults_and_ints_count_as_booleans() {
        let runner = MockRunner::new(|_, args| match args {
            ["getoption", "input:sensitivity", "-j"] => reply("{\"option\": \"input:sensitivity\", \"float\": 0.000000, \"set\": false }"),
            ["getoption", "input:accel_profile", "-j"] => reply("{\"option\": \"input:accel_profile\", \"str\": \"[[EMPTY]]\", \"set\": false }"),
            ["getoption", "input:natural_scroll", "-j"] => reply("{\"option\": \"input:natural_scroll\", \"int\": 1, \"set\": false }"),
            ["getoption", "input:scroll_factor", "-j"] => reply("{\"option\": \"input:scroll_factor\", \"int\": 1, \"set\": false }"),
            other => failure(&format!("unexpected {other:?}")),
        });
        assert_eq!(read_live(&runner), Ok(pointer(0.0, Accel::Default, true, 1.0)));
    }

    #[test]
    fn a_live_answer_that_cannot_be_read_is_an_error() {
        let garbage = MockRunner::new(|_, _| reply("not json"));
        assert!(read_live(&garbage).unwrap_err().contains("isn't readable"));
        let unknown = MockRunner::new(|_, _| reply("{\"option\": \"input:sensitivity\", \"str\": \"x\"}"));
        assert!(read_live(&unknown).is_err());
        let down = MockRunner::new(|_, _| failure("no socket"));
        assert!(read_live(&down).is_err());
    }
}
