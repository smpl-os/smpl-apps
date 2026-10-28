//! Native framebuffer regression fixture, not an application or desktop capture.
//! Build: cargo build -p smpl-common --example cpu_alpha_probe
//! Run: python tests/run_cpu_alpha_probe.py
//!
//! X11 reads presented pixels. Wayland checks snapshots plus the runner's actual
//! ARGB wl_shm protocol trace. Neither proves desktop compositor blur.

use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::ComponentHandle;
use smpl_common::theme::{parse_theme, ThemeRole};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::rc::Rc;
use std::time::{Duration, Instant};

slint::slint! {
    export component AlphaProbe inherits Window {
        width: 200px;
        height: 120px;
        title: "Isolated CPU alpha fixture";
        background: transparent;
        in property <color> surface;
        in property <color> surface-light;
        in property <color> foreground;
        in property <color> accent;
        in property <float> background-alpha;
        in property <bool> overlay: false;
        in property <string> retained-edit: "Unsaved fixture edit";
        if !root.overlay: Rectangle {
            background: root.surface.with-alpha(root.background-alpha);
            Text {
                x: 20px; y: 12px; width: 60px; height: 56px;
                text: "H"; color: root.foreground; font-size: 40px; font-weight: 700;
            }
            Rectangle {
                x: 104.25px; y: 20.25px; width: 36px; height: 36px;
                border-radius: 18px; background: root.accent;
                drop-shadow-blur: 8px; drop-shadow-color: #000000aa;
            }
            Rectangle {
                x: 20px; y: 80px; width: 100px; height: 24px;
                background: root.surface-light.with-alpha(0.12);
            }
        }
        Rectangle {
            x: 155px; y: 80px; width: 30px; height: 24px;
            background: root.surface.with-alpha(root.background-alpha);
            Text {
                text: "H"; color: root.foreground; font-size: 18px; font-weight: 700;
                horizontal-alignment: center; vertical-alignment: center;
            }
        }
    }
}

#[link(name = "X11")]
unsafe extern "C" {
    fn XOpenDisplay(name: *const c_char) -> *mut c_void;
    fn XCloseDisplay(display: *mut c_void) -> c_int;
    fn XDefaultRootWindow(display: *mut c_void) -> c_ulong;
    fn XSetInputFocus(display: *mut c_void, window: c_ulong, revert: c_int, time: c_ulong)
        -> c_int;
    fn XSync(display: *mut c_void, discard: c_int) -> c_int;
    fn XGetImage(
        display: *mut c_void,
        window: c_ulong,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        planes: c_ulong,
        format: c_int,
    ) -> *mut c_void;
    fn XGetPixel(image: *mut c_void, x: c_int, y: c_int) -> c_ulong;
    fn XDestroyImage(image: *mut c_void) -> c_int;
    fn XGetGeometry(
        display: *mut c_void,
        window: c_ulong,
        root: *mut c_ulong,
        x: *mut c_int,
        y: *mut c_int,
        width: *mut c_uint,
        height: *mut c_uint,
        border: *mut c_uint,
        depth: *mut c_uint,
    ) -> c_int;
}

fn window_id(ui: &AlphaProbe) -> c_ulong {
    ui.window()
        .with_winit_window(|window| match window.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(handle) => handle.window,
            RawWindowHandle::Xcb(handle) => handle.window.get() as c_ulong,
            _ => panic!("This fixture requires the runner's private X11 display"),
        })
        .unwrap()
}

fn focus(ui: &AlphaProbe, focused: bool) {
    let window_id = window_id(ui);
    // Only our authenticated private display is allowed by main's environment guard.
    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        assert!(!display.is_null(), "could not connect to private Xvfb");
        let target = if focused {
            window_id
        } else {
            XDefaultRootWindow(display)
        };
        XSetInputFocus(display, target, 1, 0);
        XSync(display, 0);
        XCloseDisplay(display);
    }
}

