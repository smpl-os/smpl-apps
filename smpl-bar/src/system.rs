use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UsbDevice {
    pub name: String,
    pub path: String,
    pub size: String,
    pub label: String,
    pub mounted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemState {
    pub battery_present: bool,
    pub battery_level: u8,
    pub battery_charging: bool,
    pub battery_low: bool,
    pub brightness: u8,
    pub wired: bool,
    pub wired_available: bool,
    pub wifi_available: bool,
    pub wifi_on: bool,
    pub ssid: String,
    pub online: bool,
    pub vpn: bool,
    pub bluetooth_available: bool,
    pub bluetooth_on: bool,
    pub airplane: bool,
    pub nightlight: bool,
    pub dictation_available: bool,
    pub dictation_active: bool,
    pub sync_active: bool,
    pub keypad_present: bool,
    pub notification_count: u32,
    pub notifications_paused: bool,
    pub volume: u8,
    pub volume_muted: bool,
    pub usb: Vec<UsbDevice>,
}

#[derive(Default)]
pub struct SystemSampler {
    battery_path: Option<PathBuf>,
    battery_discovered: bool,
    backlight_path: Option<PathBuf>,
    path_directories: Vec<PathBuf>,
    home: PathBuf,
    runtime: PathBuf,
}

impl SystemSampler {
    pub fn new() -> Self {
        Self {
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            runtime: std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
                }),
            path_directories: executable_directories(),
            ..Self::default()
        }
    }

    #[cfg(test)]
    pub fn with_roots(root: &Path) -> Self {
        let sampler = Self::new();
        Self {
            path_directories: sampler.path_directories,
            home: root.to_owned(),
            runtime: root.join("run"),
            ..Self::default()
        }
    }

    pub fn sample(&mut self) -> SystemState {
        self.sample_below(Path::new("/"), Path::new("/proc"), &self.home.clone())
    }

    pub fn sample_below(&mut self, sys_root: &Path, proc_root: &Path, home: &Path) -> SystemState {
        let mut state = SystemState::default();
        self.sample_battery(sys_root.join("sys/class/power_supply"), &mut state);
        self.sample_brightness(sys_root.join("sys/class/backlight"), &mut state);
        self.sample_network(
            sys_root.join("sys/class/net"),
            &proc_root.join("net/route"),
            sys_root.join("sys/class/rfkill"),
            &mut state,
        );
        self.sample_bluetooth(
            sys_root.join("sys/class/bluetooth"),
            sys_root.join("sys/class/rfkill"),
            &mut state,
        );
        self.sample_processes(proc_root, &mut state);
        state.usb = scan_usb(
            sys_root.join("sys/class/block"),
            proc_root.join("self/mountinfo"),
        );
        state.keypad_present = scan_keypad(sys_root.join("sys/bus/usb/devices"));
        state.dictation_available = executable_exists(&self.path_directories, "voxtype");
        state.airplane = state.wifi_available && !state.wifi_on && !state.bluetooth_on;
        state.brightness = state.brightness.max(1);
        state.volume = 50;
        state.ssid = wireless_ssid(home, &self.runtime);
        state
    }

    fn sample_battery(&mut self, root: PathBuf, state: &mut SystemState) {
        if !self.battery_discovered {
            self.battery_path = first_battery(&root);
            self.battery_discovered = true;
        }
        let Some(path) = &self.battery_path else {
            return;
        };
        state.battery_present = true;
        state.battery_level = read_number(path.join("capacity")).unwrap_or(0).min(100) as u8;
        let status = read_trimmed(path.join("status")).unwrap_or_default();
        state.battery_charging = status == "Charging";
        state.battery_low =
            state.battery_level <= 15 && !state.battery_charging && status != "Full";
    }

    fn sample_brightness(&mut self, root: PathBuf, state: &mut SystemState) {
        if self.backlight_path.is_none() {
            self.backlight_path = sorted_dirs(&root).into_iter().next();
        }
        let Some(path) = &self.backlight_path else {
            state.brightness = 100;
            return;
        };
        let maximum = read_number(path.join("max_brightness")).unwrap_or(0);
        let current = read_number(path.join("brightness")).unwrap_or(0);
        state.brightness = current
            .min(maximum)
            .saturating_mul(100)
            .checked_div(maximum)
            .unwrap_or(100)
            .clamp(1, 100) as u8;
    }

    fn sample_network(
        &self,
        interfaces: PathBuf,
        route: &Path,
        rfkill_root: PathBuf,
        state: &mut SystemState,
    ) {
        for directory in sorted_dirs(&interfaces) {
            let Some(name) = directory.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if name == "lo" {
                continue;
            }
            let up = read_trimmed(directory.join("operstate")).as_deref() == Some("up");
            if directory.join("wireless").exists() {
                state.wifi_available = true;
                state.wifi_on |= up;
            } else if read_trimmed(directory.join("type")).as_deref() == Some("1")
                && directory.join("device").exists()
            {
                state.wired_available = true;
                state.wired |= up;
            }
            if up
                && (name.starts_with("wg")
                    || name.starts_with("tun")
                    || name.starts_with("tap")
                    || name.contains("vpn"))
            {
                state.vpn = true;
            }
        }
        let mut wifi_radios = 0;
        let mut active_wifi_radios = 0;
        for entry in sorted_dirs(&rfkill_root) {
            if read_trimmed(entry.join("type")).as_deref() != Some("wlan") {
                continue;
            }
            wifi_radios += 1;
            if read_trimmed(entry.join("state")).as_deref() == Some("1") {
                active_wifi_radios += 1;
            }
        }
        state.wifi_available |= wifi_radios > 0;
        state.wifi_on |= active_wifi_radios > 0;
        state.online = route_default_exists(route);
    }

    fn sample_bluetooth(&self, root: PathBuf, rfkill_root: PathBuf, state: &mut SystemState) {
        state.bluetooth_available = std::fs::read_dir(root)
            .ok()
            .into_iter()
            .flat_map(|entries| entries.filter_map(Result::ok))
            .any(|entry| entry.file_name().to_string_lossy().starts_with("hci"));
        let mut bluetooth_radios = 0;
        let mut powered_radios = 0;
        for entry in sorted_dirs(&rfkill_root) {
            if read_trimmed(entry.join("type")).as_deref() != Some("bluetooth") {
                continue;
            }
            bluetooth_radios += 1;
            if read_trimmed(entry.join("state")).as_deref() == Some("1") {
                powered_radios += 1;
            }
        }
        state.bluetooth_available |= bluetooth_radios > 0;
        state.bluetooth_on = powered_radios > 0;
    }

    fn sample_processes(&self, root: &Path, state: &mut SystemState) {
        let mut names = HashSet::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.filter_map(Result::ok).take(8192) {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .bytes()
                    .all(|byte| byte.is_ascii_digit())
                {
                    continue;
                }
                if let Some(name) = read_trimmed(entry.path().join("comm")) {
                    names.insert(name);
                }
            }
        }
        state.nightlight = names.contains("gammastep");
        state.dictation_active = names.contains("voxtype");
    }
}

