//! Settings → Display: a projection of verified Hyprland state.
//!
//! Rows come only from `hyprctl -j monitors all`; edits are normalized so the
//! canvas shows exactly what Apply writes to `~/.config/hypr/monitors.conf`
//! (see [`conf`]); Apply reloads Hyprland and re-reads what it applied.

pub mod backend;
pub mod conf;
pub mod demo;
pub mod hyprctl;
pub mod hyprland;
pub mod layout;
pub mod model;
pub mod monitor;
pub mod scale;
pub mod ui;
pub mod verify;
