//! Bluetooth settings backend.
//!
//! Uses `bluetoothctl` (part of BlueZ) for all Bluetooth operations.
//! Every public function is self-contained: it spawns a `bluetoothctl` child
//! process, waits for it to exit, then returns structured data or an error string.

pub mod ctl;

// ── Re-export commonly used items ─────────────────────────────────────────────

pub use ctl::{
    connect, disconnect, forget_device, get_adapter_info, is_powered, list_devices,
    scan_devices, set_discoverable, set_powered, trust_device,
    pair_device, BluetoothDevice,
};
