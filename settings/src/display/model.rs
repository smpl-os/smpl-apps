//! The Display tab's state: the last verified live snapshot plus pending
//! edits. Rows are always derived from here; nothing else holds a copy.

use super::backend::{Snapshot, Target};
use super::layout::{self, Rect};
use super::monitor::{self, Mode, Output};
use super::scale;
use super::verify::{self, Drift};

/// One managed display as shown and edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    pub mode: Mode,
    /// Available modes, plus the live mode if Hyprland did not list it.
    pub modes: Vec<Mode>,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    /// Exact transform 0–7.
    pub transform: i32,
    /// The transform when loaded; decides the orientation choices.
    pub live_transform: i32,
}

impl Monitor {
    fn from_output(output: &Output) -> Self {
        let mut modes = output.available_modes.clone();
        if !modes.iter().any(|mode| mode.matches(&output.mode)) {
            modes.insert(0, output.mode);
        }
        Self {
            name: output.name.clone(),
            description: output.description.clone(),
            mode: output.mode,
            modes,
            x: output.x,
            y: output.y,
            scale: output.scale(),
            transform: output.transform,
            live_transform: output.transform,
        }
    }

    pub fn logical_size(&self) -> (i32, i32) {
        monitor::logical_size(self.mode, self.scale, self.transform)
    }

    pub fn rect(&self) -> Rect {
        let (w, h) = self.logical_size();
        Rect {
            x: self.x,
            y: self.y,
            w,
            h,
        }
    }

    /// The current mode's row: same resolution, nearest refresh rate.
    pub fn mode_index(&self) -> Option<usize> {
        let distance = |mode: &Mode| (mode.refresh - self.mode.refresh).abs();
        self.modes
            .iter()
            .enumerate()
            .filter(|(_, mode)| mode.matches(&self.mode))
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))
            .map(|(index, _)| index)
    }

    pub fn orientation_choices(&self) -> Vec<i32> {
        monitor::orientation_choices(self.live_transform)
    }

    pub fn orientation_index(&self) -> Option<usize> {
        self.orientation_choices()
            .iter()
            .position(|&transform| transform == self.transform)
    }

    /// Rotated pixel size on one line and on two (for narrow rectangles).
    pub fn size_labels(&self) -> (String, String) {
        let (w, h) = monitor::rotated(self.mode.width, self.mode.height, self.transform);
        (format!("{w}×{h}"), format!("{w} ×\n{h}"))
    }

    pub fn target(&self) -> Target {
        Target {
            name: self.name.clone(),
            mode: self.mode,
            x: self.x,
            y: self.y,
            scale: self.scale,
            transform: self.transform,
        }
    }
}

pub const CANVAS_W: f64 = 580.0;
pub const CANVAS_H: f64 = 200.0;
const CANVAS_MARGIN: f64 = 20.0;
/// Magnetic drop distance in canvas pixels.
const SNAP_PIXELS: f64 = 8.0;

/// Maps logical coordinates onto the canvas card and back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    pub scale: f64,
    min_x: f64,
    min_y: f64,
}

impl Canvas {
    pub fn fit(monitors: &[Monitor]) -> Self {
        let rects: Vec<Rect> = monitors.iter().map(Monitor::rect).collect();
        let (Some(min_x), Some(min_y), Some(max_x), Some(max_y)) = (
            rects.iter().map(|r| r.x as f64).reduce(f64::min),
            rects.iter().map(|r| r.y as f64).reduce(f64::min),
            rects
                .iter()
                .map(|r| r.x as f64 + r.w as f64)
                .reduce(f64::max),
            rects
                .iter()
                .map(|r| r.y as f64 + r.h as f64)
                .reduce(f64::max),
        ) else {
            return Self {
                scale: 0.1,
                min_x: 0.0,
                min_y: 0.0,
            };
        };
        // Content starts at the margin and leaves room below/right to drag into.
        let fit = |space: f64, size: f64| (space - 3.0 * CANVAS_MARGIN) / size.max(1.0);
        Self {
            scale: fit(CANVAS_W, max_x - min_x)
                .min(fit(CANVAS_H, max_y - min_y))
                .min(0.25),
            min_x,
            min_y,
        }
    }

