use chrono::{DateTime, Local};
use serde_json::Value;
use std::process::Command;

const MAX_NOTIFICATIONS: usize = 50;

#[derive(Clone, Debug)]
pub struct Notification {
    pub id: i32,
    pub appname: String,
    pub desktop_entry: String,
    pub icon: String,
    pub summary: String,
    pub body: String,
    pub date: String,
    pub time: String,
    pub action: String,
}

/// Map well-known notification summaries to shell commands.
/// These are notifications sent by smplOS scripts (first-run, etc.)
/// that should be actionable from the notif-center even though
/// dunst history doesn't preserve the original dunst action.
fn action_for_notification(appname: &str, summary: &str, body: &str) -> String {
    debug_log(&format!(
        "action_for_notification: appname={appname:?} summary={summary:?} body={body:?}"
    ));

    if summary == "System Update" {
        debug_log("action_for_notification -> smplos-update");
        return "smplos-update".to_string();
    }

    let a = appname.to_lowercase();
    let s = summary.to_lowercase();
    let b = body.to_lowercase();
    // Match by appname (most reliable — set via notify-send -a),
    // or by summary/body text as fallback.
    if a == "launch-webapp"
        || s.contains("web app launch error")
        || b.contains("launch-webapp.log")
    {
        debug_log("action_for_notification -> __OPEN_WEBAPP_LOG__");
        return "__OPEN_WEBAPP_LOG__".to_string();
    }

    debug_log("action_for_notification -> (no action)");
    String::new()
}

