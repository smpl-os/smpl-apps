//! Comparing what Settings wrote or saved with what Hyprland applied.

use super::conf::{self, Target};
use super::monitor::{refresh_text, transform_label, Output};
use super::scale;

fn short_scale(scale: f64) -> String {
    refresh_text(scale)
}

/// Every written field that Hyprland did not apply: mode exact, refresh
/// ±0.5 Hz, position exact, scale ±0.01, transform exact.
pub fn differences(targets: &[Target], live: &[Output]) -> Vec<String> {
    let mut found = Vec::new();
    for target in targets {
        let name = &target.name;
        let Some(output) = live.iter().find(|output| &output.name == name) else {
            found.push(format!("{name} is no longer connected"));
            continue;
        };
        if output.disabled {
            found.push(format!("{name} is turned off"));
            continue;
        }
        if output.mirror_of.is_some() {
            found.push(format!("{name} is mirroring another display"));
        }
        let (mode, wanted) = (output.mode, target.mode);
        if (mode.width, mode.height) != (wanted.width, wanted.height) {
            found.push(format!(
                "{name} resolution: wanted {}×{}, active {}×{}",
                wanted.width, wanted.height, mode.width, mode.height
            ));
        }
        if (mode.refresh - wanted.refresh).abs() > 0.5 {
            found.push(format!(
                "{name} refresh rate: wanted {} Hz, active {} Hz",
                refresh_text(wanted.refresh),
                refresh_text(mode.refresh)
            ));
        }
        if (output.x, output.y) != (target.x, target.y) {
            found.push(format!(
                "{name} position: wanted {}x{}, active {}x{}",
                target.x, target.y, output.x, output.y
            ));
        }
        if (output.reported_scale - target.scale).abs() > 0.01 {
            found.push(format!(
                "{name} scale: wanted {}, active {}",
                short_scale(target.scale),
                short_scale(output.reported_scale)
            ));
        }
        if output.transform != target.transform {
            found.push(format!(
                "{name} orientation: wanted {}, active {}",
                transform_label(target.transform),
                transform_label(output.transform)
            ));
        }
    }
    found
}

/// A managed output whose effective saved rule differs from the live state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drift {
    pub name: String,
    pub saved: Vec<String>,
    pub active: Vec<String>,
}

impl Drift {
    pub fn sentence(&self) -> String {
        format!(
            "Saved settings for {} differ from what is active: saved {}, active {}.",
            self.name,
            self.saved.join(" and "),
            self.active.join(" and ")
        )
    }
}