    pub fn position(&self, x: i32, y: i32) -> (f32, f32) {
        (
            ((x as f64 - self.min_x) * self.scale + CANVAS_MARGIN) as f32,
            ((y as f64 - self.min_y) * self.scale + CANVAS_MARGIN) as f32,
        )
    }

    pub fn size(&self, w: i32, h: i32) -> (f32, f32) {
        (
            (w as f64 * self.scale) as f32,
            (h as f64 * self.scale) as f32,
        )
    }

    pub fn logical_at(&self, x: f32, y: f32) -> (i32, i32) {
        (
            ((x as f64 - CANVAS_MARGIN) / self.scale + self.min_x).round() as i32,
            ((y as f64 - CANVAS_MARGIN) / self.scale + self.min_y).round() as i32,
        )
    }

    pub fn snap_threshold(&self) -> i32 {
        (SNAP_PIXELS / self.scale).round() as i32
    }
}

#[derive(Default)]
pub struct DisplayModel {
    snapshot: Option<Snapshot>,
    baseline: u64,
    pub monitors: Vec<Monitor>,
    original: Vec<Monitor>,
    /// Selection by connector name, so reloads that reorder outputs keep it.
    selected: Option<String>,
    /// The live configuration changed while edits were pending.
    pub stale: bool,
    loaded: bool,
}

impl DisplayModel {
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Fingerprint of the snapshot the edits are based on.
    pub fn baseline(&self) -> u64 {
        self.baseline
    }

    /// Replaces edits and baseline with the live state. The selection follows
    /// its connector name and is cleared when that display is gone; the
    /// first load selects the first display.
    pub fn load(&mut self, snapshot: Snapshot) {
        let monitors: Vec<Monitor> = snapshot.managed().map(Monitor::from_output).collect();
        self.baseline = snapshot.fingerprint();
        self.original = monitors.clone();
        self.monitors = monitors;
        self.stale = false;
        if !self.loaded {
            self.selected = self.monitors.first().map(|m| m.name.clone());
            self.loaded = true;
        } else if let Some(name) = &self.selected {
            if !self.monitors.iter().any(|m| &m.name == name) {
                self.selected = None;
            }
        }
        self.snapshot = Some(snapshot);
    }

    /// Takes a newer snapshot with the same display configuration (the
    /// workspace-1 host or policy may differ). Returns whether those changed.
    pub fn refresh_details(&mut self, snapshot: Snapshot) -> bool {
        let changed = self.snapshot.as_ref().is_none_or(|old| {
            old.workspace_one != snapshot.workspace_one
                || old.workspace_policy != snapshot.workspace_policy
        });
        self.snapshot = Some(snapshot);
        changed
    }

    pub fn has_changes(&self) -> bool {
        self.monitors != self.original
    }

    pub fn selected_index(&self) -> i32 {
        self.selected
            .as_ref()
            .and_then(|name| self.monitors.iter().position(|m| &m.name == name))
            .map_or(-1, |index| index as i32)
    }

    pub fn select(&mut self, index: i32) {
        self.selected = usize::try_from(index)
            .ok()
            .and_then(|index| self.monitors.get(index))
            .map(|m| m.name.clone());
    }

    /// Picks another mode and re-snaps the scale for it. Choosing the row
    /// that already represents the live mode (e.g. 59.95 Hz for 59.951 Hz)
    /// is not a change.
    pub fn change_mode(&mut self, index: usize, mode_index: usize) -> bool {
        let before = self.rects();
        let Some(monitor) = self.monitors.get_mut(index) else {
            return false;
        };
        let Some(&mode) = monitor.modes.get(mode_index) else {
            return false;
        };
        if mode == monitor.mode || monitor.mode_index() == Some(mode_index) {
            return false;
        }
        monitor.mode = mode;
        if !scale::is_exact(mode.width, mode.height, monitor.scale) {
            monitor.scale = scale::snap(mode.width, mode.height, monitor.scale);
        }
        self.normalize_after(&before);
        true
    }

