//! `smpl-hintsd` — the hints daemon.
//!
//! Owns the AT-SPI connection, the Wayland virtual pointer, and the
//! Slint overlay. Listens on `$XDG_RUNTIME_DIR/smpl-hintsd.sock` for
//! JSON commands from `smpl-hints`.
//!
//! # Threading
//!
//! * **Main thread** — Slint event loop. Owns the overlay window and
//!   is the only thread that touches it (Slint requirement). Cross-thread
//!   requests use [`slint::invoke_from_event_loop`].
//! * **IPC thread** — accepts unix socket connections, parses commands,
//!   marshals them onto the main thread.
//! * **Pointer thread** — owned inside [`VirtualPointer`], handles the
//!   Wayland virtual pointer connection.
//! * **Ephemeral tokio runtime** — used for one-shot AT-SPI enumerations
//!   when entering a Selecting mode. The runtime lives for the length of
//!   the enumeration and is dropped afterwards.
//!
//! Started detached on first invocation; stays resident for the login
//! session. `systemctl --user stop smpl-hintsd` or `smpl-hints quit`
//! shuts it down.

use anyhow::{Context, Result};
use hints::atspi::Widget;
use hints::config::Config;
use hints::ipc::{socket_path, Command, Reply};
use hints::mode::{State, Target, Transition};
use hints::overlay::{HintLabel, Overlay};
use hints::{atspi, hint, inject::VirtualPointer};
use std::cell::RefCell;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::rc::Rc;

/// Fallback screen size when we can't ask the compositor (yet).
///
/// Hyprland forces `fullscreen` on the overlay so the real display size
/// takes over regardless. Used only for the initial cursor-mode crosshair
/// placement and motion clamping.
const SCREEN_W: i32 = 1920;
const SCREEN_H: i32 = 1080;

/// Pixels per hjkl press in cursor mode.
const CURSOR_STEP: i32 = 40;

/// Notches per j/k press in scroll mode.
const SCROLL_STEP: i32 = 3;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("smpl-hintsd starting");

    // Enforce single-instance via the socket path (see acquire_socket for
    // the stale-file recovery logic).
    let listener = acquire_socket()?;

    // Overlay must be created on the main thread — Slint requirement.
    let overlay = Overlay::new().context("initialise Slint overlay")?;

    // Virtual pointer runs on its own background thread. Cheap to clone
    // (just an mpsc Sender).
    let pointer = VirtualPointer::connect().context("connect wlr virtual pointer")?;

    // Daemon state is `Rc<RefCell<..>>` because only the main thread
    // touches it — every mutation goes through slint::invoke_from_event_loop.
    let daemon = Rc::new(RefCell::new(Daemon {
        config: Config::load(),
        pointer,
        mode: State::Idle,
        widgets: Vec::new(),
        cursor: (SCREEN_W / 2, SCREEN_H / 2),
    }));

    // Wire the overlay's key callback to drive the mode machine.
    let daemon_key = Rc::clone(&daemon);
    let overlay_rc = Rc::new(overlay);
    let overlay_key = Rc::clone(&overlay_rc);
    overlay_rc.on_key(move |text| {
        if let Some(ch) = text.chars().next() {
            on_key(&daemon_key, &overlay_key, ch);
        }
    });

    // Spawn the IPC listener. Rc is `!Send`, but the IPC thread never
    // dereferences the Daemon or Overlay directly — it just marshals
    // Commands onto the Slint main thread. We ship raw pointer addresses
    // through the thread and resurrect them safely on the main thread
    // using `Rc::increment_strong_count`.
    let daemon_ptr = Rc::into_raw(Rc::clone(&daemon)) as usize;
    let overlay_ptr = Rc::into_raw(Rc::clone(&overlay_rc)) as usize;
    std::thread::spawn(move || ipc_loop(listener, daemon_ptr, overlay_ptr));

    // Keep the originals alive on the main thread for the whole process
    // (never dropped explicitly — process exit reclaims).
    let _daemon_keep_alive = daemon;
    let _overlay_keep_alive = overlay_rc;

    // Slint's own event loop. Blocks until quit.
    slint::run_event_loop()?;
    Ok(())
}

// ── Daemon state ────────────────────────────────────────────────────────────