pub(crate) fn debug_log(msg: &str) {
    let home = std::env::var("HOME").unwrap_or_default();
    let cache = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| format!("{home}/.cache"));
    let ts = chrono::Local::now().format("%F %T%.3f");
    let line = format!("{ts} {msg}\n");
    // Write to ~/.cache only (no /mnt host-share logging)
    let cache_path = format!("{cache}/smplos/notif-center-debug.log");
    let _ = std::fs::OpenOptions::new().create(true).append(true).open(&cache_path)
        .map(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// Session environment forwarded to a detached action child on top of
/// the fixed PATH/HOME after `env_clear`.
///
/// The installed focus-or-launch helper contract requires the session
/// discriminators: its IS_HYPRLAND flag is true only when
/// HYPRLAND_INSTANCE_SIGNATURE is nonempty (its envs.conf fallback may
/// not carry the runtime-generated signature), and gtk-launch
/// D-Bus activation needs DBUS_SESSION_BUS_ADDRESS. Empty values are
/// omitted so the child never receives empty discriminators.
pub fn session_env_passthrough(
    wayland_display: &str,
    display: &str,
    xdg_runtime_dir: &str,
    hyprland_signature: &str,
    dbus_session_bus_address: &str,
) -> Vec<(String, String)> {
    [
        ("WAYLAND_DISPLAY", wayland_display),
        ("DISPLAY", display),
        ("XDG_RUNTIME_DIR", xdg_runtime_dir),
        ("HYPRLAND_INSTANCE_SIGNATURE", hyprland_signature),
        ("DBUS_SESSION_BUS_ADDRESS", dbus_session_bus_address),
    ]
    .iter()
    .filter(|(_, value)| !value.is_empty())
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

fn spawn_detached(args: &[&str]) -> bool {
    if args.is_empty() {
        debug_log("spawn_detached: ABORT - empty args");
        return false;
    }

    debug_log(&format!("spawn_detached ENTRY: args={args:?}"));

    // Check binary existence
    let bin = args[0];
    let bin_exists = std::path::Path::new(bin).exists();
    debug_log(&format!("spawn_detached: bin={bin:?} exists={bin_exists}"));
    if !bin_exists {
        // Try to find it in PATH as a fallback hint
        let which = Command::new("which").arg(bin).output();
        let which_out = which.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "which-failed".to_string());
        debug_log(&format!("spawn_detached: which {bin:?} => {which_out:?}"));
    }

    // Collect env
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let xdg_runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    // Session discriminators the installed focus-or-launch helper contract
    // needs: its IS_HYPRLAND flag is true only when the signature is
    // nonempty (its envs.conf fallback may not carry the runtime-generated
    // signature), and gtk-launch D-Bus activation needs the session bus.
    let hyprland_signature =
        std::env::var("HYPRLAND_INSTANCE_SIGNATURE").unwrap_or_default();
    let dbus_session_bus_address =
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("PATH")
        .unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".to_string());
    let path = if path.contains("/usr/local/bin") {
        path
    } else {
        format!("/usr/local/bin:{path}")
    };

    debug_log(&format!(
        "spawn_detached: env WAYLAND={wayland_display:?} DISPLAY={display:?} \
         XDG_RUNTIME_DIR={xdg_runtime_dir:?} HYPRLAND={hyprland_signature:?} \
         DBUS={dbus_session_bus_address:?} HOME={home:?} PATH={path:?}"
    ));

    // Redirect stderr to cache debug log file so we can see child process errors
    let stderr_path = format!("{home}/.cache/smplos/notif-spawn-stderr.log");
    let _ = std::fs::create_dir_all(format!("{home}/.cache/smplos"));
    let stderr_file = std::fs::OpenOptions::new()
        .create(true).append(true)
        .open(stderr_path)
        .ok()
        .map(std::process::Stdio::from)
        .unwrap_or_else(std::process::Stdio::null);

    // Spawn directly (no setsid) so stderr is captured
    let mut cmd = Command::new(bin);
    if args.len() > 1 {
        cmd.args(&args[1..]);
    }
    cmd.env_clear()
        .env("PATH", &path)
        .env("HOME", &home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_file);
    for (key, value) in session_env_passthrough(
        &wayland_display,
        &display,
        &xdg_runtime_dir,
        &hyprland_signature,
        &dbus_session_bus_address,
    ) {
        cmd.env(&key, &value);
    }

    let result = cmd.spawn();
    match &result {
        Ok(child) => debug_log(&format!("spawn_detached: spawned OK pid={}", child.id())),
        Err(e) => debug_log(&format!("spawn_detached: FAILED to spawn: {e}")),
    }
    result.is_ok()
}

fn open_webapp_log() -> bool {
    debug_log("open_webapp_log: ENTRY");

    let home = std::env::var("HOME").unwrap_or_default();
    let cache = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| format!("{home}/.cache"));
    let log_path = format!("{cache}/smplos/launch-webapp.log");
    let log_exists = std::path::Path::new(&log_path).exists();
    debug_log(&format!("open_webapp_log: log_path={log_path:?} exists={log_exists}"));

    // Kill orphaned terminal windows before opening a new one.
    // Previous spawns may have left invisible st-wl windows on top of notif-center.
    let _ = Command::new("pkill").args(["-x", "st-wl"]).output();
    debug_log("open_webapp_log: pkill st-wl done");

    // STRATEGY: use hyprctl dispatch exec which gives proper Hyprland floating placement.
    // This is far more reliable than setsid/fork because Hyprland manages the window.
    let hyprctl = "/usr/bin/hyprctl";
    if std::path::Path::new(hyprctl).exists() {
        // Kill any existing terminal windows first via hyprctl
        let _ = Command::new(hyprctl)
            .args(["dispatch", "killwindow", "class:terminal"])
            .output();
        debug_log("open_webapp_log: hyprctl killwindow done");

        let exec_cmd = if log_exists {
            format!("terminal -e micro {log_path}")
        } else {
            // Log doesn't exist yet — open a terminal showing an error message
            format!("terminal -e sh -c 'echo \"Log file not found: {log_path}\"; echo; echo \"Press Enter to close\"; read'")
        };
        let dispatch_rule = "[float;size 900 600;center]".to_string();
        debug_log(&format!("open_webapp_log: hyprctl dispatch exec {dispatch_rule:?} {exec_cmd:?}"));
        let result = Command::new(hyprctl)
            .args(["dispatch", "exec", &format!("{dispatch_rule} {exec_cmd}")])
            .output();
        match &result {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let stderr = String::from_utf8_lossy(&o.stderr);
                debug_log(&format!("open_webapp_log: hyprctl result status={} stdout={stdout:?} stderr={stderr:?}", o.status));
            }
            Err(e) => debug_log(&format!("open_webapp_log: hyprctl FAILED: {e}")),
        }
        return result.is_ok();
    }

    // Non-Hyprland fallback: delegate to open-log-file shell script
    debug_log("open_webapp_log: no hyprctl, falling back to open-log-file script");
    let script = "/usr/local/bin/open-log-file";
    let result = if std::path::Path::new(script).exists() {
        spawn_detached(&[script, &log_path])
    } else {
        // Last resort: zenity to at least show the path
        let msg = format!("Web App Log:\n{log_path}");
        spawn_detached(&["/usr/bin/zenity", "--info", "--title=Web App Error Log", &format!("--text={msg}")])
    };
    debug_log(&format!("open_webapp_log: fallback result={result}"));
    result
}

