//! Outputs exactly as Hyprland reports them, plus the display arithmetic
//! shared by the canvas, the monitors.conf writer and verification.

use serde::Deserialize;

use super::{conf, scale};

/// One display mode: unrotated pixel size and refresh rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: f64,
}

impl Mode {
    /// Parses an `availableModes` entry such as `1920x1080@60.00Hz`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_suffix("Hz").unwrap_or(text);
        let (resolution, refresh) = text.split_once('@')?;
        let (width, height) = resolution.split_once('x')?;
        let mode = Self {
            width: width.trim().parse().ok()?,
            height: height.trim().parse().ok()?,
            refresh: refresh.trim().parse().ok()?,
        };
        (mode.width > 0 && mode.height > 0 && mode.refresh.is_finite() && mode.refresh > 0.0)
            .then_some(mode)
    }

    /// Keeps refresh decimals so 59.94 Hz and 60 Hz modes stay distinguishable.
    pub fn label(&self) -> String {
        format!(
            "{}×{} @ {} Hz",
            self.width,
            self.height,
            refresh_text(self.refresh)
        )
    }

    /// `WxH@R.RR`, the form written to monitors.conf.
    pub fn conf_text(&self) -> String {
        format!("{}x{}@{:.2}", self.width, self.height, self.refresh)
    }

    /// Same resolution and a refresh rate within ±0.5 Hz.
    pub fn matches(&self, other: &Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && (self.refresh - other.refresh).abs() <= 0.5
    }
}

pub fn refresh_text(refresh: f64) -> String {
    let text = format!("{refresh:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The four standard orientations, indexed by transform 0–3.
pub const ORIENTATIONS: [&str; 4] = [
    "Landscape",
    "Portrait",
    "Landscape (flipped)",
    "Portrait (flipped)",
];

/// Exact name of a Wayland/Hyprland output transform (0–7).
pub fn transform_label(transform: i32) -> String {
    match transform {
        0..=3 => ORIENTATIONS[transform as usize].to_string(),
        4 => "Mirrored (transform 4)".into(),
        5 => "Mirrored, rotated 90° (transform 5)".into(),
        6 => "Mirrored, rotated 180° (transform 6)".into(),
        7 => "Mirrored, rotated 270° (transform 7)".into(),
        other => format!("Transform {other}"),
    }
}

/// Transforms offered for a display: the four standard orientations plus the
/// live transform when it is one of the mirrored values 4–7, so the dropdown
/// never has to misreport an active transform.
pub fn orientation_choices(live_transform: i32) -> Vec<i32> {
    let mut choices = vec![0, 1, 2, 3];
    if !choices.contains(&live_transform) {
        choices.push(live_transform);
    }
    choices
}

/// Pixel size after rotation: odd transforms swap the axes.
pub fn rotated(width: i32, height: i32, transform: i32) -> (i32, i32) {
    if transform.rem_euclid(2) == 1 {
        (height, width)
    } else {
        (width, height)
    }
}

/// Hyprland's logical size: rotated pixels divided by the (single precision)
/// scale, rounded (`CMonitor::applyMonitorRule`).
pub fn logical_size(mode: Mode, scale: f64, transform: i32) -> (i32, i32) {
    let (width, height) = rotated(mode.width, mode.height, transform);
    let scale = scale as f32 as f64;
    if scale.is_nan() || scale <= 0.0 {
        return (width, height);
    }
    (
        (width as f64 / scale).round() as i32,
        (height as f64 / scale).round() as i32,
    )
}

/// Why an output is (not) shown and written by Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// Enabled, physical and not mirroring.
    Managed,
    Disabled,
    Virtual,
    /// Mirroring the output with this Hyprland id.
    Mirroring(String),
}

/// One entry of `hyprctl -j monitors all`.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub id: i64,
    pub name: String,
    /// "make model serial" as Hyprland prints it (its short description).
    pub description: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub mode: Mode,
    pub x: i32,
    pub y: i32,
    /// The scale as printed by hyprctl (two decimals); see [`Output::scale`].
    pub reported_scale: f64,
    pub transform: i32,
    pub disabled: bool,
    pub mirror_of: Option<String>,
    pub available_modes: Vec<Mode>,
}