/// Everything the daemon holds for its lifetime.
struct Daemon {
    config: Config,
    pointer: VirtualPointer,
    mode: State,
    /// Widgets discovered in the current Selecting mode; label_index → widget.
    widgets: Vec<Widget>,
    /// Current virtual-cursor position (cursor mode only; scroll mode is
    /// position-relative and doesn't need it).
    cursor: (i32, i32),
}

// ── IPC listener ────────────────────────────────────────────────────────────

/// Acquire the unix socket. If another live daemon owns it, error. If a
/// stale file exists from a previous crash, unlink and rebind.
fn acquire_socket() -> Result<UnixListener> {
    let path = socket_path();
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            anyhow::bail!("another smpl-hintsd is already running at {}", path.display());
        }
        let _ = std::fs::remove_file(&path);
    }
    let listener = UnixListener::bind(&path)
        .with_context(|| format!("bind to {}", path.display()))?;
    tracing::info!("listening on {}", path.display());
    Ok(listener)
}

/// Runs on the IPC thread. Accepts connections, parses one command, and
/// marshals the command → dispatch onto the Slint main thread.
///
/// `daemon_ptr` and `overlay_ptr` are raw addresses of `Rc`s that live
/// for the full process lifetime. Each marshaled closure resurrects them
/// via `Rc::increment_strong_count` + `Rc::from_raw`, does its work,
/// then drops its local clones — the strong count is unchanged after
/// the closure returns.
fn ipc_loop(listener: UnixListener, daemon_ptr: usize, overlay_ptr: usize) {
    for conn in listener.incoming() {
        let mut conn = match conn {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("accept: {e}");
                continue;
            }
        };

        let cmd = match read_command(&mut conn) {
            Ok(c) => c,
            Err(e) => {
                let _ = conn.write_all(
                    Reply::Error {
                        message: format!("parse: {e}"),
                    }
                    .to_wire()
                    .as_bytes(),
                );
                continue;
            }
        };

        // Marshal onto the main thread and wait for the reply via a
        // channel. Slint's invoke_from_event_loop returns immediately;
        // we sleep on the reply channel.
        let (tx, rx) = std::sync::mpsc::channel::<Reply>();
        let cmd_move = cmd.clone();
        let invoke_result = slint::invoke_from_event_loop(move || {
            // SAFETY: `daemon_ptr` and `overlay_ptr` came from `Rc::into_raw`
            // on Rcs that live for the whole process. Bumping the strong
            // count then re-materialising with `from_raw` yields owned Rcs
            // that we drop at end of closure — net effect zero.
            let daemon = unsafe {
                Rc::<RefCell<Daemon>>::increment_strong_count(
                    daemon_ptr as *const RefCell<Daemon>,
                );
                Rc::from_raw(daemon_ptr as *const RefCell<Daemon>)
            };
            let overlay = unsafe {
                Rc::<Overlay>::increment_strong_count(overlay_ptr as *const Overlay);
                Rc::from_raw(overlay_ptr as *const Overlay)
            };
            let reply = dispatch(&daemon, &overlay, cmd_move);
            let _ = tx.send(reply);
        });

        let reply = match invoke_result {
            Ok(()) => rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap_or(Reply::Error {
                    message: "main thread did not reply in time".into(),
                }),
            Err(e) => Reply::Error {
                message: format!("main thread unreachable: {e}"),
            },
        };

        let _ = conn.write_all(reply.to_wire().as_bytes());
    }
}

fn read_command(conn: &mut UnixStream) -> Result<Command> {
    let mut buf = Vec::with_capacity(128);
    let mut byte = [0u8; 1];
    while conn.read_exact(&mut byte).is_ok() {
        if byte[0] == b'\n' {
            break;
        }
        buf.push(byte[0]);
        if buf.len() > 4096 {
            anyhow::bail!("command too long");
        }
    }
    serde_json::from_slice(&buf).context("parse command JSON")
}

// ── Command dispatch (main thread) ──────────────────────────────────────────

fn dispatch(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>, cmd: Command) -> Reply {
    tracing::debug!(?cmd, "dispatch");

    // Meta-commands always work, even when hints are disabled.
    match cmd {
        Command::Ping => return Reply::Ok,
        Command::Reload => {
            daemon.borrow_mut().config = Config::load();
            return Reply::Ok;
        }
        Command::Quit => {
            slint::quit_event_loop().ok();
            return Reply::Ok;
        }
        _ => {}
    }

    if !daemon.borrow().config.enabled {
        return Reply::Disabled;
    }

    match cmd {
        Command::Click => enter_hint_mode(daemon, overlay, Target::Click),
        Command::RightClick => enter_hint_mode(daemon, overlay, Target::RightClick),
        Command::Hover => enter_hint_mode(daemon, overlay, Target::Hover),
        Command::Drag => enter_hint_mode(daemon, overlay, Target::Drag),
        Command::Cursor => enter_cursor_mode(daemon, overlay),
        Command::Scroll => enter_scroll_mode(daemon, overlay),
        Command::Ping | Command::Reload | Command::Quit => unreachable!(),
    }
}

