//! Hand-off from a later `settings` launch to the instance that is already
//! running.
//!
//! The newcomer leaves a small request file in the runtime directory, waits for
//! the running instance to pick it up, raises that window and exits. The
//! running instance watches for the file (see [`watch`]) and navigates.

use crate::MainWindow;
use slint::ComponentHandle;
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Sidebar index of the Keybindings tab (`--tab keybindings`).
pub const KEYBINDINGS_TAB: i32 = 5;

const REQUEST_FILE: &str = "settings-deeplink";
const WINDOW_CLASS: &str = "settings";
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const PICKUP_TIMEOUT: Duration = Duration::from_millis(1500);
const PICKUP_STEP: Duration = Duration::from_millis(25);
const MAX_COMPOSITOR_REPLY: u64 = 256 * 1024;

/// What a launch asks of the Settings window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Request {
    /// Tab to show; `None` leaves the current one alone.
    pub tab: Option<i32>,
    /// Setting label to blink; empty for none.
    pub highlight: String,
}

impl Request {
    /// The request implied by the command line: `tab` is `Some` only when
    /// `--tab` was given.
    pub fn from_launch(tab: Option<i32>, highlight: &str) -> Self {
        // `--highlight` on its own has always meant the first tab.
        let tab = tab.or_else(|| (!highlight.is_empty()).then_some(0));
        Self {
            tab,
            highlight: highlight.to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tab.is_none() && self.highlight.is_empty()
    }

    /// Arriving on the Keybindings tab this way leaves the keyboard in its
    /// filter box.
    pub fn focuses_keybindings_filter(&self) -> bool {
        self.tab == Some(KEYBINDINGS_TAB)
    }

    /// The tab to show. Wi-Fi and Bluetooth are two sections of the Connections
    /// tab, and a launch lands on the section its label belongs to, whichever of
    /// `--tab wifi`, `--tab bluetooth` and `--tab connections` was given. The
    /// request keeps the tab as the launch named it, so builds that don't know
    /// about sections read the same file.
    pub fn landing_tab(&self) -> Option<i32> {
        self.tab
            .map(|tab| crate::connections::route(tab, &self.highlight))
    }

    /// `<tab>\n<highlight>\n`. The tab line is empty when there is no tab. Builds
    /// that only knew the tab-and-highlight pair read the same two lines.
    fn encode(&self) -> String {
        let tab = self.tab.map(|tab| tab.to_string()).unwrap_or_default();
        let highlight = self.highlight.replace(['\n', '\r'], " ");
        format!("{tab}\n{highlight}\n")
    }

    fn decode(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let tab = lines.next()?.trim();
        let tab = if tab.is_empty() {
            None
        } else {
            Some(tab.parse().ok()?)
        };
        let highlight = lines.next().unwrap_or_default().to_string();
        Some(Self { tab, highlight })
    }
}

fn run_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| PathBuf::from("/tmp"), PathBuf::from)
}

fn request_path(dir: &Path) -> PathBuf {
    dir.join(REQUEST_FILE)
}

// ── Newcomer side ────────────────────────────────────────────────────────────

/// Take the single-instance lock, or hand `request` to the running instance,
/// raise its window and exit.
pub fn acquire_or_hand_off(request: &Request) {
    use std::os::unix::io::AsRawFd;
    let dir = run_dir();
    let lock_path = dir.join("settings.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap_or_else(|e| {
            eprintln!("[settings] cannot open lock file: {}", e);
            std::process::exit(1);
        });
    let fd = file.as_raw_fd();
    let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if ret != 0 {
        eprintln!("[settings] another instance is already running — bringing it to focus");
        hand_off_in(&dir, request, PICKUP_TIMEOUT, raise_window);
        std::process::exit(0);
    }
    std::mem::forget(file);
}

/// Leave `request` for the running instance, wait until it has been picked up
/// (so the window is raised already showing the right tab), then `raise`.
fn hand_off_in(dir: &Path, request: &Request, timeout: Duration, raise: impl FnOnce()) {
    if !request.is_empty() {
        match send_in(dir, request) {
            Ok(()) => {
                wait_for_pickup(dir, timeout);
            }
            Err(e) => eprintln!("[settings] cannot pass the request on: {e}"),
        }
    }
    raise();
}