fn first_battery(root: &Path) -> Option<PathBuf> {
    sorted_dirs(root).into_iter().find(|path| {
        read_trimmed(path.join("type")).as_deref() == Some("Battery")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("hid-") || name.starts_with("hidpp_"))
    })
}

fn sorted_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    paths.sort();
    paths
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn read_number(path: PathBuf) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

fn route_default_exists(path: &Path) -> bool {
    std::fs::read_to_string(path).ok().is_some_and(|source| {
        source
            .lines()
            .skip(1)
            .any(|line| line.split_whitespace().nth(1) == Some("00000000"))
    })
}

fn executable_directories() -> Vec<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|path| path.is_absolute())
        .collect()
}

fn executable_exists(directories: &[PathBuf], name: &str) -> bool {
    directories
        .iter()
        .any(|directory| directory.join(name).is_file())
}

fn scan_usb(block_root: PathBuf, mountinfo: PathBuf) -> Vec<UsbDevice> {
    let mut devices = Vec::new();
    let mounts = std::fs::read_to_string(mountinfo).unwrap_or_default();
    let mut mounted_bases = HashSet::new();
    for line in mounts.lines() {
        let (before, after) = match line.split_once(" - ") {
            Some(parts) => parts,
            None => continue,
        };
        let fields: Vec<_> = before.split_whitespace().collect();
        let mount = fields.get(4).map(|value| mount_unescape(value));
        let filesystem: Vec<_> = after.split_whitespace().collect();
        let source = filesystem.get(1).map(|value| mount_unescape(value));
        let (Some(mount), Some(source)) = (mount, source) else {
            continue;
        };
        if !mount.starts_with("/run/media/") || !source.starts_with("/dev/") {
            continue;
        }
        let name = Path::new(&source)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_owned();
        let base = block_base(&name);
        if read_trimmed(block_root.join(&base).join("removable")).as_deref() != Some("1") {
            continue;
        }
        mounted_bases.insert(base.clone());
        devices.push(UsbDevice {
            label: Path::new(&mount)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(&name)
                .to_owned(),
            name,
            path: mount,
            size: block_size(&block_root, &base),
            mounted: true,
        });
    }
    for path in sorted_dirs(&block_root) {
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if read_trimmed(path.join("removable")).as_deref() != Some("1")
            || mounted_bases.contains(name)
        {
            continue;
        }
        devices.push(UsbDevice {
            label: name.to_owned(),
            name: name.to_owned(),
            path: String::new(),
            size: block_size(&block_root, name),
            mounted: false,
        });
    }
    devices.sort_by(|left, right| left.name.cmp(&right.name));
    devices
}