/// Compares each managed output's effective saved rule with the live state:
/// disabled, transform (default 0), and mode, position and scale only when
/// the rule sets them explicitly. Scales are compared after Hyprland's
/// clean-divisor fix-up, ±0.01.
pub fn drift(outputs: &[Output], conf_text: &str) -> Vec<Drift> {
    let rules = conf::rules(conf_text);
    outputs
        .iter()
        .filter(|output| output.is_managed())
        .filter_map(|output| {
            let rule = conf::effective(&rules, output)?;
            let mut saved = Vec::new();
            let mut active = Vec::new();
            if rule.disabled {
                saved.push("turned off".to_string());
                active.push("turned on".to_string());
            } else {
                if rule.transform != output.transform {
                    saved.push(transform_label(rule.transform));
                    active.push(transform_label(output.transform));
                }
                if let Some(mode) = rule.mode {
                    let resolution =
                        (mode.width, mode.height) != (output.mode.width, output.mode.height);
                    let refresh = mode
                        .refresh
                        .is_some_and(|refresh| (refresh - output.mode.refresh).abs() > 0.5);
                    if resolution || refresh {
                        saved.push(mode.label());
                        active.push(output.mode.label());
                    }
                }
                if let Some((x, y)) = rule.position {
                    if (x, y) != (output.x, output.y) {
                        saved.push(format!("position {x}x{y}"));
                        active.push(format!("position {}x{}", output.x, output.y));
                    }
                }
                if let Some(requested) = rule.scale {
                    let applied =
                        scale::hyprland_effective(output.mode.width, output.mode.height, requested);
                    if let Some(applied) =
                        applied.filter(|a| (a - output.reported_scale).abs() > 0.01)
                    {
                        saved.push(format!("scale {}", short_scale(applied)));
                        active.push(format!("scale {}", short_scale(output.reported_scale)));
                    }
                }
            }
            (!saved.is_empty()).then(|| Drift {
                name: output.name.clone(),
                saved,
                active,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::monitor::tests::live_pair;
    use crate::display::monitor::Mode;

    fn targets() -> Vec<Target> {
        let live = live_pair();
        live.iter()
            .map(|o| Target {
                name: o.name.clone(),
                mode: o.mode,
                x: o.x,
                y: o.y,
                scale: o.scale(),
                transform: o.transform,
            })
            .collect()
    }

    #[test]
    fn matching_live_state_verifies() {
        assert!(differences(&targets(), &live_pair()).is_empty());
        // Refresh within 0.5 Hz and scale within 0.01 still verify.
        let mut wanted = targets();
        wanted[0].mode.refresh = 59.95;
        wanted[1].scale = 1.004;
        assert!(differences(&wanted, &live_pair()).is_empty());
    }

    #[test]
    fn every_differing_field_is_listed() {
        let mut wanted = targets();
        wanted[1].transform = 0;
        wanted[1].x = 2000;
        wanted[1].scale = 1.5;
        wanted[1].mode = Mode {
            width: 1280,
            height: 1024,
            refresh: 75.03,
        };
        let found = differences(&wanted, &live_pair());
        assert_eq!(
            found,
            [
                "DP-3 resolution: wanted 1280×1024, active 1920×1080",
                "DP-3 refresh rate: wanted 75.03 Hz, active 60 Hz",
                "DP-3 position: wanted 2000x0, active 2560x0",
                "DP-3 scale: wanted 1.5, active 1",
                "DP-3 orientation: wanted Landscape, active Portrait",
            ]
        );
        let mut gone = live_pair();
        gone.pop();
        assert_eq!(
            differences(&targets(), &gone),
            ["DP-3 is no longer connected"]
        );
        let mut off = live_pair();
        off[1].disabled = true;
        assert_eq!(differences(&targets(), &off), ["DP-3 is turned off"]);
    }

    #[test]
    fn drift_reports_saved_portrait_active_landscape() {
        let mut live = live_pair();
        live[1].transform = 0;
        let saved = "monitor = DP-3, 1920x1080@60.00, 2560x0, 1.00, transform, 1\n";
        let found = drift(&live, saved);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].sentence(),
            "Saved settings for DP-3 differ from what is active: saved Portrait, active Landscape."
        );
        assert!(drift(&live_pair(), saved).is_empty());
    }

    #[test]
    fn drift_only_checks_explicit_values_and_effective_rules() {
        let live = live_pair();
        // No rule, the catch-all, auto values and an implicit transform 0 that matches.
        for text in [
            "",
            "monitor = , preferred, auto, 2\n",
            "monitor = HDMI-A-1, preferred, auto, auto\n",
            "monitor = DP-3, transform, 0\nmonitor = DP-3, highres, auto-right, auto, transform, 1\n",
        ] {
            assert!(drift(&live, text).is_empty(), "{text:?}");
        }
        // A later desc rule overrides the connector rule.
        let later = "monitor = DP-3, preferred, auto, 1, transform, 1\nmonitor = desc:Dell Inc. DELL, preferred, auto, 1\n";
        assert_eq!(drift(&live, later)[0].saved, ["Landscape"]);
        let disabled = drift(&live, "monitor = DP-3, disable\n");
        assert_eq!(disabled[0].saved, ["turned off"]);
        // An invalid transform on a full line means 0.
        let invalid = drift(
            &live,
            "monitor = DP-3, preferred, auto, 1, transform, 1.0\n",
        );
        assert_eq!(invalid[0].saved, ["Landscape"]);
    }

    #[test]
    fn drift_compares_scale_after_hyprland_fixup() {
        let mut live = live_pair();
        live[0].reported_scale = 1.6;
        assert!(drift(&live, "monitor = HDMI-A-1, 2560x1440@59.95, 0x0, 1.5\n").is_empty());
        let found = drift(&live, "monitor = HDMI-A-1, 2560x1440@59.95, 0x0, 1.25\n");
        assert_eq!(found[0].saved, ["scale 1.25"]);
        assert_eq!(found[0].active, ["scale 1.6"]);
        let moved = drift(&live_pair(), "monitor = HDMI-A-1, 1920x1080@60, 100x0, 1\n");
        assert_eq!(moved[0].saved, ["1920×1080 @ 60 Hz", "position 100x0"]);
        assert_eq!(moved[0].active, ["2560×1440 @ 59.95 Hz", "position 0x0"]);
    }
}