/// Written under a temporary name and renamed into place, so the running
/// instance never sees a half-written request.
fn send_in(dir: &Path, request: &Request) -> std::io::Result<()> {
    let temp = dir.join(format!("{REQUEST_FILE}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&temp);
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(request.encode().as_bytes())?;
        drop(file);
        std::fs::rename(&temp, request_path(dir))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

fn wait_for_pickup(dir: &Path, timeout: Duration) -> bool {
    let path = request_path(dir);
    let deadline = Instant::now() + timeout;
    while path.exists() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(PICKUP_STEP);
    }
    true
}

// ── Raising the window ───────────────────────────────────────────────────────

/// Bring the running Settings window to the foreground.
fn raise_window() {
    match focus_class(WINDOW_CLASS) {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => eprintln!("[settings] cannot raise the window through the compositor: {e}"),
    }
    let _ = std::process::Command::new("hyprctl")
        .args(["dispatch", "focuswindow", "class:settings"])
        .status();
}

fn focus_class(class: &str) -> Result<bool, String> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return Ok(false);
    }
    focus_with(hyprland_request, class)
}

/// Focus the mapped window of `class` through compositor `request`s. `Ok(false)`
/// means there was no such window.
fn focus_with(
    mut request: impl FnMut(&str) -> Result<String, String>,
    class: &str,
) -> Result<bool, String> {
    let Some(address) = window_address(&request("j/clients")?, class)? else {
        return Ok(false);
    };
    // Lua-configured Hyprland reads dispatch arguments as Lua expressions; the
    // classic dispatcher is the fallback for hyprlang configs.
    let reply = request(&format!(
        "dispatch hl.dsp.focus({{window=\"address:{address}\"}})"
    ))?;
    let reply = if reply.trim() == "Invalid dispatcher" {
        request(&format!("dispatch focuswindow address:{address}"))?
    } else {
        reply
    };
    if reply.trim() == "ok" {
        Ok(true)
    } else {
        Err(format!("the compositor answered {:?}", reply.trim()))
    }
}

fn window_address(clients: &str, class: &str) -> Result<Option<String>, String> {
    let clients: Vec<serde_json::Value> =
        serde_json::from_str(clients).map_err(|e| format!("unreadable client list: {e}"))?;
    for client in clients {
        let wanted = client.get("class").and_then(|v| v.as_str()) == Some(class)
            && client.get("mapped").and_then(|v| v.as_bool()) == Some(true);
        if !wanted {
            continue;
        }
        let address = client
            .get("address")
            .and_then(|v| v.as_str())
            .ok_or("the settings window has no compositor address")?;
        if address.len() <= 2
            || !address.starts_with("0x")
            || !address[2..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(format!("invalid window address {address:?}"));
        }
        return Ok(Some(address.to_string()));
    }
    Ok(None)
}

fn hyprland_socket(runtime_dir: &Path, signature: &str) -> Result<PathBuf, String> {
    if signature.is_empty() || signature.contains('/') || signature == "." || signature == ".." {
        return Err("invalid compositor instance".into());
    }
    Ok(runtime_dir
        .join("hypr")
        .join(signature)
        .join(".socket.sock"))
}

fn hyprland_request(request: &str) -> Result<String, String> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(|e| e.to_string())?;
    let runtime = dirs::runtime_dir().ok_or("no runtime directory for the compositor")?;
    let socket = hyprland_socket(&runtime, &signature)?;
    let mut stream = UnixStream::connect(&socket)
        .map_err(|e| format!("cannot connect to the compositor: {e}"))?;
    let timeout = Some(Duration::from_secs(1));
    stream.set_read_timeout(timeout).map_err(|e| e.to_string())?;
    stream.set_write_timeout(timeout).map_err(|e| e.to_string())?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    stream
        .shutdown(std::net::Shutdown::Write)
        .map_err(|e| e.to_string())?;
    let mut reply = String::new();
    stream
        .take(MAX_COMPOSITOR_REPLY + 1)
        .read_to_string(&mut reply)
        .map_err(|e| e.to_string())?;
    if reply.len() as u64 > MAX_COMPOSITOR_REPLY {
        return Err("the compositor reply is too large".into());
    }
    Ok(reply)
}