#[derive(Deserialize)]
struct RawOutput {
    #[serde(default)]
    id: i64,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    make: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    serial: String,
    #[serde(default)]
    width: i32,
    #[serde(default)]
    height: i32,
    #[serde(rename = "refreshRate", default)]
    refresh_rate: f64,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default)]
    transform: i32,
    #[serde(default)]
    disabled: bool,
    #[serde(rename = "mirrorOf", default)]
    mirror_of: Option<String>,
    #[serde(rename = "availableModes", default)]
    available_modes: Vec<String>,
}

fn default_scale() -> f64 {
    1.0
}

/// Parses `hyprctl -j monitors all`.
pub fn parse_outputs(json: &str) -> Result<Vec<Output>, String> {
    let raw: Vec<RawOutput> = serde_json::from_str(json)
        .map_err(|error| format!("unexpected hyprctl monitors output: {error}"))?;
    Ok(raw
        .into_iter()
        .map(|raw| Output {
            id: raw.id,
            name: raw.name,
            description: raw.description,
            make: raw.make,
            model: raw.model,
            serial: raw.serial,
            mode: Mode {
                width: raw.width,
                height: raw.height,
                refresh: raw.refresh_rate,
            },
            x: raw.x,
            y: raw.y,
            reported_scale: raw.scale,
            transform: raw.transform,
            disabled: raw.disabled,
            mirror_of: raw
                .mirror_of
                .filter(|source| !source.is_empty() && source != "none"),
            available_modes: raw
                .available_modes
                .iter()
                .filter_map(|mode| Mode::parse(mode))
                .collect(),
        })
        .collect())
}

/// Headless (XR glasses), nested Wayland and Hyprland's fallback outputs.
pub fn is_virtual(name: &str) -> bool {
    name.starts_with("HEADLESS-") || name.starts_with("WL-") || name == "FALLBACK"
}

impl Output {
    pub fn kind(&self) -> Kind {
        if is_virtual(&self.name) {
            Kind::Virtual
        } else if self.disabled || self.mode.width <= 0 || self.mode.height <= 0 {
            Kind::Disabled
        } else if let Some(source) = &self.mirror_of {
            Kind::Mirroring(source.clone())
        } else {
            Kind::Managed
        }
    }

    pub fn is_managed(&self) -> bool {
        self.kind() == Kind::Managed
    }

    /// The scale Hyprland is really using, recovered from the two printed
    /// decimals (4/3 is printed as 1.33 but laid out as exactly 4/3).
    pub fn scale(&self) -> f64 {
        scale::recover_live(self.mode.width, self.mode.height, self.reported_scale)
    }

    pub fn logical_size(&self) -> (i32, i32) {
        logical_size(self.mode, self.scale(), self.transform)
    }

    /// Hyprland's full output description (`make model serial (connector)`).
    fn long_description(&self) -> String {
        format!(
            "{} {} {} ({})",
            self.make, self.model, self.serial, self.name
        )
    }