fn icon_for_app(app: &str) -> String {
    match app.to_lowercase().as_str() {
        "signal" => "󰍡",
        "discord" => "󰙯",
        "brave" | "brave-browser" => "󰖟",
        "spotify" => "󰓇",
        "thunderbird" => "󰇰",
        "steam" => "󰓓",
        "notify-send" => "󰂚",
        "dunstctl" => "󰂚",
        "volume" | "volume-ctl" => "󰕾",
        "brightness" | "brightness-ctl" => "󰃟",
        _ => "󰂚",
    }
    .to_string()
}

fn parse_epoch_from_dunst(raw_timestamp_us: i64) -> Option<i64> {
    let now = Local::now().timestamp();
    let uptime_s = std::fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()? as i64;

    // Dunst timestamp is microseconds since boot.
    Some(now - (uptime_s - (raw_timestamp_us / 1_000_000)))
}

fn get_str_path<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

fn get_i64_path(v: &Value, path: &[&str]) -> Option<i64> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_i64()
}

pub fn get_notifications() -> Vec<Notification> {
    let output = Command::new("dunstctl").arg("history").output();
    let Ok(out) = output else {
        debug_log("get_notifications: dunstctl failed to run");
        return Vec::new();
    };

    if !out.status.success() {
        debug_log(&format!("get_notifications: dunstctl exited {}", out.status));
        return Vec::new();
    }

    let Ok(json) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };

    let Some(items) = json
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut notifications = Vec::new();

    for item in items {
        let id = get_i64_path(item, &["id", "data"]).unwrap_or(0) as i32;
        let appname = get_str_path(item, &["appname", "data"]).unwrap_or("Unknown").to_string();
        let desktop_entry =
            get_str_path(item, &["desktop_entry", "data"]).unwrap_or("").to_string();
        let summary = get_str_path(item, &["summary", "data"]).unwrap_or("").to_string();
        let body = get_str_path(item, &["body", "data"]).unwrap_or("").to_string();

        if summary.is_empty() && body.is_empty() {
            continue;
        }

        let raw_ts_us = get_i64_path(item, &["timestamp", "data"]).unwrap_or(0);
        let epoch = parse_epoch_from_dunst(raw_ts_us).unwrap_or_else(|| Local::now().timestamp());
        let dt: DateTime<Local> = DateTime::from_timestamp(epoch, 0)
            .map(|dt| dt.with_timezone(&Local))
            .unwrap_or_else(Local::now);

        let action = action_for_notification(&appname, &summary, &body);
        debug_log(&format!(
            "get_notifications: parsed notif id={id} appname={appname:?} \
             desktop_entry={desktop_entry:?} summary={summary:?} action={action:?}"
        ));
        notifications.push(Notification {
            id,
            appname: appname.clone(),
            desktop_entry,
            icon: icon_for_app(&appname),
            summary,
            body,
            date: dt.format("%b %d").to_string(),
            time: dt.format("%I:%M %p").to_string().trim_start_matches('0').to_string(),
            action,
        });
    }

    notifications.truncate(MAX_NOTIFICATIONS);
    notifications
}

pub fn dismiss_notification(id: i32) {
    let _ = Command::new("dunstctl")
        .arg("history-rm")
        .arg(id.to_string())
        .status();
}