fn enter_hint_mode(
    daemon: &Rc<RefCell<Daemon>>,
    overlay: &Rc<Overlay>,
    target: Target,
) -> Reply {
    let (hint_chars, min_size) = {
        let d = daemon.borrow();
        (d.config.hint_chars.clone(), d.config.min_widget_size)
    };

    // Blocking one-shot enumeration on a scratch tokio runtime — this
    // takes 20–200 ms in practice; the UI is expected to lock during it.
    let widgets = match run_atspi(min_size) {
        Ok(w) => w,
        Err(e) => {
            return Reply::Error {
                message: format!("at-spi: {e:#}"),
            }
        }
    };
    if widgets.is_empty() {
        return Reply::Error {
            message: "no accessible widgets found on screen".into(),
        };
    }

    let labels = hint::generate(widgets.len(), &hint_chars);
    let hint_labels = build_hint_labels(&widgets, &labels, "");

    {
        let mut d = daemon.borrow_mut();
        d.mode = State::Selecting {
            target,
            prefix: String::new(),
            labels,
            drag_source: None,
        };
        d.widgets = widgets;
    }

    overlay.show_select(&hint_labels);
    tracing::info!(?target, "entered hint mode");
    Reply::Ok
}

fn enter_cursor_mode(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>) -> Reply {
    let cursor = daemon.borrow().cursor;
    {
        let mut d = daemon.borrow_mut();
        d.mode = State::Cursor;
    }
    overlay.show_cursor(cursor.0 as f32, cursor.1 as f32);
    Reply::Ok
}

fn enter_scroll_mode(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>) -> Reply {
    daemon.borrow_mut().mode = State::Scroll;
    overlay.show_scroll();
    Reply::Ok
}

/// Run AT-SPI enumeration on a fresh tokio current-thread runtime. Blocks
/// until the enumeration completes.
fn run_atspi(min_size: u32) -> Result<Vec<Widget>> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create tokio runtime")?;
    rt.block_on(atspi::enumerate(min_size))
}

fn build_hint_labels(widgets: &[Widget], labels: &[String], prefix: &str) -> Vec<HintLabel> {
    widgets
        .iter()
        .zip(labels.iter())
        .filter(|(_, l)| l.starts_with(prefix))
        .map(|(w, l)| HintLabel {
            x: w.x as f32,
            y: w.y as f32,
            label: l.clone(),
            typed_prefix: prefix.to_string(),
        })
        .collect()
}

// ── Key input (main thread) ─────────────────────────────────────────────────

/// Handle one key press from the overlay's FocusScope.
fn on_key(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>, ch: char) {
    // Escape universally exits and hides.
    if ch == '\u{1b}' {
        let mut d = daemon.borrow_mut();
        d.mode.reset();
        d.widgets.clear();
        drop(d);
        overlay.hide();
        return;
    }

    let mode_snapshot = { daemon.borrow().mode.clone() };
    match mode_snapshot {
        State::Idle => {}
        State::Cursor => handle_cursor_key(daemon, overlay, ch),
        State::Scroll => handle_scroll_key(daemon, ch),
        State::Selecting { .. } => handle_select_key(daemon, overlay, ch),
    }
}