    /// Hyprland's static selector match (`CMonitor::matchesStaticSelector`):
    /// `desc:` selectors are trimmed prefixes of either description, anything
    /// else must equal the connector name. The empty catch-all never matches.
    pub fn matches_selector(&self, selector: &str) -> bool {
        match selector.strip_prefix("desc:") {
            Some(text) => {
                let text = conf::trim_space(text);
                self.description.starts_with(text) || self.long_description().starts_with(text)
            }
            None => !selector.is_empty() && self.name == selector,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `hyprctl -j monitors all` entry with the fields Settings reads.
    #[allow(clippy::too_many_arguments)]
    pub fn output_json(
        id: i64,
        name: &str,
        description: &str,
        mode: (i32, i32, f64),
        position: (i32, i32),
        scale: f64,
        transform: i32,
        modes: &[&str],
    ) -> String {
        let (make, rest) = description.split_once(' ').unwrap_or((description, ""));
        format!(
            r#"{{"id": {id}, "name": "{name}", "description": "{description}", "make": "{make}", "model": "{rest}", "serial": "", "width": {}, "height": {}, "refreshRate": {:.5}, "x": {}, "y": {}, "activeWorkspace": {{"id": 1, "name": "1"}}, "scale": {scale:.2}, "transform": {transform}, "focused": false, "dpmsStatus": true, "disabled": false, "mirrorOf": "none", "availableModes": [{}]}}"#,
            mode.0,
            mode.1,
            mode.2,
            position.0,
            position.1,
            modes
                .iter()
                .map(|m| format!("\"{m}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    pub fn outputs(entries: &[String]) -> Vec<Output> {
        parse_outputs(&format!("[{}]", entries.join(","))).unwrap()
    }

    /// The live machine from the bug report: DP-3 is really portrait.
    pub fn live_pair() -> Vec<Output> {
        outputs(&[
            output_json(
                0,
                "HDMI-A-1",
                "Lenovo Group Limited LEN P27h-10 0x01010101",
                (2560, 1440, 59.951),
                (0, 0),
                1.0,
                0,
                &[
                    "2560x1440@59.95Hz",
                    "1920x1080@60.00Hz",
                    "1920x1080@59.94Hz",
                ],
            ),
            output_json(
                1,
                "DP-3",
                "Dell Inc. DELL P2412H KG49T35D59GU",
                (1920, 1080, 60.0),
                (2560, 0),
                1.0,
                1,
                &["1920x1080@60.00Hz", "1280x1024@75.03Hz"],
            ),
        ])
    }

    #[test]
    fn parses_live_hyprctl_output_with_exact_transform_and_modes() {
        let json = r#"[{"id": 1, "name": "DP-3", "description": "Dell Inc. DELL P2412H KG49T35D59GU",
            "make": "Dell Inc.", "model": "DELL P2412H", "serial": "KG49T35D59GU",
            "width": 1920, "height": 1080, "refreshRate": 60.00000, "x": 2560, "y": 0,
            "scale": 1.00, "transform": 5, "disabled": false, "mirrorOf": "none",
            "availableModes": ["1920x1080@60.00Hz","1280x1024@75.03Hz","bogus"]}]"#;
        let outputs = parse_outputs(json).unwrap();
        assert_eq!(outputs.len(), 1);
        let dp3 = &outputs[0];
        assert_eq!(dp3.transform, 5);
        assert_eq!(dp3.mirror_of, None);
        assert_eq!(dp3.available_modes.len(), 2);
        assert_eq!(dp3.logical_size(), (1080, 1920));
        assert!(dp3.is_managed());
        assert!(parse_outputs("not json").is_err());
    }

    #[test]
    fn virtual_disabled_and_mirroring_outputs_are_not_managed() {
        let mut outputs = live_pair();
        let mut headless = outputs[0].clone();
        headless.name = "HEADLESS-2".into();
        let mut disabled = outputs[0].clone();
        disabled.disabled = true;
        let mut mirror = outputs[0].clone();
        mirror.mirror_of = Some("1".into());
        assert_eq!(headless.kind(), Kind::Virtual);
        assert_eq!(disabled.kind(), Kind::Disabled);
        assert_eq!(mirror.kind(), Kind::Mirroring("1".into()));
        for name in ["WL-1", "FALLBACK"] {
            outputs[0].name = name.into();
            assert_eq!(outputs[0].kind(), Kind::Virtual);
        }
    }

    #[test]
    fn mode_labels_keep_fractional_refresh_rates_distinct() {
        let sixty = Mode::parse("1920x1080@60.00Hz").unwrap();
        let ntsc = Mode::parse("1920x1080@59.94Hz").unwrap();
        assert_eq!(sixty.label(), "1920×1080 @ 60 Hz");
        assert_eq!(ntsc.label(), "1920×1080 @ 59.94 Hz");
        assert_eq!(ntsc.conf_text(), "1920x1080@59.94");
        assert!(sixty.matches(&ntsc));
        assert!(Mode::parse("0x0@60Hz").is_none());
    }

    #[test]
    fn transforms_four_to_seven_have_exact_names_and_choices() {
        assert_eq!(orientation_choices(1), vec![0, 1, 2, 3]);
        for transform in 4..=7 {
            assert_eq!(orientation_choices(transform), vec![0, 1, 2, 3, transform]);
            assert!(transform_label(transform).contains(&format!("transform {transform}")));
        }
        assert_eq!(rotated(1920, 1080, 5), (1080, 1920));
        assert_eq!(rotated(1920, 1080, 6), (1920, 1080));
    }

    #[test]
    fn desc_selectors_match_by_prefix_of_either_description() {
        let dp3 = &live_pair()[1];
        assert!(dp3.matches_selector("desc:Dell Inc. DELL P2412H KG49T35D59GU"));
        assert!(dp3.matches_selector("desc:  Dell Inc. DELL  "));
        assert!(dp3.matches_selector("DP-3"));
        assert!(!dp3.matches_selector("DP-31"));
        assert!(!dp3.matches_selector("desc:Lenovo"));
        assert!(!dp3.matches_selector(""));
    }
}
