//! Wayland virtual-pointer injection.
//!
//! Uses the `wlr_virtual_pointer_manager_v1` protocol (available on any
//! wlroots-based compositor, including Hyprland) to synthesise pointer
//! motion, buttons, and scroll axis events.
//!
//! # Why virtual pointer, not `hyprctl`?
//!
//! `hyprctl dispatch movecursor` only moves within the focused window's
//! coordinate space and has no button-press dispatcher. Virtual pointer
//! is the correct protocol for what we need — it's the same one
//! wl-kbptr and warpd use on wlroots.
//!
//! # Threading model
//!
//! A single background thread owns the Wayland connection + event queue.
//! Actions arrive over an mpsc channel; the thread flushes them and
//! blocks on `event_queue.blocking_dispatch()` between batches. The
//! foreground [`VirtualPointer`] handle is `Send + Clone`-shaped through
//! the sender, so both the IPC thread and the Slint main thread can push
//! actions cheaply.
//!
//! # X11 fallback
//!
//! Under X11 (future DWM), swap for XTEST via the `x11rb` crate. The
//! [`Action`] enum stays identical so nothing else in the daemon needs
//! to change.

use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_output::WlOutput, wl_pointer, wl_registry, wl_seat::WlSeat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

/// Linux input-event-codes button numbers. `linux/input-event-codes.h`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// One "notch" of scroll wheel = 15.0 wl_fixed units, matching how
/// wlroots and libinput report physical mice.
const SCROLL_NOTCH: f64 = 15.0;

/// A pointer action to perform.
#[derive(Clone, Copy, Debug)]
pub enum Action {
    /// Move cursor to absolute screen coordinates.
    Move { x: i32, y: i32 },
    /// Single left-click at current position.
    LeftClick,
    /// Single right-click at current position.
    RightClick,
    /// Single middle-click at current position.
    #[allow(dead_code)]
    MiddleClick,
    /// Press and hold left button (for drag start).
    LeftPress,
    /// Release left button (for drag end).
    LeftRelease,
    /// Vertical scroll — positive = down, negative = up. Units: discrete notches.
    ScrollY(i32),
    /// Horizontal scroll — positive = right, negative = left.
    #[allow(dead_code)]
    ScrollX(i32),
}

/// Handle to a Wayland virtual pointer.
///
/// Cheap to clone (just an mpsc sender). Held by the daemon for its
/// lifetime.
#[derive(Clone)]
pub struct VirtualPointer {
    tx: mpsc::Sender<Msg>,
}

enum Msg {
    Act(Action),
    Shutdown,
}

impl VirtualPointer {
    /// Connect to the Wayland display and bind the virtual pointer manager.
    ///
    /// Spawns a background thread that owns the Wayland connection.
    /// Returns `Err` if `WAYLAND_DISPLAY` is unset or if the compositor
    /// does not advertise `zwlr_virtual_pointer_manager_v1` (unlikely on
    /// wlroots, but confirm-then-fail is friendlier than a mystery hang).
    pub fn connect() -> Result<Self> {
        std::env::var("WAYLAND_DISPLAY")
            .ok()
            .filter(|s| !s.is_empty())
            .context("WAYLAND_DISPLAY is not set (X11 backend not yet implemented)")?;

        let conn = Connection::connect_to_env()
            .context("connect to Wayland display via WAYLAND_DISPLAY")?;

        // One-shot registry roundtrip → get the global list synchronously.
        let (globals, mut event_queue) = registry_queue_init::<PointerState>(&conn)
            .context("registry_queue_init (compositor did not answer wl_registry)")?;
        let qh = event_queue.handle();

        // Bind seat + optional output first; the manager takes them by
        // reference in create_virtual_pointer_with_output.
        let seat: WlSeat = globals
            .bind(&qh, 1..=8, ())
            .context("no wl_seat global advertised")?;

        // Output is optional — with_output pins motion to a single monitor,
        // without it wlroots picks the current-cursor output.
        let output: Option<WlOutput> = globals.bind(&qh, 1..=4, ()).ok();

        let manager: ZwlrVirtualPointerManagerV1 = globals
            .bind(&qh, 1..=2, ())
            .map_err(|e| anyhow!("compositor lacks zwlr_virtual_pointer_manager_v1: {e}"))?;

        // Prefer the with_output variant when we have an output — pins
        // motion_absolute coords to that monitor. Multi-monitor support
        // is a follow-up (pass output based on cursor position).
        let pointer: ZwlrVirtualPointerV1 = if let Some(out) = output.as_ref() {
            manager.create_virtual_pointer_with_output(Some(&seat), Some(out), &qh, ())
        } else {
            manager.create_virtual_pointer(Some(&seat), &qh, ())
        };

        // First roundtrip so the compositor sees our object before we
        // start sending events (otherwise Hyprland treats early motion as
        // "unknown object" and disconnects us).
        event_queue
            .roundtrip(&mut PointerState)
            .context("initial wayland roundtrip after creating virtual pointer")?;

        let (tx, rx) = mpsc::channel::<Msg>();

        // Screen extent for motion_absolute. Real values come from
        // wl_output.geometry/mode events; until we plumb them through, use
        // a large sentinel and let Hyprland's `motion_absolute` semantics
        // treat coords as pixels-on-the-output.
        let extent = ScreenExtent {
            width: 3840,
            height: 2160,
        };

        thread::Builder::new()
            .name("hints-pointer".into())
            .spawn(move || {
                if let Err(e) = pump(event_queue, pointer, rx, extent) {
                    tracing::error!("virtual-pointer thread exited: {e:#}");
                }
            })
            .context("spawn hints-pointer thread")?;

        Ok(Self { tx })
    }