fn block_base(partition: &str) -> String {
    if partition.starts_with("nvme") || partition.starts_with("mmcblk") {
        partition
            .rsplit_once('p')
            .and_then(|(base, suffix)| {
                suffix
                    .chars()
                    .all(|value| value.is_ascii_digit())
                    .then(|| base.to_owned())
            })
            .unwrap_or_else(|| partition.to_owned())
    } else {
        partition.trim_end_matches(char::is_numeric).to_owned()
    }
}

fn block_size(root: &Path, base: &str) -> String {
    let sectors = read_number(root.join(base).join("size")).unwrap_or(0);
    let bytes = sectors.saturating_mul(512);
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1_000_000_000.0)
    } else if bytes > 0 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else {
        String::new()
    }
}

fn mount_unescape(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn scan_keypad(root: PathBuf) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        read_trimmed(entry.path().join("idVendor")).as_deref() == Some("1189")
            && read_trimmed(entry.path().join("idProduct")).as_deref() == Some("8890")
    })
}

fn wireless_ssid(home: &Path, runtime: &Path) -> String {
    for path in [
        runtime.join("NetworkManager"),
        home.join(".cache/smplos/network.json"),
    ] {
        if path.is_file() {
            return std::fs::read_to_string(path)
                .ok()
                .and_then(|source| serde_json::from_str::<NetworkHint>(&source).ok())
                .and_then(|hint| hint.ssid)
                .unwrap_or_default();
        }
    }
    String::new()
}

#[derive(Deserialize)]
struct NetworkHint {
    ssid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_partition_bases_cover_usb_names() {
        assert_eq!(block_base("sdb1"), "sdb");
        assert_eq!(block_base("nvme0n1p3"), "nvme0n1");
        assert_eq!(block_base("mmcblk0p1"), "mmcblk0");
    }

    #[test]
    fn low_battery_is_shape_and_state_based() {
        let fixture = tempfile::tempdir().unwrap();
        let supply = fixture.path().join("sys/class/power_supply/BAT0");
        std::fs::create_dir_all(&supply).unwrap();
        std::fs::write(supply.join("type"), "Battery\n").unwrap();
        std::fs::write(supply.join("capacity"), "12\n").unwrap();
        std::fs::write(supply.join("status"), "Discharging\n").unwrap();
        let mut sampler = SystemSampler::with_roots(fixture.path());
        let state =
            sampler.sample_below(fixture.path(), &fixture.path().join("proc"), fixture.path());
        assert_eq!(state.battery_level, 12);
        assert!(state.battery_low);
    }

    #[test]
    fn only_removable_mounted_usb_partitions_are_listed() {
        let fixture = tempfile::tempdir().unwrap();
        let block = fixture.path().join("sys/class/block/sdb");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("removable"), "1\n").unwrap();
        std::fs::write(block.join("size"), "1000000\n").unwrap();
        let mountinfo = fixture.path().join("mountinfo");
        std::fs::write(
            &mountinfo,
            "24 0 8:17 / /run/media/user/FLASH rw - ext4 /dev/sdb1 rw\n",
        )
        .unwrap();
        let devices = scan_usb(fixture.path().join("sys/class/block"), mountinfo);
        assert_eq!(
            devices.len(),
            1,
            "the mounted partition and its parent are one drive"
        );
        assert_eq!(devices[0].name, "sdb1");
        assert_eq!(devices[0].path, "/run/media/user/FLASH");
    }

    #[test]
    fn unmounted_removable_disk_remains_visible() {
        let fixture = tempfile::tempdir().unwrap();
        let block = fixture.path().join("sys/class/block/sdc");
        std::fs::create_dir_all(&block).unwrap();
        std::fs::write(block.join("removable"), "1\n").unwrap();
        std::fs::write(block.join("size"), "1000\n").unwrap();
        let devices = scan_usb(
            fixture.path().join("sys/class/block"),
            fixture.path().join("missing-mountinfo"),
        );
        assert_eq!(devices.len(), 1);
        assert!(!devices[0].mounted);
        assert_eq!(devices[0].path, "");
    }
}
