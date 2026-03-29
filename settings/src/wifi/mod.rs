//! Wi-Fi settings backend.
//!
//! Three sub-modules with clear responsibilities:
//!   - [`nmcli`]  — all NetworkManager/nmcli subprocess calls
//!   - [`qr`]     — WIFI: URI encoding, decoding, and pixel-buffer rendering
//!
//! Security invariants
//! -------------------
//! * Passwords are **never** written to disk by this module.
//! * Credentials are held only in stack/heap memory for the duration of a single call.
//! * The `nmcli` sub-process receives passwords via argv (not via stdin or env) so
//!   they never touch the filesystem; nmcli then hands them to the NetworkManager
//!   D-Bus secret agent which manages its own secure keyring.
//! * QR pixel buffers are held only in `slint::SharedPixelBuffer` (process memory)
//!   unless the user explicitly requests an export.

pub mod nmcli;
pub mod qr;
pub mod secure;

// ── Re-export the most commonly used items ────────────────────────────────────

pub use nmcli::{
    connect, connect_open, disconnect, forget_network, get_current_ssid, get_saved_password,
    list_networks, set_airplane_mode, WifiNetwork,
};
pub use qr::{export_qr_svg, generate_wifi_qr, parse_wifi_uri, wifi_uri, WifiAuth};
pub use secure::SecretString;