    /// Snaps to the nearest scale Hyprland keeps for the current mode.
    pub fn change_scale(&mut self, index: usize, requested: f64) -> bool {
        let before = self.rects();
        let Some(monitor) = self.monitors.get_mut(index) else {
            return false;
        };
        let snapped = scale::snap(monitor.mode.width, monitor.mode.height, requested);
        if (snapped - monitor.scale).abs() < 1e-9 {
            return false;
        }
        monitor.scale = snapped;
        self.normalize_after(&before);
        true
    }

    pub fn change_orientation(&mut self, index: usize, choice: usize) -> bool {
        let before = self.rects();
        let Some(monitor) = self.monitors.get_mut(index) else {
            return false;
        };
        let Some(&transform) = monitor.orientation_choices().get(choice) else {
            return false;
        };
        if transform == monitor.transform {
            return false;
        }
        monitor.transform = transform;
        self.normalize_after(&before);
        true
    }

    /// Drops a dragged display at logical `x`, `y`.
    pub fn drop_at(&mut self, index: usize, x: i32, y: i32, threshold: i32) -> bool {
        let Some(monitor) = self.monitors.get(index) else {
            return false;
        };
        let before: Vec<(i32, i32)> = self.monitors.iter().map(|m| (m.x, m.y)).collect();
        let others: Vec<Rect> = self
            .monitors
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .map(|(_, m)| m.rect())
            .collect();
        let dropped = layout::snap_drop(
            &others,
            Rect {
                x,
                y,
                ..monitor.rect()
            },
            threshold,
        );
        let rects = self.rects();
        self.monitors[index].x = dropped.x;
        self.monitors[index].y = dropped.y;
        self.normalize_after(&rects);
        self.monitors.iter().map(|m| (m.x, m.y)).collect::<Vec<_>>() != before
    }

    fn rects(&self) -> Vec<Rect> {
        self.monitors.iter().map(Monitor::rect).collect()
    }

    /// Normalizes once a user edit changed any rectangle; edits that keep the
    /// geometry (a refresh rate, a 180° turn) leave the layout as it was.
    fn normalize_after(&mut self, before: &[Rect]) {
        let rects = self.rects();
        if rects == before {
            return;
        }
        for (monitor, rect) in self.monitors.iter_mut().zip(layout::normalize(&rects)) {
            monitor.x = rect.x;
            monitor.y = rect.y;
        }
    }

    pub fn revert(&mut self) {
        self.monitors = self.original.clone();
    }

    pub fn targets(&self) -> Vec<Target> {
        self.monitors.iter().map(Monitor::target).collect()
    }

    /// The loaded live layout ("Keep current").
    pub fn live_targets(&self) -> Vec<Target> {
        self.original.iter().map(Monitor::target).collect()
    }

    pub fn workspace_one(&self) -> Option<&str> {
        self.snapshot.as_ref()?.workspace_one.as_deref()
    }