    /// Perform one action. Non-blocking — actions queue on the pointer
    /// thread. Latency budget: single-digit ms end-to-end.
    pub fn dispatch(&self, action: Action) -> Result<()> {
        self.tx
            .send(Msg::Act(action))
            .map_err(|_| anyhow!("virtual-pointer thread died"))
    }

    /// Convenience: a full left-click at (x, y) in one call.
    pub fn click_at(&self, x: i32, y: i32) -> Result<()> {
        self.dispatch(Action::Move { x, y })?;
        self.dispatch(Action::LeftClick)?;
        Ok(())
    }

    /// Convenience: a full right-click at (x, y) in one call.
    pub fn right_click_at(&self, x: i32, y: i32) -> Result<()> {
        self.dispatch(Action::Move { x, y })?;
        self.dispatch(Action::RightClick)?;
        Ok(())
    }

    /// Convenience: hover (move only) at (x, y).
    pub fn hover_at(&self, x: i32, y: i32) -> Result<()> {
        self.dispatch(Action::Move { x, y })
    }

    /// Drag from (x1, y1) to (x2, y2).
    pub fn drag(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> Result<()> {
        self.dispatch(Action::Move { x: x1, y: y1 })?;
        self.dispatch(Action::LeftPress)?;
        self.dispatch(Action::Move { x: x2, y: y2 })?;
        self.dispatch(Action::LeftRelease)?;
        Ok(())
    }
}

impl Drop for VirtualPointer {
    fn drop(&mut self) {
        // Best-effort shutdown; if the thread is already gone we ignore.
        let _ = self.tx.send(Msg::Shutdown);
    }
}

// ── Background pump ─────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct ScreenExtent {
    width: u32,
    height: u32,
}

fn pump(
    mut event_queue: EventQueue<PointerState>,
    pointer: ZwlrVirtualPointerV1,
    rx: mpsc::Receiver<Msg>,
    extent: ScreenExtent,
) -> Result<()> {
    let epoch = Instant::now();
    let mut state = PointerState;

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Shutdown => break,
            Msg::Act(action) => {
                let t = time_ms(epoch);
                dispatch_one(&pointer, action, t, extent);
                // Drain any additional actions queued while we were
                // processing so hjkl bursts don't get frame-batched into
                // one big jump.
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Shutdown => return Ok(()),
                        Msg::Act(a) => {
                            let t = time_ms(epoch);
                            dispatch_one(&pointer, a, t, extent);
                        }
                    }
                }
                event_queue.flush().context("flush wayland event queue")?;
                // Non-blocking dispatch so we pick up any protocol errors
                // (e.g. compositor kicked us for a bad message) but don't
                // sit idle waiting for events that will never come — the
                // virtual pointer protocol is client→server only.
                event_queue
                    .dispatch_pending(&mut state)
                    .context("dispatch_pending after action batch")?;
            }
        }
    }
    Ok(())
}

fn dispatch_one(
    p: &ZwlrVirtualPointerV1,
    action: Action,
    t: u32,
    extent: ScreenExtent,
) {
    match action {
        Action::Move { x, y } => {
            p.motion_absolute(
                t,
                x.max(0) as u32,
                y.max(0) as u32,
                extent.width,
                extent.height,
            );
            p.frame();
        }
        Action::LeftClick => click(p, t, BTN_LEFT),
        Action::RightClick => click(p, t, BTN_RIGHT),
        Action::MiddleClick => click(p, t, BTN_MIDDLE),
        Action::LeftPress => {
            p.button(t, BTN_LEFT, wl_pointer::ButtonState::Pressed);
            p.frame();
        }
        Action::LeftRelease => {
            p.button(t, BTN_LEFT, wl_pointer::ButtonState::Released);
            p.frame();
        }
        Action::ScrollY(notches) => scroll(p, t, wl_pointer::Axis::VerticalScroll, notches),
        Action::ScrollX(notches) => scroll(p, t, wl_pointer::Axis::HorizontalScroll, notches),
    }
}

fn click(p: &ZwlrVirtualPointerV1, t: u32, btn: u32) {
    p.button(t, btn, wl_pointer::ButtonState::Pressed);
    p.frame();
    p.button(t.wrapping_add(1), btn, wl_pointer::ButtonState::Released);
    p.frame();
}

fn scroll(p: &ZwlrVirtualPointerV1, t: u32, axis: wl_pointer::Axis, notches: i32) {
    let value = f64::from(notches) * SCROLL_NOTCH;
    p.axis(t, axis, value);
    p.frame();
}

fn time_ms(epoch: Instant) -> u32 {
    // Wraps around every ~49 days; virtual pointer accepts any monotonic
    // sequence so wrap is fine.
    epoch.elapsed().as_millis() as u32
}

// ── Dispatch impls ──────────────────────────────────────────────────────────
//
// Virtual pointer + manager are one-way client→server, but wayland-client
// still requires Dispatch impls for every object it creates. The impls are
// empty because we never expect events from these interfaces.

#[derive(Copy, Clone)]
struct PointerState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PointerState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

macro_rules! empty_dispatch {
    ($($t:ty),* $(,)?) => {
        $(
            impl Dispatch<$t, ()> for PointerState {
                fn event(
                    _: &mut Self,
                    _: &$t,
                    _: <$t as Proxy>::Event,
                    _: &(),
                    _: &Connection,
                    _: &QueueHandle<Self>,
                ) {
                }
            }
        )*
    };
}

empty_dispatch!(
    WlSeat,
    WlOutput,
    ZwlrVirtualPointerManagerV1,
    ZwlrVirtualPointerV1,
);
