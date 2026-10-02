//! Native compositor-managed sizing for the undecorated details window.

use crate::MainWindow;
use anyhow::{ensure, Context, Result};
use i_slint_backend_winit::{
    winit::{event::WindowEvent, window::ResizeDirection},
    EventResult, WinitWindowAccessor,
};
use serde::Deserialize;
use slint::ComponentHandle;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

fn direction(edge: i32) -> Option<ResizeDirection> {
    Some(match edge {
        0 => ResizeDirection::North,
        1 => ResizeDirection::East,
        2 => ResizeDirection::South,
        3 => ResizeDirection::West,
        4 => ResizeDirection::NorthWest,
        5 => ResizeDirection::NorthEast,
        6 => ResizeDirection::SouthWest,
        7 => ResizeDirection::SouthEast,
        _ => return None,
    })
}

/// Retain the returned timer for authoritative Hyprland state synchronization.
pub fn wire(ui: &MainWindow) -> Option<slint::Timer> {
    let weak = ui.as_weak();
    ui.on_resize_window(move |edge| {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.get_is_details() {
            return;
        }
        let Some(direction) = direction(edge) else {
            return;
        };
        // This callback is invoked on pointer-down while Wayland's press serial
        // is current. The compositor owns motion, constraints and final geometry.
        match ui
            .window()
            .with_winit_window(|window| window.drag_resize_window(direction))
        {
            Some(Ok(())) => {}
            Some(Err(error)) => eprintln!("[calendar] Native resize failed: {error}"),
            None => eprintln!("[calendar] Native resize window is unavailable"),
        }
    });

    if ui.get_is_details() {
        if let Some(socket) = hyprland_socket() {
            return Some(wire_hyprland(ui, socket));
        }
    }

    let weak = ui.as_weak();
    ui.on_toggle_details_maximized(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_is_details() {
            ui.window()
                .with_winit_window(|window| window.set_maximized(!window.is_maximized()));
        }
    });

    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        if matches!(
            event,
            WindowEvent::Resized(_) | WindowEvent::Focused(_) | WindowEvent::RedrawRequested
        ) {
            if let Some(ui) = weak.upgrade() {
                if ui.get_is_details() {
                    if let Some(maximized) =
                        window.with_winit_window(|native| native.is_maximized())
                    {
                        ui.set_details_maximized(maximized);
                    }
                }
            }
        }
        EventResult::Propagate
    });
    None
}

fn hyprland_socket() -> Option<PathBuf> {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("WAYLAND_SOCKET").is_none()
    {
        return None;
    }
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    if signature.is_empty() || signature.contains('/') || signature.contains("..") {
        return None;
    }
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    runtime
        .is_absolute()
        .then(|| runtime.join("hypr").join(signature).join(".socket.sock"))
}

#[derive(Debug, Deserialize)]
struct HyprWindow {
    address: String,
    pid: i64,
    mapped: bool,
    fullscreen: u8,
}

fn own_window(bytes: &[u8], pid: u32) -> Result<Option<HyprWindow>> {
    let windows: Vec<HyprWindow> =
        serde_json::from_slice(bytes).context("Invalid compositor window state")?;
    let mut owned = windows
        .into_iter()
        .filter(|window| window.mapped && window.pid == i64::from(pid));
    let Some(window) = owned.next() else {
        return Ok(None);
    };
    ensure!(
        owned.next().is_none(),
        "More than one mapped window belongs to this process"
    );
    ensure!(window.fullscreen <= 2, "Invalid compositor fullscreen mode");
    let address = window
        .address
        .strip_prefix("0x")
        .context("Invalid compositor address")?;
    ensure!(
        !address.is_empty() && address.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid compositor address"
    );
    Ok(Some(window))
}

fn state_request(window: &HyprWindow, pid: u32) -> String {
    let mode = if window.fullscreen == 0 { 1 } else { 0 };
    // Recheck identity in the compositor, in the same operation that changes
    // state. Never dispatch against whichever unrelated window is active.
    format!(
        "/eval local w=hl.get_window(\"address:{}\"); \
         if not w or w.pid ~= {} then error(\"Calendar window identity changed\") end; \
         hl.dispatch(hl.dsp.window.fullscreen_state({{internal={},client={},action=\"set\",window=w}}))",
        window.address, pid, mode, mode,
    )
}

fn ipc(socket: &Path, request: &str) -> Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket).context("Could not connect to compositor")?;
    let deadline = Instant::now() + Duration::from_millis(500);
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(request.as_bytes())?;
    let mut response = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("Compositor request timed out")?;
        stream.set_read_timeout(Some(remaining))?;
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        ensure!(
            response.len() + count <= 1024 * 1024,
            "Compositor response is too large"
        );
        response.extend_from_slice(&chunk[..count]);
    }
    Ok(response)
}

