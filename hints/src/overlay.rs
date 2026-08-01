//! Slint transparent overlay window.
//!
//! Full-screen, layer-shell top overlay, click-through outside hint
//! labels. Uses the exact `smpl_common::init()` FemtoVG + no-decorations
//! setup that every other smplOS app uses, so transparency + blur just
//! work.
//!
//! # Hyprland integration (matching windowrulev2)
//!
//! `src/shared/configs/hypr/windows.conf` should declare:
//!
//! ```text
//! # Full-screen click-catcher overlay for keyboard hint navigation.
//! windowrulev2 = float, initialClass:hints-overlay
//! windowrulev2 = fullscreen, initialClass:hints-overlay
//! windowrulev2 = pin, initialClass:hints-overlay
//! windowrulev2 = noborder, initialClass:hints-overlay
//! windowrulev2 = noshadow, initialClass:hints-overlay
//! windowrulev2 = norounding, initialClass:hints-overlay
//! layerrule    = blur, hints-overlay
//! ```
//!
//! The daemon uses `app_id = "hints-overlay"` (via `smpl_common::init`)
//! so those rules apply automatically.

//! Slint transparent overlay window.
//!
//! Full-screen, layer-shell top overlay, click-through outside hint
//! labels. Uses the exact `smpl_common::init()` FemtoVG + no-decorations
//! setup that every other smplOS app uses, so transparency + blur just
//! work.
//!
//! # Hyprland integration (matching windowrulev2)
//!
//! `src/shared/configs/hypr/windows.conf` should declare:
//!
//! ```text
//! # Full-screen click-catcher overlay for keyboard hint navigation.
//! windowrulev2 = float, initialClass:hints-overlay
//! windowrulev2 = fullscreen, initialClass:hints-overlay
//! windowrulev2 = pin, initialClass:hints-overlay
//! windowrulev2 = noborder, initialClass:hints-overlay
//! windowrulev2 = noshadow, initialClass:hints-overlay
//! windowrulev2 = norounding, initialClass:hints-overlay
//! layerrule    = blur, hints-overlay
//! ```
//!
//! The daemon uses `app_id = "hints-overlay"` (via `smpl_common::init`)
//! so those rules apply automatically.

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::rc::Rc;

slint::include_modules!();

/// One hint label to render.
#[derive(Clone)]
pub struct HintLabel {
    pub x: f32,
    pub y: f32,
    pub label: String,
    /// Prefix already typed — rendered dimmer so remaining chars stand out.
    pub typed_prefix: String,
}

impl From<&HintLabel> for HintItem {
    fn from(h: &HintLabel) -> Self {
        HintItem {
            x: h.x,
            y: h.y,
            label: SharedString::from(h.label.as_str()),
            typed_prefix: SharedString::from(h.typed_prefix.as_str()),
        }
    }
}

/// Which visual mode the overlay is in.
#[derive(Clone, Copy, Debug)]
pub enum VisualMode {
    /// Show hint pills.
    Select,
    /// Show the hjkl cursor crosshair.
    Cursor,
    /// No pills, no crosshair — just a focus-catcher for j/k scroll keys.
    Scroll,
}

impl VisualMode {
    fn as_str(self) -> &'static str {
        match self {
            VisualMode::Select => "select",
            VisualMode::Cursor => "cursor",
            VisualMode::Scroll => "scroll",
        }
    }
}

/// Manage the overlay window's lifecycle.
///
/// One instance for the life of the daemon. `show()` renders + presents,
/// `hide()` hides but keeps the window resources alive for instant
/// re-open.
pub struct Overlay {
    ui: OverlayWindow,
    model: Rc<VecModel<HintItem>>,
}

impl Overlay {
    /// Initialise the Slint backend, create the overlay window.
    ///
    /// Call ONCE per process, BEFORE any other Slint window.
    pub fn new() -> anyhow::Result<Self> {
        // Full-screen — we let Hyprland's `fullscreen` windowrule handle
        // the actual sizing, but pass a sensible default here for
        // pre-rule geometry.
        smpl_common::init("hints-overlay", 1920.0, 1080.0)
            .map_err(|e| anyhow::anyhow!("slint backend init failed: {e}"))?;

        let ui = OverlayWindow::new()
            .map_err(|e| anyhow::anyhow!("create overlay window: {e}"))?;
        let model = Rc::new(VecModel::from(Vec::<HintItem>::new()));
        ui.set_hints(ModelRc::from(model.clone()));
        Ok(Self { ui, model })
    }

    /// Populate the label model and show the overlay in select mode.
    pub fn show_select(&self, labels: &[HintLabel]) {
        self.set_visual_mode(VisualMode::Select);
        let items: Vec<HintItem> = labels.iter().map(HintItem::from).collect();
        self.model.set_vec(items);
        let _ = self.ui.show();
    }

    /// Refresh the hint pills in place (typed-prefix filtering).
    pub fn refresh(&self, labels: &[HintLabel]) {
        let items: Vec<HintItem> = labels.iter().map(HintItem::from).collect();
        self.model.set_vec(items);
    }

    /// Show the overlay in cursor mode (crosshair at (x, y)).
    pub fn show_cursor(&self, x: f32, y: f32) {
        self.set_visual_mode(VisualMode::Cursor);
        self.model.set_vec(Vec::new());
        self.ui.set_cursor_x(x);
        self.ui.set_cursor_y(y);
        let _ = self.ui.show();
    }

    /// Update cursor crosshair position without re-showing.
    pub fn set_cursor(&self, x: f32, y: f32) {
        self.ui.set_cursor_x(x);
        self.ui.set_cursor_y(y);
    }

    /// Show the overlay in scroll mode (no visible UI, just focus catcher).
    pub fn show_scroll(&self) {
        self.set_visual_mode(VisualMode::Scroll);
        self.model.set_vec(Vec::new());
        let _ = self.ui.show();
    }

    /// Hide the overlay without destroying it.
    pub fn hide(&self) {
        let _ = self.ui.hide();
    }

    /// Wire the key-pressed callback. Argument is the raw text of the
    /// pressed key (single char for printable, `"\u{1b}"` for Escape).
    pub fn on_key<F: FnMut(&str) + 'static>(&self, mut cb: F) {
        self.ui.on_key_pressed(move |ch| {
            cb(ch.as_str());
        });
    }

    fn set_visual_mode(&self, m: VisualMode) {
        self.ui.set_mode(SharedString::from(m.as_str()));
    }
}
