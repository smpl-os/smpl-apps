//! Native framebuffer regression fixture, not an application or desktop capture.
//! Build: cargo build -p smpl-common --example femtovg_alpha_probe
//! Run: python tests/run_femtovg_alpha_probe.py
//!
//! This proves FemtoVG/OpenGL framebuffer alpha on isolated X11, not Wayland
//! compositor blur, compositor opacity rules, or every consuming application's UI.

use i_slint_backend_winit::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, GraphicsAPI, RenderingState};
use smpl_common::theme::{parse_theme, ThemeRole};
use std::cell::Cell;
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void, CStr};
use std::rc::Rc;
use std::time::Duration;

slint::slint! {
    export component AlphaProbe inherits Window {
        width: 200px;
        height: 120px;
        title: "Isolated FemtoVG alpha fixture";
        background: transparent;
        in property <color> surface;
        in property <color> surface-light;
        in property <color> foreground;
        in property <color> accent;
        in property <float> background-alpha;
        in property <bool> overlay: false;
        if !root.overlay: Rectangle {
            background: root.surface.with-alpha(root.background-alpha);
            Text {
                x: 20px; y: 12px; width: 60px; height: 56px;
                text: "H"; color: root.foreground; font-size: 40px; font-weight: 700;
            }
            Rectangle {
                x: 104.25px; y: 20.25px; width: 36px; height: 36px;
                border-radius: 18px; background: root.accent;
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
}

fn focus(ui: &AlphaProbe, focused: bool) {
    let window_id = ui
        .window()
        .with_winit_window(|window| match window.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(handle) => handle.window,
            RawWindowHandle::Xcb(handle) => handle.window.get() as c_ulong,
            _ => panic!("This fixture requires the runner's private X11 display"),
        })
        .unwrap();
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

fn framebuffer(api: &GraphicsAPI, report_driver: bool) -> Vec<u8> {
    let GraphicsAPI::NativeOpenGL { get_proc_address } = api else {
        panic!("Expected native FemtoVG OpenGL, received {api:?}");
    };
    let address = |name: &CStr| {
        let pointer = get_proc_address(name);
        assert!(!pointer.is_null(), "missing GL function {name:?}");
        pointer
    };
    // The notifier runs with Slint's GL context current, after drawing and before
    // swap. Read the actual default framebuffer; do not create an RGBA substitute.
    unsafe {
        if report_driver {
            let get_string: unsafe extern "system" fn(c_uint) -> *const c_char =
                std::mem::transmute(address(c"glGetString"));
            let renderer = get_string(0x1F01);
            let version = get_string(0x1F02);
            assert!(!renderer.is_null() && !version.is_null());
            println!(
                "native_gl_renderer={} version={}",
                CStr::from_ptr(renderer).to_string_lossy(),
                CStr::from_ptr(version).to_string_lossy()
            );
        }
        let get_integer: unsafe extern "system" fn(c_uint, *mut c_int) =
            std::mem::transmute(address(c"glGetIntegerv"));
        let get_error: unsafe extern "system" fn() -> c_uint =
            std::mem::transmute(address(c"glGetError"));
        let read_pixels: unsafe extern "system" fn(
            c_int,
            c_int,
            c_int,
            c_int,
            c_uint,
            c_uint,
            *mut c_void,
        ) = std::mem::transmute(address(c"glReadPixels"));
        let mut viewport = [0; 4];
        get_integer(0x0BA2, viewport.as_mut_ptr());
        assert_eq!(viewport, [0, 0, 200, 120], "unexpected viewport/scale");
        let mut binding = -1;
        get_integer(0x8CA6, &mut binding);
        assert_eq!(binding, 0, "must read the native default framebuffer");
        let mut pixels = vec![0; 200 * 120 * 4];
        read_pixels(0, 0, 200, 120, 0x1908, 0x1401, pixels.as_mut_ptr().cast());
        assert_eq!(get_error(), 0, "GL readback failed");
        pixels
    }
}

fn alpha(pixels: &[u8], x: usize, y: usize) -> u8 {
    pixels[((119 - y) * 200 + x) * 4 + 3]
}

fn region(pixels: &[u8], x: std::ops::Range<usize>, y: std::ops::Range<usize>) -> Vec<u8> {
    y.flat_map(|y| x.clone().map(move |x| alpha(pixels, x, y)))
        .collect()
}

fn check(pixels: &[u8], stage: usize, focused: bool) {
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
    let glyph = region(pixels, 20..80, 12..68);
    assert!(glyph.contains(&255), "glyph must have opaque interior");
    let icon = region(pixels, 102..142, 18..58);
    let aa = |values: &[u8]| values.iter().filter(|&&a| a > expected && a < 255).count();
    if expected != 255 {
        assert!(aa(&glyph) > 0, "glyph AA edge coverage");
        assert!(aa(&icon) > 0, "icon AA edge coverage");
    }
    if stage == 3 {
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
    let display = std::env::var("DISPLAY").unwrap_or_default();
    assert!(
        std::env::var("SMPL_ALPHA_PROBE_PRIVATE_DISPLAY").as_deref() == Ok("1")
            && display.starts_with("127.0.0.1:")
            && std::env::var_os("WAYLAND_DISPLAY").is_none(),
        "Run only through tests/run_femtovg_alpha_probe.py, never on a user display"
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
    let armed = Rc::new(Cell::new(false));
    let captured = Rc::new(Cell::new(false));
    let weak = ui.as_weak();
    ui.window().set_rendering_notifier({
        let stage = stage.clone();
        let armed = armed.clone();
        let captured = captured.clone();
        let mut baseline = None;
        move |state, api| {
            if !matches!(state, RenderingState::AfterRendering) || !armed.replace(false) {
                return;
            }
            let ui = weak.unwrap();
            let focused = ui
                .window()
                .with_winit_window(|window| window.has_focus())
                .unwrap();
            let pixels = framebuffer(api, stage.get() == 0);
            check(&pixels, stage.get(), focused);
            if (2..=4).contains(&stage.get()) {
                assert!(
                    baseline.as_ref() == Some(&pixels),
                    "opacity roundtrip and native focus changes must preserve every RGBA pixel"
                );
            }
            if stage.get() == 0 {
                baseline = Some(pixels);
            }
            captured.set(true);
        }
    })?;
    ui.show()?;
    let timer = slint::Timer::default();
    let mut prepare = true;
    let mut ticks = 0;
    let weak = ui.as_weak();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), move || {
        ticks += 1;
        assert!(ticks < 50, "native renderer/focus probe timed out");
        let ui = weak.unwrap();
        if captured.replace(false) {
            if stage.get() == 5 {
                println!("PASS: native FemtoVG framebuffer alpha; Xvfb, not Wayland compositor proof");
                slint::quit_event_loop().unwrap();
                return;
            }
            stage.set(stage.get() + 1);
            prepare = true;
        }
        if prepare {
            apply(&ui, if stage.get() == 1 { 1.0 } else { 0.55 });
            ui.set_overlay(stage.get() == 5);
            focus(&ui, stage.get() != 3);
            prepare = false;
        } else {
            armed.set(true);
            ui.window().request_redraw();
        }
    });
    slint::run_event_loop()?;
    Ok(())
}