fn compositor_state(socket: &Path, pid: u32, toggle: bool) -> Result<Option<bool>> {
    let Some(window) = own_window(&ipc(socket, "j/clients")?, pid)? else {
        return Ok(None);
    };
    if toggle {
        let reply = ipc(socket, &state_request(&window, pid))?;
        ensure!(
            std::str::from_utf8(&reply)?.trim() == "ok",
            "Compositor rejected window-state request"
        );
        return Ok(
            own_window(&ipc(socket, "j/clients")?, pid)?.map(|window| window.fullscreen != 0)
        );
    }
    Ok(Some(window.fullscreen != 0))
}

fn wire_hyprland(ui: &MainWindow, socket: PathBuf) -> slint::Timer {
    let pid = std::process::id();
    let (requests, receiver) = mpsc::sync_channel(1);
    let (results, updates) = mpsc::channel();
    let toggle = Arc::new(AtomicBool::new(false));
    let pending = toggle.clone();
    std::thread::spawn(move || {
        while receiver.recv().is_ok() {
            let result = compositor_state(&socket, pid, pending.swap(false, Ordering::AcqRel))
                .map_err(|error| error.to_string());
            if results.send(result).is_err() {
                break;
            }
        }
    });
    let weak = ui.as_weak();
    let request_toggle = requests.clone();
    ui.on_toggle_details_maximized(move || {
        if let Some(ui) = weak.upgrade() {
            if ui.get_is_details() {
                toggle.store(true, Ordering::Release);
                let _ = request_toggle.try_send(());
            }
        }
    });
    let timer = slint::Timer::default();
    let weak = ui.as_weak();
    let last_error = std::cell::RefCell::new(String::new());
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(250),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            for result in updates.try_iter() {
                match result {
                    Ok(Some(maximized)) => {
                        ui.set_details_maximized(maximized);
                        last_error.borrow_mut().clear();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        if *last_error.borrow() != error {
                            eprintln!("[calendar] {error}");
                            *last_error.borrow_mut() = error;
                        }
                    }
                }
            }
            if ui.window().is_visible() && !ui.window().is_minimized() {
                let _ = requests.try_send(());
            }
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_resize_handle_maps_to_the_native_direction() {
        let expected = [
            ResizeDirection::North,
            ResizeDirection::East,
            ResizeDirection::South,
            ResizeDirection::West,
            ResizeDirection::NorthWest,
            ResizeDirection::NorthEast,
            ResizeDirection::SouthWest,
            ResizeDirection::SouthEast,
        ];
        for (edge, expected) in expected.into_iter().enumerate() {
            assert_eq!(direction(edge as i32), Some(expected));
        }
        assert_eq!(direction(-1), None);
        assert_eq!(direction(8), None);
    }

    #[test]
    fn compositor_internal_mode_overrides_misleading_client_maximized_state() {
        let bytes = br#"[
            {"address":"0xabc","pid":42,"mapped":true,"fullscreen":0,"fullscreenClient":1},
            {"address":"0xdef","pid":99,"mapped":true,"fullscreen":1,"fullscreenClient":1}
        ]"#;
        let own = own_window(bytes, 42).unwrap().unwrap();
        assert_eq!(own.fullscreen, 0);
        let request = state_request(&own, 42);
        assert!(request.contains("internal=1,client=1"));
        assert!(request.contains("address:0xabc"));
        assert!(request.contains("w.pid ~= 42"));
        assert!(!request.contains("0xdef"));
        let maximized = HyprWindow {
            fullscreen: 1,
            ..own
        };
        assert!(state_request(&maximized, 42).contains("internal=0,client=0"));
    }

    #[test]
    fn compositor_requests_never_fall_back_to_other_or_ambiguous_windows() {
        assert!(own_window(
            br#"[{"address":"0xabc","pid":99,"mapped":true,"fullscreen":0}]"#,
            42
        )
        .unwrap()
        .is_none());
        for invalid in [
            "not json",
            r#"[{"address":"0xabc","pid":42,"mapped":true,"fullscreen":9}]"#,
            r#"[{"address":"injected","pid":42,"mapped":true,"fullscreen":0}]"#,
            r#"[{"address":"0xabc","pid":42,"mapped":true,"fullscreen":0},{"address":"0xdef","pid":42,"mapped":true,"fullscreen":0}]"#,
        ] {
            assert!(own_window(invalid.as_bytes(), 42).is_err());
        }
    }
}
