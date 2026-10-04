//! The display backend interface and session detection.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;

pub use super::conf::Target;
use super::hyprctl::SystemRunner;
use super::hyprland::HyprlandBackend;
use super::monitor::{Kind, Output};

/// Everything the Display tab shows, read in one pass.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Every output from `hyprctl -j monitors all`, managed or not.
    pub outputs: Vec<Output>,
    /// monitors.conf content (`None` when missing) or why it is unreadable.
    pub conf: Result<Option<String>, String>,
    /// The output currently showing workspace 1.
    pub workspace_one: Option<String>,
    /// `workspace-ctl enabled`: workspaces follow the display arrangement.
    pub workspace_policy: bool,
}

impl Snapshot {
    /// Identity of the display configuration (outputs and saved file),
    /// ignoring focus, workspaces and other volatile fields.
    pub fn fingerprint(&self) -> u64 {
        let mut outputs: Vec<String> = self
            .outputs
            .iter()
            .map(|o| {
                let modes: Vec<String> = o.available_modes.iter().map(|m| m.conf_text()).collect();
                format!(
                    "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{:.2}\u{1f}{}\u{1f}{}\u{1f}{:?}\u{1f}{}",
                    o.name,
                    o.description,
                    o.mode.conf_text(),
                    o.x,
                    o.y,
                    o.reported_scale,
                    o.transform,
                    o.disabled,
                    o.mirror_of,
                    modes.join(",")
                )
            })
            .collect();
        outputs.sort();
        let mut hasher = DefaultHasher::new();
        outputs.hash(&mut hasher);
        format!("{:?}", self.conf).hash(&mut hasher);
        hasher.finish()
    }

    pub fn conf_text(&self) -> &str {
        match &self.conf {
            Ok(Some(text)) => text,
            _ => "",
        }
    }

    pub fn managed(&self) -> impl Iterator<Item = &Output> {
        self.outputs.iter().filter(|output| output.is_managed())
    }

    /// Outputs that are neither drawn nor written, for the status line.
    pub fn unmanaged_note(&self) -> Option<String> {
        let entries: Vec<String> = self
            .outputs
            .iter()
            .filter_map(|output| match output.kind() {
                Kind::Managed => None,
                Kind::Disabled => Some(format!("{} (turned off)", output.name)),
                Kind::Virtual => Some(format!("{} (virtual)", output.name)),
                Kind::Mirroring(source) => {
                    let source = self
                        .outputs
                        .iter()
                        .find(|candidate| candidate.id.to_string() == source)
                        .map_or(source, |candidate| candidate.name.clone());
                    Some(format!("{} (mirroring {source})", output.name))
                }
            })
            .collect();
        (!entries.is_empty()).then(|| format!("Not shown or saved: {}.", entries.join(", ")))
    }
}

/// Result of Apply, "Keep current" and "Use saved".
#[derive(Debug, Clone)]
pub struct ApplyReport {
    /// The live state after reloading; it replaces the edited model.
    pub snapshot: Snapshot,
    /// Fields Hyprland did not apply (empty = verified).
    pub differences: Vec<String>,
    /// Config errors and side effects that failed without undoing the change.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ApplyOutcome {
    /// Refused before writing: the displays changed since editing began.
    Stale,
    /// Not completed. `snapshot` is the live state when something may have
    /// changed already; `None` means nothing was changed.
    Failed {
        message: String,
        snapshot: Option<Snapshot>,
    },
    Finished(ApplyReport),
}

pub trait DisplayBackend: Send + Sync {
    fn snapshot(&self) -> Result<Snapshot, String>;
    /// Writes `targets` unless the live fingerprint differs from `baseline`,
    /// reloads Hyprland and verifies the result.
    fn apply(&self, baseline: u64, targets: &[Target]) -> ApplyOutcome;
    /// Reloads the saved file and reports what still differs from it.
    fn use_saved(&self) -> ApplyOutcome;
    fn move_workspace_one(&self, connector: &str) -> Result<(), String>;
    /// Shows `(connector, label)` overlays on the matching screens.
    fn identify(&self, labels: &[(String, String)]) -> Result<(), String>;
    fn is_demo(&self) -> bool {
        false
    }
}