// ── Running instance side ────────────────────────────────────────────────────

/// Claim the pending request, if any. The file is renamed away before it is
/// read, so a request arriving meanwhile is never lost with it.
fn take_from(dir: &Path) -> Option<Request> {
    let claimed = dir.join(format!("{REQUEST_FILE}.{}.claimed", std::process::id()));
    std::fs::rename(request_path(dir), &claimed).ok()?;
    let text = std::fs::read_to_string(&claimed);
    let _ = std::fs::remove_file(&claimed);
    Request::decode(&text.ok()?)
}

/// What a launch can do to the window. The real window implements it; tests use
/// a recorder.
pub trait Navigation {
    fn show_tab(&self, tab: i32);
    fn focus_keybindings_filter(&self);
    fn blink_setting(&self, label: &str);
}

impl Navigation for MainWindow {
    fn show_tab(&self, tab: i32) {
        self.set_active_tab(tab);
    }

    fn focus_keybindings_filter(&self) {
        self.set_kb_focus_filter(true);
    }

    fn blink_setting(&self, label: &str) {
        if self.get_active_tab() == 13 {
            match label {
                "Print History" => self.set_print_history(true),
                "Print Queue" | "Print Jobs" => self.set_print_history(false),
                _ => {}
            }
            self.invoke_print_refresh();
        }
        self.set_highlight_setting(label.into());
        crate::start_highlight_blink(self);
    }
}

/// Navigate to what a launch asked for: the first instance does this for its own
/// command line, a running one for each later launch.
pub fn apply(window: &impl Navigation, request: &Request) {
    if let Some(tab) = request.landing_tab() {
        window.show_tab(tab);
    }
    if request.focuses_keybindings_filter() {
        window.focus_keybindings_filter();
    }
    if !request.highlight.is_empty() {
        window.blink_setting(&request.highlight);
    }
}

