//! Shared, resident-shell building blocks for smplOS native surfaces.
//!
//! This crate deliberately contains no UI code. Apps share the desktop models,
//! process-safe launch helpers, IPC wire protocol and event adapters while
//! each binary owns its Slint layout and lifetime.

pub mod app_cache;
pub mod compositor;
pub mod config;
pub mod icons;
pub mod ipc;
pub mod launch;
pub mod monitor;
pub mod telemetry;

pub use smpl_common::theme;