pub fn clear_all_notifications() {
    let _ = Command::new("dunstctl").arg("history-clear").status();
}

/// Outcome of dispatching a notification click.
///
/// `Dispatched` only proves the resolved command was started; it is not
/// proof that the helper/app finished its work. `Failed` means no effect
/// could be started (nothing to dispatch, or the executor reported
/// failure). `Stale` means the clicked id is no longer in the current
/// list/history snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    Dispatched,
    Failed,
    Stale,
}

/// Resolved effect for a notification click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchCmd {
    /// `__OPEN_WEBAPP_LOG__` special action: open the web-app error log.
    WebappLog,
    /// Named shell action (e.g. `smplos-update`) run via `sh -lc`.
    ShellAction { action: String },
    /// Default path: installed focus-or-launch helper with best target.
    FocusOrLaunch { target: String },
    /// No action, no desktop-entry, no appname: nothing to dispatch.
    None,
}

/// Pure decision: map a notification's dispatch fields to an effect.
///
/// No side effects, so unit tests can verify the precedence
/// (special action > named action > desktop_entry > appname) without
/// touching the real helper, bus, apps or history.
pub fn decide_dispatch(appname: &str, desktop_entry: &str, action: &str) -> DispatchCmd {
    if action == "__OPEN_WEBAPP_LOG__" {
        return DispatchCmd::WebappLog;
    }

    // 1. Named action command (e.g. smplos-update)
    if !action.is_empty() {
        return DispatchCmd::ShellAction { action: action.to_string() };
    }

    // 2. Use focus-or-launch which handles both focusing existing windows
    //    AND launching the app from its .desktop file.
    //    Prefer desktop_entry (most specific), fall back to appname.
    let target = if !desktop_entry.is_empty() {
        desktop_entry.trim_end_matches(".desktop").to_string()
    } else if !appname.is_empty() {
        appname.to_string()
    } else {
        return DispatchCmd::None;
    };
    DispatchCmd::FocusOrLaunch { target }
}

/// Execute a resolved effect for real (production path only; tests
/// inject their own `run` instead of calling this).
pub fn execute_dispatch(cmd: &DispatchCmd) -> bool {
    match cmd {
        DispatchCmd::WebappLog => open_webapp_log(),
        DispatchCmd::ShellAction { action } => {
            Command::new("sh").arg("-lc").arg(action).spawn().is_ok()
        }
        DispatchCmd::FocusOrLaunch { target } => {
            debug_log(&format!(
                "execute_dispatch: calling focus-or-launch {target:?}"
            ));
            let fol = "/usr/local/bin/focus-or-launch";
            spawn_detached(&[fol, target])
        }
        DispatchCmd::None => false,
    }
}

/// Shared click-to-effect wiring used by the Slint `activate` callback:
/// look the clicked id up in the current notification list, resolve its
/// effect and hand it to `run`.
///
/// `run` is injected so tests can fake every process effect. The
/// outcome distinguishes stale ids from dispatch failures so the click
/// path can no longer fail silently.
pub fn dispatch_notification(
    notifications: &[Notification],
    id: i32,
    run: &mut dyn FnMut(&DispatchCmd) -> bool,
) -> DispatchOutcome {
    let Some(n) = notifications.iter().find(|n| n.id == id) else {
        debug_log(&format!("dispatch_notification: stale id={id} not in current list"));
        return DispatchOutcome::Stale;
    };

    let cmd = decide_dispatch(&n.appname, &n.desktop_entry, &n.action);
    if matches!(cmd, DispatchCmd::None) {
        debug_log(&format!(
            "dispatch_notification: id={} appname={} desktop_entry={} has no target",
            n.id, n.appname, n.desktop_entry
        ));
        return DispatchOutcome::Failed;
    }

    let started = run(&cmd);
    debug_log(&format!(
        "dispatch_notification: id={id} cmd={cmd:?} started={started}"
    ));
    if started {
        DispatchOutcome::Dispatched
    } else {
        DispatchOutcome::Failed
    }
}