fn framebuffer(ui: &AlphaProbe) -> Vec<u8> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let image = ui.window().take_snapshot().unwrap();
        return (0..120)
            .flat_map(|y| {
                let image = &image;
                (0..200).flat_map(move |x| {
                    let p = image.as_slice()[y * image.width() as usize + x];
                    [p.r, p.g, p.b, p.a]
                })
            })
            .collect();
    }
    // Read pixels actually presented to X11, not a separately rendered snapshot.
    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        assert!(!display.is_null());
        let (mut root, mut x, mut y, mut width, mut height, mut border, mut depth) =
            (0, 0, 0, 0, 0, 0, 0);
        assert_ne!(
            XGetGeometry(
                display,
                window_id(ui),
                &mut root,
                &mut x,
                &mut y,
                &mut width,
                &mut height,
                &mut border,
                &mut depth,
            ),
            0
        );
        assert_eq!(depth, 32, "native window must have an ARGB visual");
        assert!(width >= 200 && height >= 120);
        let image = XGetImage(display, window_id(ui), 0, 0, 200, 120, !0, 2);
        assert!(!image.is_null());
        let mut pixels = Vec::with_capacity(200 * 120 * 4);
        for y in 0..120 {
            for x in 0..200 {
                let argb = XGetPixel(image, x, y);
                pixels.extend_from_slice(&[
                    (argb >> 16) as u8,
                    (argb >> 8) as u8,
                    argb as u8,
                    (argb >> 24) as u8,
                ]);
            }
        }
        XDestroyImage(image);
        XCloseDisplay(display);
        pixels
    }
}

fn alpha(pixels: &[u8], x: usize, y: usize) -> u8 {
    pixels[(y * 200 + x) * 4 + 3]
}

fn region(pixels: &[u8], x: std::ops::Range<usize>, y: std::ops::Range<usize>) -> Vec<u8> {
    y.flat_map(|y| x.clone().map(move |x| alpha(pixels, x, y)))
        .collect()
}