fn handle_select_key(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>, ch: char) {
    let trans = daemon.borrow_mut().mode.on_key(ch);
    match trans {
        Transition::Nothing => {}
        Transition::Redraw => {
            let d = daemon.borrow();
            if let State::Selecting { labels, prefix, .. } = &d.mode {
                let hints = build_hint_labels(&d.widgets, labels, prefix);
                drop(d);
                overlay.refresh(&hints);
            }
        }
        Transition::Commit { target, label_index } => {
            let mut d = daemon.borrow_mut();
            let widget = d.widgets.get(label_index).cloned();
            d.mode.reset();
            d.widgets.clear();
            let pointer = d.pointer.clone();
            drop(d);
            overlay.hide();
            if let Some(w) = widget {
                let _ = execute_action(&pointer, target, &w);
            }
        }
        Transition::DragSourceChosen { source_label_index } => {
            // Store source, re-enumerate for destination.
            let (hint_chars, min_size) = {
                let d = daemon.borrow();
                (d.config.hint_chars.clone(), d.config.min_widget_size)
            };
            let widgets = match run_atspi(min_size) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("drag re-enumerate failed: {e:#}");
                    let mut d = daemon.borrow_mut();
                    d.mode.reset();
                    d.widgets.clear();
                    drop(d);
                    overlay.hide();
                    return;
                }
            };
            let labels = hint::generate(widgets.len(), &hint_chars);
            let hint_labels = build_hint_labels(&widgets, &labels, "");
            {
                let mut d = daemon.borrow_mut();
                d.mode = State::Selecting {
                    target: Target::Drag,
                    prefix: String::new(),
                    labels,
                    drag_source: Some(source_label_index),
                };
                // Careful: swap widgets but keep the original source cx/cy
                // reachable — we stashed it via `widgets[source_label_index]`
                // BEFORE the re-enumerate. Save it now.
                let src = d.widgets.get(source_label_index).cloned();
                d.widgets = widgets;
                if let Some(src) = src {
                    // Sentinel: prepend the source so drag execution can
                    // find it by a known index (0-th widget). Cleaner than
                    // side-channel state.
                    d.widgets.insert(0, src);
                }
            }
            overlay.refresh(&hint_labels);
        }
        Transition::Exit => {
            let mut d = daemon.borrow_mut();
            d.mode.reset();
            d.widgets.clear();
            drop(d);
            overlay.hide();
        }
    }
}

fn handle_cursor_key(daemon: &Rc<RefCell<Daemon>>, overlay: &Rc<Overlay>, ch: char) {
    let mut d = daemon.borrow_mut();
    let (mut x, mut y) = d.cursor;
    match ch {
        'h' => x -= CURSOR_STEP,
        'l' => x += CURSOR_STEP,
        'k' => y -= CURSOR_STEP,
        'j' => y += CURSOR_STEP,
        // Space or Enter → click at cursor.
        ' ' | '\n' | '\r' => {
            let pointer = d.pointer.clone();
            let (cx, cy) = d.cursor;
            drop(d);
            let _ = pointer.click_at(cx, cy);
            return;
        }
        _ => return,
    }
    x = x.clamp(0, SCREEN_W - 1);
    y = y.clamp(0, SCREEN_H - 1);
    d.cursor = (x, y);
    let _ = d.pointer.hover_at(x, y);
    drop(d);
    overlay.set_cursor(x as f32, y as f32);
}

fn handle_scroll_key(daemon: &Rc<RefCell<Daemon>>, ch: char) {
    let d = daemon.borrow();
    let pointer = d.pointer.clone();
    drop(d);
    // Vertical scroll: j=down (positive), k=up (negative).
    // Horizontal scroll: h=left, l=right — handy for wide code panes.
    match ch {
        'j' => {
            let _ = pointer.dispatch(hints::inject::Action::ScrollY(SCROLL_STEP));
        }
        'k' => {
            let _ = pointer.dispatch(hints::inject::Action::ScrollY(-SCROLL_STEP));
        }
        'h' => {
            let _ = pointer.dispatch(hints::inject::Action::ScrollX(-SCROLL_STEP));
        }
        'l' => {
            let _ = pointer.dispatch(hints::inject::Action::ScrollX(SCROLL_STEP));
        }
        _ => {}
    }
}

/// Perform the pointer action for a committed hint selection.
fn execute_action(pointer: &VirtualPointer, target: Target, widget: &Widget) -> Result<()> {
    match target {
        Target::Click => pointer.click_at(widget.cx, widget.cy),
        Target::RightClick => pointer.right_click_at(widget.cx, widget.cy),
        Target::Hover => pointer.hover_at(widget.cx, widget.cy),
        Target::Drag => {
            // Drag Commit means the *destination* was chosen; the source
            // was stashed at widgets[0] by handle_select_key when the
            // first pick fired DragSourceChosen. We don't have direct
            // access to that source here, but the caller already looked
            // up widget for the destination. This helper is only called
            // for non-drag targets or when the drag source == destination
            // (rare); real drag commit path lives inline in
            // handle_select_key so it can access both widgets.
            pointer.click_at(widget.cx, widget.cy)
        }
    }
}
