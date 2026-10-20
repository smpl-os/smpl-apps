//! Settings > Connections: Wi-Fi and Bluetooth together, under one Airplane Mode switch.
//!
//! The page has two sections that share the sidebar entry "Connections". Their
//! window indices stay 7 (Wi-Fi) and 8 (Bluetooth), so saved deep links, the search
//! index and the number-key shortcuts keep working unchanged.

pub mod airplane;
pub mod radio;
pub mod state;
pub mod ui;
pub mod view;

#[cfg(test)]
mod tests;

/// Window index of the Wi-Fi section (also what `--tab connections` opens).
pub const WIFI_SECTION: i32 = 7;
/// Window index of the Bluetooth section.
pub const BLUETOOTH_SECTION: i32 = 8;

/// Search and `--highlight` labels whose controls are in the Wi-Fi section.
pub const WIFI_LABELS: [&str; 9] = [
    "Wi-Fi",
    "WiFi",
    "Wireless",
    "Connect Wi-Fi",
    "Wi-Fi Password",
    "Network",
    "QR Code Wi-Fi",
    "Share Wi-Fi",
    "Scan QR",
];

/// Search and `--highlight` labels whose controls are in the Bluetooth section.
pub const BLUETOOTH_LABELS: [&str; 6] = [
    "Bluetooth",
    "Bluetooth Devices",
    "Pair Bluetooth",
    "Connect Bluetooth",
    "Headphones",
    "Discoverable",
];

/// The section a label belongs to, if it belongs to one. "Airplane Mode" and
/// "Connections" are on top of both sections and do not pick one.
pub fn section_for_label(label: &str) -> Option<i32> {
    let matches = |labels: &[&str]| labels.iter().any(|l| l.eq_ignore_ascii_case(label));
    if matches(&BLUETOOTH_LABELS) {
        Some(BLUETOOTH_SECTION)
    } else if matches(&WIFI_LABELS) {
        Some(WIFI_SECTION)
    } else {
        None
    }
}

/// Where a deep link lands, given the window index its `--tab` name maps to and
/// its `--highlight` label.
///
/// Inside Connections the label decides the section (`--tab wifi --highlight
/// Headphones` shows Bluetooth); without a label the tab name decides. Other
/// tabs are left alone.
pub fn route(tab: i32, highlight: &str) -> i32 {
    if tab != WIFI_SECTION && tab != BLUETOOTH_SECTION {
        return tab;
    }
    section_for_label(highlight).unwrap_or(tab)
}