fn check(pixels: &[u8], stage: usize, focused: bool, wayland: bool) {
    let expected = if stage == 1 { 255 } else { 140 };
    let root = alpha(pixels, 5, 5);
    if stage == 5 {
        assert_eq!(root, 0, "overlay root must stay clear");
        assert_eq!(alpha(pixels, 158, 83), 140, "badge background");
        assert!(
            region(pixels, 160..181, 83..101).contains(&255),
            "badge glyph interior"
        );
        println!("stage=overlay root_alpha=0 badge_alpha=140 glyph_alpha=255");
        return;
    }
    assert_eq!(root, expected, "background alpha, stage {stage}");
    let card = alpha(pixels, 30, 90);
    // Source-over alpha: .12 + .55 * (1 - .12), rounded to the 8-bit framebuffer.
    assert_eq!(card, if stage == 1 { 255 } else { 154 }, "card tint alpha");
    assert_eq!(alpha(pixels, 122, 38), 255, "solid icon interior");
    if stage != 1 {
        assert!(
            alpha(pixels, 100, 38) > expected,
            "CPU must retain the icon's shadow"
        );
    }
    let glyph = region(pixels, 20..80, 12..68);
    assert!(glyph.contains(&255), "glyph must have opaque interior");
    let icon = region(pixels, 102..142, 18..58);
    let aa = |values: &[u8]| values.iter().filter(|&&a| a > expected && a < 255).count();
    if expected != 255 {
        assert!(aa(&glyph) > 0, "glyph AA edge coverage");
        assert!(aa(&icon) > 0, "icon AA edge coverage");
    }
    if wayland {
        // Headless Weston has no real keyboard seat; focus is checked on X11.
    } else if stage == 3 {
        assert!(!focused, "native focus must have moved to private root");
    } else {
        assert!(focused, "native fixture window must have focus");
    }
    println!(
        "stage={stage} focused={focused} background_alpha={root} card_alpha={card} glyph_alpha=255 \
         icon_alpha=255 glyph_aa_pixels={} icon_aa_pixels={}",
        aa(&glyph),
        aa(&icon)
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let display = std::env::var("DISPLAY").unwrap_or_default();
    let wayland = std::env::var("WAYLAND_DISPLAY").as_deref() == Ok("smpl-cpu-probe");
    let private_wayland = wayland
        && std::env::var_os("DISPLAY").is_none()
        && std::env::var("XDG_RUNTIME_DIR")
            .unwrap()
            .contains("smpl-cpu-wayland-");
    assert!(
        std::env::var("SMPL_ALPHA_PROBE_PRIVATE_DISPLAY").as_deref() == Ok("1")
            && (private_wayland
                || (display.starts_with("127.0.0.1:")
                    && std::env::var_os("WAYLAND_DISPLAY").is_none())),
        "Run only through tests/run_cpu_alpha_probe.py, never on a user display"
    );
    smpl_common::init("smpl-alpha-probe", 200.0, 120.0)?;
    let ui = AlphaProbe::new()?;
    let apply = |ui: &AlphaProbe, opacity: f32| {
        let palette = parse_theme(
            &format!(
                "$theme-bg: #1e1e2e;\n$theme-fg: #cdd6f4;\n\
                 $theme-accent: #89b4fa;\n$theme-app-background-opacity: {opacity};"
            ),
            ThemeRole::Application,
        )
        .unwrap();
        ui.set_surface(palette.bg);
        ui.set_surface_light(palette.bg_light);
        ui.set_foreground(palette.fg);
        ui.set_accent(palette.accent);
        ui.set_background_alpha(palette.opacity);
    };
    apply(&ui, 0.55);
    let stage = Rc::new(Cell::new(0usize));
    ui.show()?;
    let first_frame = slint::Timer::default();
    let weak = ui.as_weak();
    let mut reported = false;
    first_frame.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(2),
        move || {
            if reported {
                return;
            }
            let ui = weak.unwrap();
            if ui.window().with_winit_window(|_| ()).is_some()
                && alpha(&framebuffer(&ui), 5, 5) == 140
            {
                println!(
                    "first_present_ms={:.3}",
                    started.elapsed().as_secs_f64() * 1000.
                );
                reported = true;
                if std::env::args().any(|arg| arg == "--startup-only") {
                    slint::quit_event_loop().unwrap();
                }
            }
        },
    );
    let timer = slint::Timer::default();
    let mut prepare = true;
    let mut ticks = 0;
    let mut baseline = None;
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        ticks += 1;
        assert!(ticks < 50, "native renderer/focus probe timed out");
        let ui = weak.unwrap();
        if prepare {
            apply(&ui, if stage.get() == 1 { 1.0 } else { 0.55 });
            ui.set_overlay(stage.get() == 5);
            if !wayland {
                focus(&ui, stage.get() != 3);
            }
            if stage.get() == 6 {
                ui.window().set_size(slint::LogicalSize::new(240., 160.));
            } else if stage.get() == 7 {
                ui.hide().unwrap();
                ui.window().set_size(slint::LogicalSize::new(200., 120.));
                ui.show().unwrap();
                ui.window().request_redraw();
            } else if stage.get() == 8 {
                let palette = parse_theme(
                    "$theme-bg: #f0f0f0;\n$theme-fg: #202020;\n$theme-accent: #2860c0;",
                    ThemeRole::Application,
                ).unwrap();
                ui.set_surface(palette.bg);
                ui.set_surface_light(palette.bg_light);
                ui.set_foreground(palette.fg);
                ui.set_accent(palette.accent);
            }
            prepare = false;
        } else {
            let pixels = framebuffer(&ui);
            let focused = ui.window().with_winit_window(|window| window.has_focus()).unwrap();
            check(&pixels, if stage.get() >= 6 { 0 } else { stage.get() }, focused, wayland);
            if (2..=4).contains(&stage.get()) || (6..=7).contains(&stage.get()) {
                assert_eq!(baseline.as_ref(), Some(&pixels), "opacity/focus/resize/reopen must preserve native RGBA pixels");
            }
            if stage.get() == 0 {
                baseline = Some(pixels);
            }
            assert_eq!(ui.get_retained_edit(), "Unsaved fixture edit");
            if stage.get() == 8 {
                let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
                for driver in ["libEGL", "libGLX", "libnvidia", "libvulkan", "/dri/"] {
                    assert!(!maps.contains(driver), "CPU GUI loaded a GPU driver: {driver}");
                }
                println!("PASS: native CPU ARGB, shadows, opaque text, focus, resize/reopen; no GPU driver");
                slint::quit_event_loop().unwrap();
                return;
            }
            stage.set(stage.get() + 1);
            prepare = true;
        }
    });
    slint::run_event_loop_until_quit()?;
    Ok(())
}
