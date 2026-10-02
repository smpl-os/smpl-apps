use crate::MainWindow;
use anyhow::{ensure, Context, Result};
use slint::ComponentHandle;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::OpenOptionsExt, net::UnixStream},
    },
    path::Path,
    process::Command,
    time::{Duration, Instant},
};

pub fn acquire_compact() -> Result<Option<File>> {
    acquire_compact_named("calendar")
}

pub fn acquire_compact_named(name: &str) -> Result<Option<File>> {
    ensure!(
        name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
        "Invalid calendar instance name"
    );
    let directory = dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .context("No directory for the calendar instance lock")?
        .join("smplos");
    ensure!(
        directory.is_absolute(),
        "Calendar lock directory must be absolute"
    );
    std::fs::create_dir_all(&directory)?;
    let display = std::env::var("WAYLAND_DISPLAY")
        .or_else(|_| std::env::var("DISPLAY"))
        .unwrap_or_else(|_| "default".into());
    let display: String = display
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    acquire_lock(&directory.join(format!("{name}-{display}.lock")))
}

fn acquire_lock(path: &Path) -> Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "Calendar instance lock is not a regular file"
    );
    // Keep this descriptor for the compact process lifetime; never unlink a live lock.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(None);
    }
    Err(error).context("Could not lock the calendar instance")
}

fn hyprland_request(request: &str) -> Result<String> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")?;
    ensure!(
        !signature.is_empty() && !signature.contains('/') && signature != "." && signature != "..",
        "Invalid compositor instance"
    );
    let socket = dirs::runtime_dir()
        .context("No compositor runtime directory")?
        .join("hypr")
        .join(signature)
        .join(".socket.sock");
    let mut stream = UnixStream::connect(socket).context("Could not connect to the compositor")?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    stream.write_all(request.as_bytes())?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = String::new();
    stream.take(256 * 1024 + 1).read_to_string(&mut response)?;
    ensure!(
        response.len() <= 256 * 1024,
        "Compositor response is too large"
    );
    Ok(response)
}

fn compact_address(bytes: &str, class: &str) -> Result<Option<String>> {
    let clients: Vec<serde_json::Value> = serde_json::from_str(bytes)?;
    for client in clients {
        if client.get("class").and_then(|v| v.as_str()) == Some(class)
            && client.get("mapped").and_then(|v| v.as_bool()) == Some(true)
        {
            let address = client
                .get("address")
                .and_then(|v| v.as_str())
                .context("Calendar window has no compositor address")?;
            ensure!(
                address.starts_with("0x")
                    && address.len() > 2
                    && address[2..].chars().all(|c| c.is_ascii_hexdigit()),
                "Invalid calendar window address"
            );
            return Ok(Some(address.into()));
        }
    }
    Ok(None)
}

pub fn focus_compact() -> Result<bool> {
    focus_class("smpl-calendar")
}

fn focus_class(class: &str) -> Result<bool> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        return Ok(false);
    }
    focus_with(hyprland_request, class)
}

fn focus_with(mut request: impl FnMut(&str) -> Result<String>, class: &str) -> Result<bool> {
    let Some(address) = compact_address(&request("j/clients")?, class)? else {
        return Ok(false);
    };
    // Lua-configured Hyprland interprets dispatch arguments as Lua expressions.
    let response = request(&format!(
        "dispatch hl.dsp.focus({{window=\"address:{address}\"}})"
    ))?;
    let response = if response.trim() == "Invalid dispatcher" {
        request(&format!("dispatch focuswindow address:{address}"))?
    } else {
        response
    };
    ensure!(
        response.trim() == "ok",
        "Could not focus calendar: {}",
        response.trim()
    );
    Ok(true)
}

fn return_to_compact(executable: &Path, class: &str) -> Result<()> {
    if focus_class(class)? {
        return Ok(());
    }
    let mut child = Command::new(executable)
        .spawn()
        .context("Could not launch the calendar popup")?;
    let deadline = Instant::now() + Duration::from_secs(4);
    let compositor = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(
                status.success(),
                "Calendar popup exited before opening ({status})"
            );
            ensure!(compositor, "Calendar popup exited before opening");
        }
        if compositor {
            if focus_class(class)? {
                return Ok(());
            }
        } else if deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(3) {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Calendar is not ready; the detailed view was kept open"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub fn wire_back(ui: &MainWindow) {
    wire_back_for_class(ui, "smpl-calendar");
}

