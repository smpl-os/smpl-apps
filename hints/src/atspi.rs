//! AT-SPI2 widget-tree enumeration.
//!
//! Connects to the accessibility bus and walks every visible top-level
//! application, collecting bounding rectangles for widgets that
//! * are marked Visible + Showing, and
//! * have an interesting role (buttons, menu items, links, entries, …), and
//! * expose the `Component` interface with a non-empty screen extent that
//!   meets the configured minimum size.
//!
//! Output is a [`Vec<Widget>`] ordered top-to-bottom, left-to-right so
//! generated hint labels stay stable across enumeration calls when the
//! desktop is quiet.
//!
//! # Runtime notes
//!
//! * GTK apps expose AT-SPI natively (excellent coverage).
//! * Qt apps need `QT_ACCESSIBILITY=1` in the environment.
//! * Electron apps need `--force-renderer-accessibility` (Chromium flag).
//! * `at-spi2-registryd` must be running; on Arch/Hyprland it's autostart
//!   via `dbus-run-session`. If missing, the connection call returns an
//!   error and the daemon reports it back over IPC.
//!
//! # Walk strategy
//!
//! Iterative depth-first, guarded by a hard node budget so a pathological
//! app (some Electron trees have ~20k nodes) can't wedge the daemon. On
//! nodes whose role isn't hintable we still descend but skip the extent
//! query — that's the cheapest branch and covers containers like Panel,
//! Filler, ScrollPane, Frame, etc.

use anyhow::{Context, Result};
use atspi::proxy::accessible::{AccessibleProxy, ObjectRefExt};
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{AccessibilityConnection, CoordType, Role, State};

/// Hard cap on total nodes visited per enumeration.
///
/// A typical desktop hits 1–5k. 30k accommodates outlier Electron apps
/// (VS Code, Slack) without going runaway.
const MAX_NODES: usize = 30_000;

/// A widget worth showing a hint on.
#[derive(Clone, Debug)]
pub struct Widget {
    /// Bounding rectangle in screen coordinates.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Center of the widget — the click target.
    pub cx: i32,
    pub cy: i32,
    /// Human-readable role name (for debugging / future styling).
    pub role: String,
    /// D-Bus destination + object path — kept so the daemon can fall
    /// back to AT-SPI `Action.doAction("click")` when virtual-pointer
    /// injection fails on confined compositors.
    pub bus_name: String,
    pub object_path: String,
}

/// Enumerate every hintable widget on screen right now.
pub async fn enumerate(min_size: u32) -> Result<Vec<Widget>> {
    // Ensure AT-SPI is enabled on this session (idempotent no-op if it
    // already is). Some apps only publish their tree after `IsEnabled`
    // flips to true — GTK4 is one.
    let _ = atspi::connection::set_session_accessibility(true).await;

    let conn = AccessibilityConnection::new()
        .await
        .context("connect to at-spi bus (is at-spi2-registryd running?)")?;

    // Hold a clone of the zbus connection for the duration of the walk so
    // every AccessibleProxy we build borrows from something we own.
    let bus = conn.connection().clone();

    let registry = conn
        .root_accessible_on_registry()
        .await
        .context("query at-spi registry root")?;

    let mut out = Vec::with_capacity(256);
    walk_all_apps(&registry, &bus, min_size, &mut out).await?;

    // Sort top-to-bottom, left-to-right → deterministic hint order across
    // enumeration calls, which keeps muscle memory stable.
    out.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)));
    Ok(out)
}

/// Iterative depth-first walk of every application's tree.
async fn walk_all_apps<'a>(
    registry: &AccessibleProxy<'a>,
    bus: &zbus::Connection,
    min_size: u32,
    out: &mut Vec<Widget>,
) -> Result<()> {
    // Registry's children == one entry per running accessible app.
    let app_refs = registry
        .get_children()
        .await
        .context("registry.get_children")?;

    let mut nodes_visited: usize = 0;

    for app_ref in app_refs {
        if app_ref.is_null() {
            continue;
        }
        let Ok(app) = app_ref.into_accessible_proxy(bus).await else { continue };

        let mut stack: Vec<AccessibleProxy<'_>> = vec![app];
        while let Some(node) = stack.pop() {
            nodes_visited += 1;
            if nodes_visited > MAX_NODES {
                tracing::warn!("atspi walk hit MAX_NODES ({MAX_NODES}); truncating");
                return Ok(());
            }

            // Filter 1: only visible + showing widgets are worth hinting.
            // If the state query fails, we still descend into children
            // (pattern seen in some GTK4 apps).
            let visible = match node.get_state().await {
                Ok(states) => states.contains(State::Visible) && states.contains(State::Showing),
                Err(_) => false,
            };

            let role = node.get_role().await.unwrap_or(Role::Invalid);

            if visible && role_is_hintable(role) {
                // Filter 2: node must implement Component and have a
                // non-zero screen-coord extent that meets min_size.
                if let Ok(proxies) = node.proxies().await {
                    if let Ok(component) = proxies.component().await {
                        if let Ok((x, y, w, h)) = component.get_extents(CoordType::Screen).await {
                            if w >= min_size as i32 && h >= min_size as i32 {
                                let inner = node.inner();
                                out.push(Widget {
                                    x,
                                    y,
                                    width: w,
                                    height: h,
                                    cx: x + w / 2,
                                    cy: y + h / 2,
                                    role: format!("{role:?}"),
                                    bus_name: inner.destination().to_string(),
                                    object_path: inner.path().to_string(),
                                });
                            }
                        }
                    }
                }
            }

            // Descend regardless of role — a Panel/Filler/etc. is not
            // hintable itself but may contain buttons.
            if let Ok(children) = node.get_children().await {
                for child_ref in children {
                    if child_ref.is_null() {
                        continue;
                    }
                    if let Ok(child) = child_ref.into_accessible_proxy(bus).await {
                        stack.push(child);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Which AT-SPI roles are worth a hint?
///
/// Covers what Vimium/Hints care about: interactive UI. Extend as we
/// find false negatives in the wild.
fn role_is_hintable(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::PushButtonMenu
            | Role::ToggleButton
            | Role::RadioButton
            | Role::CheckBox
            | Role::MenuItem
            | Role::CheckMenuItem
            | Role::RadioMenuItem
            | Role::Menu
            | Role::MenuBar
            | Role::Link
            | Role::PageTab
            | Role::PageTabList
            | Role::ListItem
            | Role::TreeItem
            | Role::ComboBox
            | Role::Entry
            | Role::PasswordText
            | Role::Slider
            | Role::ScrollBar
            | Role::Icon
            | Role::Image
    )
}