/// Start answering requests from later launches.
pub fn watch(ui: &MainWindow) {
    let ui_weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, POLL_INTERVAL, move || {
        let Some(request) = take_from(&run_dir()) else {
            return;
        };
        if let Some(ui) = ui_weak.upgrade() {
            apply(&ui, &request);
        }
    });
    std::mem::forget(timer);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A scratch directory that is removed again when the test ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("settings-instance-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn request(tab: Option<i32>, highlight: &str) -> Request {
        Request {
            tab,
            highlight: highlight.into(),
        }
    }

    #[test]
    fn requests_round_trip_through_the_wire_format() {
        for sent in [
            request(Some(5), ""),
            request(Some(0), "Hostname"),
            request(Some(11), "Keypad Mapping"),
            request(None, "Hostname"),
            request(None, ""),
        ] {
            assert_eq!(Request::decode(&sent.encode()), Some(sent));
        }
        assert_eq!(request(Some(5), "").encode(), "5\n\n");
        assert_eq!(request(None, "Wi-Fi").encode(), "\nWi-Fi\n");
    }

    #[test]
    fn tab_only_requests_still_have_two_lines_for_older_builds() {
        // Older builds ignore anything with fewer than two lines.
        assert_eq!(request(Some(5), "").encode().lines().count(), 2);
        assert_eq!(request(Some(5), "").encode().lines().next(), Some("5"));
    }

    #[test]
    fn requests_written_by_older_builds_are_understood() {
        assert_eq!(
            Request::decode("3\nDisplay Scaling"),
            Some(request(Some(3), "Display Scaling"))
        );
        assert_eq!(Request::decode("7\nWi-Fi\n"), Some(request(Some(7), "Wi-Fi")));
    }

    #[test]
    fn unreadable_requests_are_rejected() {
        for text in ["", "abc\nWi-Fi", "5x\n", "99999999999999\n"] {
            assert_eq!(Request::decode(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_highlight_cannot_break_the_two_line_format() {
        let sent = request(Some(1), "one\ntwo\r\nthree");
        let received = Request::decode(&sent.encode()).unwrap();
        assert_eq!(received.tab, Some(1));
        assert_eq!(received.highlight, "one two  three");
    }

    #[test]
    fn launch_arguments_map_to_requests() {
        // `settings --tab keybindings`
        assert_eq!(Request::from_launch(Some(5), ""), request(Some(5), ""));
        // `settings --tab wifi`
        assert_eq!(Request::from_launch(Some(7), ""), request(Some(7), ""));
        // `settings --tab keybindings --highlight Keybindings`
        assert_eq!(
            Request::from_launch(Some(5), "Keybindings"),
            request(Some(5), "Keybindings")
        );
        // `settings --highlight Hostname`
        assert_eq!(
            Request::from_launch(None, "Hostname"),
            request(Some(0), "Hostname")
        );
        // plain `settings` only raises the window
        assert!(Request::from_launch(None, "").is_empty());
    }

    #[test]
    fn only_arriving_on_the_keybindings_tab_focuses_its_filter() {
        for (tab, highlight, focuses) in [
            (Some(KEYBINDINGS_TAB), "", true),
            (Some(KEYBINDINGS_TAB), "Keybindings", true),
            (Some(1), "", false),
            (Some(7), "", false),
            (Some(0), "Hostname", false),
            (None, "", false),
            (None, "Keybindings", false),
        ] {
            assert_eq!(
                Request::from_launch(tab, highlight).focuses_keybindings_filter(),
                focuses,
                "{tab:?} {highlight:?}"
            );
        }
        assert_eq!(KEYBINDINGS_TAB, 5);
    }

    #[test]
    fn a_request_is_handed_over_whole_and_only_once() {
        let dir = Scratch::new("handover");
        send_in(&dir.0, &request(Some(5), "")).unwrap();
        assert_eq!(dir.files(), [REQUEST_FILE]);
        assert_eq!(take_from(&dir.0), Some(request(Some(5), "")));
        assert_eq!(dir.files(), Vec::<String>::new());
        assert_eq!(take_from(&dir.0), None);
    }

    #[test]
    fn a_newer_request_replaces_one_that_was_not_picked_up() {
        let dir = Scratch::new("replace");
        send_in(&dir.0, &request(Some(3), "")).unwrap();
        send_in(&dir.0, &request(Some(5), "")).unwrap();
        assert_eq!(take_from(&dir.0), Some(request(Some(5), "")));
        assert_eq!(dir.files(), Vec::<String>::new());
    }

    #[test]
    fn a_garbled_request_file_is_discarded() {
        let dir = Scratch::new("garbled");
        std::fs::write(request_path(&dir.0), "not a tab\nat all").unwrap();
        assert_eq!(take_from(&dir.0), None);
        assert_eq!(dir.files(), Vec::<String>::new());
    }

    #[test]
    fn the_window_is_raised_only_after_the_request_was_picked_up() {
        let dir = Scratch::new("pickup");
        let path = dir.0.clone();
        let picked_up = std::thread::spawn({
            let path = path.clone();
            move || loop {
                if let Some(request) = take_from(&path) {
                    return request;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let raised = RefCell::new(None);
        hand_off_in(&dir.0, &request(Some(5), ""), Duration::from_secs(5), || {
            *raised.borrow_mut() = Some(request_path(&path).exists());
        });
        assert_eq!(raised.into_inner(), Some(false), "raised before pickup");
        assert_eq!(picked_up.join().unwrap(), request(Some(5), ""));
    }

    #[test]
    fn the_window_is_raised_even_if_nobody_picks_the_request_up() {
        let dir = Scratch::new("unclaimed");
        let raised = RefCell::new(false);
        let started = Instant::now();
        hand_off_in(&dir.0, &request(Some(5), ""), Duration::from_millis(60), || {
            *raised.borrow_mut() = true;
        });
        assert!(raised.into_inner());
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert_eq!(dir.files(), [REQUEST_FILE]);
    }

    /// Records what a launch did to the window.
    #[derive(Default)]
    struct RecordingWindow {
        calls: RefCell<Vec<String>>,
    }

    impl Navigation for RecordingWindow {
        fn show_tab(&self, tab: i32) {
            self.calls.borrow_mut().push(format!("tab {tab}"));
        }

        fn focus_keybindings_filter(&self) {
            self.calls.borrow_mut().push("focus filter".into());
        }

        fn blink_setting(&self, label: &str) {
            self.calls.borrow_mut().push(format!("blink {label}"));
        }
    }

    fn applied(request: &Request) -> Vec<String> {
        let window = RecordingWindow::default();
        apply(&window, request);
        window.calls.into_inner()
    }

    #[test]
    fn a_tab_only_launch_shows_the_tab_without_a_highlight_blink() {
        assert_eq!(applied(&Request::from_launch(Some(7), "")), ["tab 7"]);
        assert_eq!(applied(&Request::from_launch(Some(0), "")), ["tab 0"]);
    }

    #[test]
    fn a_launch_for_the_keybindings_tab_focuses_its_filter_without_a_blink() {
        // `settings --tab keybindings`, whether it is the first launch or a later one
        assert_eq!(
            applied(&Request::from_launch(Some(5), "")),
            ["tab 5", "focus filter"]
        );
    }

    #[test]
    fn a_highlight_still_blinks_after_the_tab_is_shown() {
        assert_eq!(
            applied(&Request::from_launch(Some(7), "Wi-Fi")),
            ["tab 7", "blink Wi-Fi"]
        );
        assert_eq!(
            applied(&Request::from_launch(None, "Hostname")),
            ["tab 0", "blink Hostname"]
        );
        assert_eq!(
            applied(&Request::from_launch(Some(5), "Keybindings")),
            ["tab 5", "focus filter", "blink Keybindings"]
        );
    }

    #[test]
    fn a_plain_launch_leaves_the_window_as_it_is() {
        assert_eq!(applied(&Request::from_launch(None, "")), Vec::<String>::new());
    }

    #[test]
    fn a_tab_only_request_sent_to_a_running_instance_reaches_the_window() {
        let dir = Scratch::new("tab-only");
        // What `settings --tab keybindings` leaves behind while Settings is
        // open on another tab; no `--highlight`.
        send_in(&dir.0, &Request::from_launch(Some(KEYBINDINGS_TAB), "")).unwrap();
        let received = take_from(&dir.0).expect("a tab-only request is accepted");
        assert_eq!(applied(&received), ["tab 5", "focus filter"]);
    }

    #[test]
    fn asking_again_for_the_tab_that_is_showing_asks_for_the_filter_again() {
        // Nothing here depends on which tab is showing: every request for the
        // Keybindings tab raises the focus request, so a second Super+K on the
        // open tab still lands in the filter.
        for _ in 0..2 {
            assert_eq!(
                applied(&Request::from_launch(Some(KEYBINDINGS_TAB), "")),
                ["tab 5", "focus filter"]
            );
        }
    }

    /// What the running window does with a later launch: the request goes through
    /// the hand-off file and is applied by the instance that picks it up.
    fn applied_by_a_running_window(request: &Request) -> Vec<String> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Tests run in parallel; each hand-off gets a directory of its own.
        static HAND_OFFS: AtomicUsize = AtomicUsize::new(0);
        let dir = Scratch::new(&format!(
            "running-window-{}",
            HAND_OFFS.fetch_add(1, Ordering::Relaxed)
        ));
        send_in(&dir.0, request).unwrap();
        let received = take_from(&dir.0).expect("the running window takes the request");
        applied(&received)
    }

    /// The first launch applies its own command line and later ones are handed to
    /// it; both must end up in the same place.
    fn applied_either_way(tab: i32, highlight: &str) -> Vec<String> {
        let launch = Request::from_launch(Some(tab), highlight);
        let first = applied(&launch);
        assert_eq!(
            applied_by_a_running_window(&launch),
            first,
            "--tab {tab} --highlight {highlight:?} differs once Settings is running"
        );
        first
    }

    #[test]
    fn connections_launches_land_on_the_right_section_in_a_new_and_a_running_window() {
        use crate::connections::{BLUETOOTH_SECTION, WIFI_SECTION};
        // `settings --tab wifi` and `--tab connections` (7), `--tab bluetooth` (8)
        assert_eq!(applied_either_way(WIFI_SECTION, ""), ["tab 7"]);
        assert_eq!(applied_either_way(BLUETOOTH_SECTION, ""), ["tab 8"]);
        // The page's own labels stay in the section that was asked for.
        assert_eq!(
            applied_either_way(WIFI_SECTION, "Airplane Mode"),
            ["tab 7", "blink Airplane Mode"]
        );
        assert_eq!(
            applied_either_way(BLUETOOTH_SECTION, "Airplane Mode"),
            ["tab 8", "blink Airplane Mode"]
        );
        // A label that belongs to the other section wins over the tab name.
        assert_eq!(
            applied_either_way(WIFI_SECTION, "Headphones"),
            ["tab 8", "blink Headphones"]
        );
        assert_eq!(
            applied_either_way(BLUETOOTH_SECTION, "Connect Wi-Fi"),
            ["tab 7", "blink Connect Wi-Fi"]
        );
    }

    #[test]
    fn every_connections_label_opens_its_section_whichever_tab_name_launched_it() {
        use crate::connections::{BLUETOOTH_LABELS, BLUETOOTH_SECTION, WIFI_LABELS, WIFI_SECTION};
        for tab in [WIFI_SECTION, BLUETOOTH_SECTION] {
            for (labels, section) in [
                (&WIFI_LABELS[..], WIFI_SECTION),
                (&BLUETOOTH_LABELS[..], BLUETOOTH_SECTION),
            ] {
                for label in labels {
                    assert_eq!(
                        applied_either_way(tab, label),
                        [format!("tab {section}"), format!("blink {label}")],
                        "--tab {tab} --highlight {label:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn only_the_connections_tab_is_redirected_by_a_connections_label() {
        assert_eq!(
            applied_either_way(3, "Bluetooth"),
            ["tab 3", "blink Bluetooth"]
        );
        assert_eq!(
            applied_either_way(KEYBINDINGS_TAB, "Wi-Fi"),
            ["tab 5", "focus filter", "blink Wi-Fi"]
        );
    }

    #[test]
    fn the_request_names_the_tab_as_launched_so_older_builds_read_it_too() {
        // The section is chosen by the window that applies the request, so the
        // file still carries `--tab wifi` as the tab it always was.
        let sent = Request::from_launch(Some(7), "Headphones");
        assert_eq!(sent.encode(), "7\nHeadphones\n");
        assert_eq!(sent.landing_tab(), Some(8));
        assert_eq!(Request::from_launch(None, "").landing_tab(), None);
    }

    #[test]
    fn a_plain_launch_only_raises_the_window() {
        let dir = Scratch::new("plain");
        let raised = RefCell::new(false);
        hand_off_in(&dir.0, &Request::from_launch(None, ""), Duration::from_secs(5), || {
            *raised.borrow_mut() = true;
        });
        assert!(raised.into_inner());
        assert_eq!(dir.files(), Vec::<String>::new());
    }

    #[test]
    fn an_unwritable_runtime_directory_still_raises_the_window() {
        let dir = Scratch::new("missing");
        let missing = dir.0.join("nowhere");
        let raised = RefCell::new(false);
        hand_off_in(&missing, &request(Some(5), ""), Duration::from_secs(5), || {
            *raised.borrow_mut() = true;
        });
        assert!(raised.into_inner());
    }

    const CLIENTS: &str = r#"[
        {"class":"settings-helper","mapped":true,"address":"0x11"},
        {"class":"settings","mapped":false,"address":"0x22"},
        {"class":"settings","mapped":true,"address":"0xab12"}
    ]"#;

    /// Replies to compositor requests in order and records what was asked.
    fn scripted<'a>(
        asked: &'a RefCell<Vec<String>>,
        replies: &'a [Result<&'a str, &'a str>],
    ) -> impl FnMut(&str) -> Result<String, String> + 'a {
        move |request| {
            let mut asked = asked.borrow_mut();
            let reply = replies[asked.len()];
            asked.push(request.to_string());
            reply.map(str::to_string).map_err(str::to_string)
        }
    }

    #[test]
    fn lua_hyprland_focuses_the_mapped_settings_window_by_address() {
        let asked = RefCell::new(Vec::new());
        let replies = [Ok(CLIENTS), Ok("ok")];
        assert_eq!(focus_with(scripted(&asked, &replies), "settings"), Ok(true));
        assert_eq!(
            *asked.borrow(),
            [
                "j/clients",
                r#"dispatch hl.dsp.focus({window="address:0xab12"})"#
            ]
        );
    }

    #[test]
    fn hyprlang_hyprland_falls_back_to_the_classic_dispatcher() {
        let asked = RefCell::new(Vec::new());
        let replies = [Ok(CLIENTS), Ok("Invalid dispatcher\n"), Ok("ok\n")];
        assert_eq!(focus_with(scripted(&asked, &replies), "settings"), Ok(true));
        assert_eq!(
            *asked.borrow(),
            [
                "j/clients",
                r#"dispatch hl.dsp.focus({window="address:0xab12"})"#,
                "dispatch focuswindow address:0xab12"
            ]
        );
    }

    #[test]
    fn a_refused_focus_is_an_error_not_a_success() {
        let asked = RefCell::new(Vec::new());
        let replies = [Ok(CLIENTS), Ok("window not found")];
        let error = focus_with(scripted(&asked, &replies), "settings").unwrap_err();
        assert!(error.contains("window not found"), "{error}");
    }

    #[test]
    fn compositor_errors_are_reported() {
        let asked = RefCell::new(Vec::new());
        let replies = [Err("socket closed")];
        assert_eq!(
            focus_with(scripted(&asked, &replies), "settings"),
            Err("socket closed".to_string())
        );
        assert_eq!(asked.borrow().len(), 1);
    }

    #[test]
    fn nothing_is_dispatched_when_there_is_no_settings_window() {
        let asked = RefCell::new(Vec::new());
        let replies = [Ok(r#"[{"class":"kitty","mapped":true,"address":"0x1"}]"#)];
        assert_eq!(focus_with(scripted(&asked, &replies), "settings"), Ok(false));
        assert_eq!(*asked.borrow(), ["j/clients"]);
        assert_eq!(window_address("[]", "settings"), Ok(None));
    }

    #[test]
    fn only_well_formed_window_addresses_are_dispatched() {
        for address in ["", "0x", "ab12", "0xzz", "0x1 ; reload", "0x1\"})"] {
            let json = format!(r#"[{{"class":"settings","mapped":true,"address":"{}"}}]"#,
                address.replace('"', "\\\""));
            assert!(window_address(&json, "settings").is_err(), "{address:?}");
        }
        assert!(window_address(r#"[{"class":"settings","mapped":true}]"#, "settings").is_err());
        assert!(window_address("not json", "settings").is_err());
        assert_eq!(
            window_address(CLIENTS, "settings"),
            Ok(Some("0xab12".to_string()))
        );
    }

    #[test]
    fn the_compositor_socket_stays_inside_the_runtime_directory() {
        let runtime = Path::new("/run/user/1000");
        assert_eq!(
            hyprland_socket(runtime, "abc_123"),
            Ok(PathBuf::from("/run/user/1000/hypr/abc_123/.socket.sock"))
        );
        for signature in ["", ".", "..", "../x", "a/b"] {
            assert!(hyprland_socket(runtime, signature).is_err(), "{signature:?}");
        }
    }
}