pub fn wire_back_for_class(ui: &MainWindow, compact_class: &'static str) {
    let weak = ui.as_weak();
    let launch = Instant::now();
    ui.on_close_details(move || {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.get_is_details() || ui.get_navigation_busy() || launch.elapsed() < Duration::from_millis(1500) {
            return;
        }
        if ui.get_show_form() {
            ui.set_navigation_status("Save or cancel the open event before returning to the calendar.".into());
            return;
        }
        ui.set_navigation_status("".into());
        ui.set_navigation_busy(true);
        let weak = ui.as_weak();
        std::thread::spawn(move || {
            let result = std::env::current_exe().context("Could not locate the calendar executable")
                .and_then(|executable| return_to_compact(&executable, compact_class));
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_navigation_busy(false);
                match result {
                    Ok(()) => {
                        if ui.get_show_form() {
                            ui.set_navigation_status("Calendar opened. Save or cancel your event before leaving details.".into());
                            return;
                        }
                        // Only this details process exits; other calendar windows are untouched.
                        if let Err(error) = slint::quit_event_loop() {
                            eprintln!("[calendar] Could not close details: {error}");
                            ui.set_navigation_status(format!("Could not close details: {error}").into());
                        }
                    }
                    Err(error) => {
                        eprintln!("[calendar] Back to calendar: {error:#}");
                        ui.set_navigation_status(format!("Could not return to calendar: {error:#}").into());
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_mapped_compact_windows_are_focus_targets() {
        assert_eq!(compact_address(r#"[{"class":"smpl-calendar-details","mapped":true,"address":"0x12"},{"class":"smpl-calendar","mapped":true,"address":"0xab"}]"#, "smpl-calendar").unwrap(), Some("0xab".into()));
        assert_eq!(
            compact_address(
                r#"[{"class":"smpl-calendar","mapped":false,"address":"0xab"}]"#,
                "smpl-calendar"
            )
            .unwrap(),
            None
        );
        assert!(compact_address(
            r#"[{"class":"smpl-calendar","mapped":true,"address":"class:other"}]"#,
            "smpl-calendar"
        )
        .is_err());
    }

    #[test]
    fn compact_lock_prevents_duplicates_and_releases_on_exit() {
        let directory = std::env::temp_dir().join(format!(
            "calendar-lock-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("instance.lock");
        let first = acquire_lock(&path).unwrap().unwrap();
        assert!(acquire_lock(&path).unwrap().is_none());
        drop(first);
        let second = acquire_lock(&path).unwrap().unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn existing_compact_is_focused_by_exact_address_without_a_launch() {
        let mut requests = vec![];
        assert!(focus_with(
            |request| {
                requests.push(request.to_string());
                Ok(if request == "j/clients" {
                    r#"[{"class":"smpl-calendar","mapped":true,"address":"0x123"}]"#.into()
                } else {
                    "ok\n".into()
                })
            },
            "smpl-calendar"
        )
        .unwrap());
        assert_eq!(
            requests,
            [
                "j/clients",
                "dispatch hl.dsp.focus({window=\"address:0x123\"})"
            ]
        );
        assert!(!focus_with(
            |request| {
                assert_eq!(request, "j/clients");
                Ok("[]".into())
            },
            "smpl-calendar"
        )
        .unwrap());
        assert!(focus_with(
            |_| Err(anyhow::anyhow!("compositor unavailable")),
            "smpl-calendar"
        )
        .is_err());
    }

    #[test]
    fn legacy_focus_fallback_is_only_for_an_unsupported_lua_dispatcher() {
        let mut requests = vec![];
        assert!(focus_with(
            |request| {
                requests.push(request.to_string());
                Ok(match requests.len() {
                    1 => r#"[{"class":"smpl-calendar","mapped":true,"address":"0x123"}]"#.into(),
                    2 => "Invalid dispatcher".into(),
                    _ => "ok".into(),
                })
            },
            "smpl-calendar"
        )
        .unwrap());
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2], "dispatch focuswindow address:0x123");
        let mut calls = 0;
        assert!(focus_with(
            |_| {
                calls += 1;
                Ok(if calls == 1 {
                    r#"[{"class":"smpl-calendar","mapped":true,"address":"0x123"}]"#.into()
                } else {
                    "Window not found".into()
                })
            },
            "smpl-calendar"
        )
        .is_err());
        assert_eq!(calls, 2);
    }
}