/// Why display settings are unavailable in this session, if they are.
pub fn unavailable_reason(var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let niri = var("NIRI_SOCKET").is_some();
    if var("HYPRLAND_INSTANCE_SIGNATURE").is_some() && !niri {
        return None;
    }
    Some(
        if niri {
            "Display settings work with Hyprland only. This niri session manages displays in its own configuration."
        } else if var("WAYLAND_DISPLAY").is_some() {
            "Display settings work with Hyprland only, and this Wayland session is not running Hyprland."
        } else if var("DISPLAY").is_some() {
            "Display settings work with Hyprland only, and this is an X11 session."
        } else {
            "No graphical session was found, so displays can't be read."
        }
        .into(),
    )
}

/// The Hyprland backend, or the reason the Display tab is unavailable.
/// There is no silent demo fallback; `--demo` must be explicit.
pub fn detect_backend() -> Result<Arc<dyn DisplayBackend>, String> {
    let var = |key: &str| std::env::var(key).ok().filter(|value| !value.is_empty());
    if let Some(reason) = unavailable_reason(&var) {
        return Err(reason);
    }
    let home =
        var("HOME").ok_or("HOME is not set, so the saved display settings can't be found.")?;
    Ok(Arc::new(HyprlandBackend::new(
        Arc::new(SystemRunner),
        PathBuf::from(home),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::monitor::tests::live_pair;

    fn snapshot() -> Snapshot {
        Snapshot {
            outputs: live_pair(),
            conf: Ok(Some("monitor = DP-3, preferred, auto, 1\n".into())),
            workspace_one: Some("HDMI-A-1".into()),
            workspace_policy: false,
        }
    }

    #[test]
    fn fingerprint_tracks_layout_and_file_but_not_focus_or_workspaces() {
        let base = snapshot();
        let mut moved_workspace = snapshot();
        moved_workspace.workspace_one = Some("DP-3".into());
        moved_workspace.workspace_policy = true;
        assert_eq!(base.fingerprint(), moved_workspace.fingerprint());
        let mut reordered = snapshot();
        reordered.outputs.reverse();
        assert_eq!(base.fingerprint(), reordered.fingerprint());
        let mut rotated = snapshot();
        rotated.outputs[1].transform = 0;
        assert_ne!(base.fingerprint(), rotated.fingerprint());
        let mut edited = snapshot();
        edited.conf = Ok(None);
        assert_ne!(base.fingerprint(), edited.fingerprint());
    }

    #[test]
    fn unmanaged_outputs_are_named_in_the_note() {
        let mut snap = snapshot();
        assert_eq!(snap.unmanaged_note(), None);
        let mut headless = snap.outputs[0].clone();
        headless.name = "HEADLESS-2".into();
        let mut mirror = snap.outputs[0].clone();
        mirror.name = "DP-1".into();
        mirror.mirror_of = Some("1".into());
        let mut off = snap.outputs[0].clone();
        off.name = "HDMI-A-2".into();
        off.disabled = true;
        snap.outputs.extend([headless, mirror, off]);
        assert_eq!(snap.managed().count(), 2);
        assert_eq!(
            snap.unmanaged_note().unwrap(),
            "Not shown or saved: HEADLESS-2 (virtual), DP-1 (mirroring DP-3), HDMI-A-2 (turned off)."
        );
    }

    #[test]
    fn non_hyprland_sessions_are_unavailable_with_a_reason() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |key: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            unavailable_reason(&env(&[("HYPRLAND_INSTANCE_SIGNATURE", "abc")])),
            None
        );
        for (pairs, expected) in [
            (
                &[
                    ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
                    ("NIRI_SOCKET", "/run/niri"),
                ][..],
                "niri",
            ),
            (
                &[("WAYLAND_DISPLAY", "wayland-1")][..],
                "not running Hyprland",
            ),
            (&[("DISPLAY", ":0")][..], "X11"),
            (&[][..], "No graphical session"),
        ] {
            let reason = unavailable_reason(&env(pairs)).unwrap();
            assert!(reason.contains(expected), "{reason}");
        }
    }
}