    pub fn workspace_policy(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| s.workspace_policy)
    }

    /// Saved rules that differ from the loaded live state.
    pub fn drift(&self) -> Vec<Drift> {
        self.snapshot
            .as_ref()
            .map(|s| verify::drift(&s.outputs, s.conf_text()))
            .unwrap_or_default()
    }

    pub fn unmanaged_note(&self) -> Option<String> {
        self.snapshot.as_ref()?.unmanaged_note()
    }

    pub fn conf_error(&self) -> Option<&str> {
        self.snapshot
            .as_ref()?
            .conf
            .as_ref()
            .err()
            .map(String::as_str)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::display::monitor::tests::live_pair;

    pub fn snapshot(outputs: Vec<Output>) -> Snapshot {
        Snapshot {
            outputs,
            conf: Ok(Some(
                "monitor = HDMI-A-1, 2560x1440@59.95, 0x0, 1.00, transform, 0\nmonitor = DP-3, 1920x1080@60.00, 2560x0, 1.00, transform, 1\n".into(),
            )),
            workspace_one: Some("HDMI-A-1".into()),
            workspace_policy: false,
        }
    }

    pub fn loaded() -> DisplayModel {
        let mut model = DisplayModel::default();
        model.load(snapshot(live_pair()));
        model
    }

    #[test]
    fn loading_shows_the_live_layout_untouched_and_selects_the_first_display() {
        let model = loaded();
        assert_eq!(model.selected_index(), 0);
        assert!(!model.has_changes());
        let dp3 = &model.monitors[1];
        assert_eq!((dp3.x, dp3.y, dp3.transform), (2560, 0, 1));
        assert_eq!(dp3.orientation_index(), Some(1), "DP-3 is portrait");
        assert_eq!(dp3.size_labels().0, "1080×1920");
        assert_eq!(
            dp3.rect(),
            Rect {
                x: 2560,
                y: 0,
                w: 1080,
                h: 1920
            }
        );
        assert!(model.drift().is_empty());
    }

    #[test]
    fn selection_follows_the_connector_across_reorders_and_clears_when_gone() {
        let mut model = loaded();
        model.select(1);
        let mut reordered = live_pair();
        reordered.reverse();
        model.load(snapshot(reordered));
        assert_eq!(model.selected_index(), 0);
        assert_eq!(model.monitors[0].name, "DP-3");
        let mut unplugged = live_pair();
        unplugged.retain(|o| o.name != "DP-3");
        model.load(snapshot(unplugged));
        assert_eq!(model.selected_index(), -1);
        model.load(snapshot(live_pair()));
        assert_eq!(
            model.selected_index(),
            -1,
            "only the first load auto-selects"
        );
        model.select(7);
        assert_eq!(model.selected_index(), -1);
    }

    #[test]
    fn unmanaged_outputs_are_not_drawn() {
        let mut outputs = live_pair();
        let mut headless = outputs[0].clone();
        headless.name = "HEADLESS-1".into();
        outputs.insert(0, headless);
        let mut model = DisplayModel::default();
        model.load(snapshot(outputs));
        assert_eq!(
            model
                .monitors
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            ["HDMI-A-1", "DP-3"]
        );
        assert!(model
            .unmanaged_note()
            .unwrap()
            .contains("HEADLESS-1 (virtual)"));
    }

    #[test]
    fn mirrored_transforms_round_trip_exactly() {
        let mut outputs = live_pair();
        outputs[1].transform = 5;
        let mut model = DisplayModel::default();
        model.load(snapshot(outputs));
        let dp3 = &model.monitors[1];
        assert_eq!(dp3.orientation_choices(), [0, 1, 2, 3, 5]);
        assert_eq!(dp3.orientation_index(), Some(4));
        assert_eq!(model.targets()[1].transform, 5);
        assert!(model.change_orientation(1, 1));
        assert_eq!(model.targets()[1].transform, 1);
        assert!(model.change_orientation(1, 4));
        assert_eq!(model.targets()[1].transform, 5);
        assert!(!model.has_changes());
    }

    #[test]
    fn orientation_changes_keep_the_arrangement_attached() {
        let mut model = loaded();
        assert!(model.change_orientation(0, 1));
        let rects: Vec<Rect> = model.monitors.iter().map(Monitor::rect).collect();
        assert_eq!(
            rects[0],
            Rect {
                x: 0,
                y: 0,
                w: 1440,
                h: 2560
            }
        );
        assert_eq!(
            rects[1],
            Rect {
                x: 1440,
                y: 0,
                w: 1080,
                h: 1920
            }
        );
        assert!(model.has_changes());
        model.revert();
        assert!(!model.has_changes());
    }

    #[test]
    fn scale_changes_snap_for_the_mode_and_mode_changes_resnap() {
        let mut model = loaded();
        assert!(model.change_scale(0, 1.5));
        assert!(
            (model.monitors[0].scale - 1.6).abs() < 1e-9,
            "1.5 is not clean on 2560x1440"
        );
        assert_eq!(
            model.monitors[1].x, 1600,
            "DP-3 follows the smaller HDMI-A-1"
        );
        assert!(!model.change_scale(0, 1.6));
        let mode_1080 = model.monitors[0]
            .modes
            .iter()
            .position(|m| m.width == 1920 && m.refresh == 60.0)
            .unwrap();
        assert!(model.change_mode(0, mode_1080));
        assert_eq!(model.monitors[0].mode.conf_text(), "1920x1080@60.00");
        assert_eq!(model.monitors[0].scale, 1.6, "1.6 is clean on 1920x1080");
        assert_eq!(model.monitors[0].mode_index(), Some(mode_1080));
        let ntsc = model.monitors[0]
            .modes
            .iter()
            .position(|m| m.refresh == 59.94)
            .unwrap();
        assert!(model.change_mode(0, ntsc));
        assert_eq!(
            model.monitors[0].mode_index(),
            Some(ntsc),
            "59.94 Hz stays distinct from 60 Hz"
        );
    }

    #[test]
    fn reselecting_the_live_mode_or_keeping_geometry_changes_nothing_else() {
        let mut outputs = live_pair();
        // A live layout that is valid but does not start at 0,0.
        for output in &mut outputs {
            output.x -= 100;
            output.y += 50;
        }
        let mut model = DisplayModel::default();
        model.load(snapshot(outputs));
        let live = model.monitors[0].mode_index().unwrap();
        assert!(
            !model.change_mode(0, live),
            "59.95 Hz row is the live 59.951 Hz mode"
        );
        assert!(!model.has_changes());
        let ntsc = model.monitors[0]
            .modes
            .iter()
            .position(|m| m.refresh == 59.94)
            .unwrap();
        let sixty = model.monitors[0]
            .modes
            .iter()
            .position(|m| m.width == 1920 && m.refresh == 60.0)
            .unwrap();
        let positions = |model: &DisplayModel| {
            model
                .monitors
                .iter()
                .map(|m| (m.x, m.y))
                .collect::<Vec<_>>()
        };
        assert!(
            model.change_mode(0, sixty),
            "a resolution change normalizes"
        );
        let normalized = positions(&model);
        assert!(
            model.change_mode(0, ntsc),
            "a refresh-only change is still a change"
        );
        assert_eq!(positions(&model), normalized, "but nothing moves");
        model.revert();
        assert!(model.change_orientation(0, 2), "180° keeps the size");
        assert_eq!((model.monitors[0].x, model.monitors[0].y), (-100, 50));
        assert_eq!((model.monitors[1].x, model.monitors[1].y), (2460, 50));
    }

    #[test]
    fn drops_snap_and_normalize_into_what_will_be_saved() {
        let mut model = loaded();
        // Dropped overlapping HDMI-A-1 from below-left: resolved and translated.
        assert!(model.drop_at(1, -1000, 1500, 100));
        let rects: Vec<Rect> = model.monitors.iter().map(Monitor::rect).collect();
        assert!(!rects[0].overlaps(&rects[1]));
        assert!(rects[0].shares_edge(&rects[1]));
        assert_eq!(rects.iter().map(|r| r.x).min(), Some(0));
        assert_eq!(rects.iter().map(|r| r.y).min(), Some(0));
        let targets = model.targets();
        assert_eq!(
            (targets[1].x, targets[1].y),
            (model.monitors[1].x, model.monitors[1].y)
        );
    }

    #[test]
    fn canvas_round_trips_positions() {
        let model = loaded();
        let canvas = Canvas::fit(&model.monitors);
        for monitor in &model.monitors {
            let (x, y) = canvas.position(monitor.x, monitor.y);
            assert_eq!(canvas.logical_at(x, y), (monitor.x, monitor.y));
        }
        let (w, _) = canvas.size(1080, 1920);
        assert!(w > 0.0);
        assert!(canvas.snap_threshold() > 0);
    }

    #[test]
    fn live_modes_missing_from_the_list_are_still_shown() {
        let mut outputs = live_pair();
        outputs[1].available_modes.clear();
        let mut model = DisplayModel::default();
        model.load(snapshot(outputs));
        assert_eq!(model.monitors[1].mode_index(), Some(0));
        assert_eq!(model.monitors[1].modes.len(), 1);
    }
}
