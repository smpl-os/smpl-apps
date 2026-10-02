//! Private native event-editor fixture; never opens production data.
use chrono::{Datelike, Timelike};
use slint::ComponentHandle;
use std::{cell::Cell, rc::Rc, time::Duration};

#[path = "../src/editor.rs"]
mod editor;
#[allow(dead_code)]
#[path = "../src/event_time.rs"]
mod event_time;
#[allow(dead_code)]
#[path = "../src/local_provider.rs"]
mod local_provider;
#[allow(dead_code)]
#[path = "../src/models.rs"]
mod models;
#[allow(dead_code)]
#[path = "../src/provider.rs"]
mod provider;

slint::include_modules!();

fn click(ui: &MainWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(60), move || {
        weak.unwrap()
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
    });
}

fn snapshot(ui: &MainWindow, name: &str, host: bool) {
    if host {
        return;
    }
    let image = ui.window().take_snapshot().unwrap();
    let mut bytes = format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
    for pixel in image.as_slice() {
        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    let path = std::path::PathBuf::from(std::env::var("SMPL_CALENDAR_PROBE_OUTPUT").unwrap());
    std::fs::write(path.join(format!("editor-{name}.ppm")), bytes).unwrap();
}

fn key(ui: &MainWindow, key: slint::platform::Key) {
    let text: slint::SharedString = key.into();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
}

fn type_text(ui: &MainWindow, text: &str) {
    for (index, character) in text.chars().enumerate() {
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(index as u64 * 50), move || {
            let ui = weak.unwrap();
            let text: slint::SharedString = character.to_string().into();
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: text.clone() });
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::KeyReleased { text });
        });
    }
}

fn typeahead_probe(ui: &MainWindow, host: bool) -> Result<(), slint::PlatformError> {
    ui.set_form_start_h(9);
    ui.invoke_form_interval_changed(false);
    let stage = Rc::new(Cell::new(0));
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let ui = weak.unwrap();
        match stage.get() {
            0 => click(&ui, 468.0, 318.0),
            1 => { key(&ui, slint::platform::Key::Escape); type_text(&ui, "21"); }
            2 => {
                assert_eq!(ui.get_form_start_h(), 9, "typing must not commit");
                snapshot(&ui, "jump-21", host); type_text(&ui, "30");
            }
            3 => {
                snapshot(&ui, "jump-2130", host);
                assert_eq!((ui.get_form_start_h(), ui.get_form_end_h()), (9, 10));
                key(&ui, slint::platform::Key::Return);
            }
            4 => {
                assert_eq!((ui.get_form_start_h(), ui.get_form_start_m()), (21, 30));
                assert_eq!((ui.get_form_end_h(), ui.get_form_end_m()), (21, 50));
                click(&ui, 468.0, 354.0);
            }
            5 => type_text(&ui, "21:55"),
            6 => { assert_eq!(ui.get_form_end_m(), 50); key(&ui, slint::platform::Key::Return); }
            7 => { assert_eq!(ui.get_form_end_m(), 55); click(&ui, 468.0, 318.0); }
            8 => type_text(&ui, "21:31"),
            9 => {
                key(&ui, slint::platform::Key::Return);
                assert_eq!(ui.get_form_start_m(), 30, "off-grid query must not commit");
                snapshot(&ui, "jump-invalid", host);
                key(&ui, slint::platform::Key::Backspace); type_text(&ui, "5");
            }
            10 => key(&ui, slint::platform::Key::Return),
            11 => {
                assert_eq!((ui.get_form_start_m(), ui.get_form_end_m()), (35, 55));
                click(&ui, 468.0, 354.0);
            }
            12 => type_text(&ui, "00:00"),
            13 => { key(&ui, slint::platform::Key::Escape); assert_eq!(ui.get_form_end_m(), 55); }
            14 => click(&ui, 468.0, 318.0),
            15 => type_text(&ui, "09"),
            16..=18 => {}
            19 => type_text(&ui, "07"),
            20 => key(&ui, slint::platform::Key::Return),
            21 => {
                assert_eq!((ui.get_form_start_h(), ui.get_form_start_m()), (7, 0), "pause resets prefix");
                click(&ui, 468.0, 354.0);
            }
            22 => { key(&ui, slint::platform::Key::Home); key(&ui, slint::platform::Key::Return); }
            23 => { assert!(!ui.get_form_valid()); click(&ui, 468.0, 354.0); }
            24 => { key(&ui, slint::platform::Key::End); key(&ui, slint::platform::Key::Return); }
            25 => {
                assert_eq!((ui.get_form_end_h(), ui.get_form_end_m()), (23, 55));
                click(&ui, 468.0, 318.0);
            }
            26 => type_text(&ui, "23:55"),
            27 => key(&ui, slint::platform::Key::Tab),
            28 => { assert_eq!(ui.get_form_start_h(), 7); type_text(&ui, "06"); }
            29 => {}
            30 => key(&ui, slint::platform::Key::Return),
            31 => {
                assert_eq!((ui.get_form_end_h(), ui.get_form_end_m()), (6, 0), "field switch resets prefix");
                ui.set_form_start_h(5); ui.invoke_form_interval_changed(true);
                click(&ui, 468.0, 318.0);
            }
            32 => {
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(468.0, 420.0), delta_x: 0.0, delta_y: -800.0,
                });
                type_text(&ui, "05:30");
            }
            33 => { snapshot(&ui, "jump-after-scroll", host); key(&ui, slint::platform::Key::Return); }
            34 => {
                assert_eq!((ui.get_form_start_h(), ui.get_form_start_m()), (5, 30));
                assert_eq!((ui.get_form_end_h(), ui.get_form_end_m()), (6, 0));
                println!("PASS: native typed time jump, visible matching, explicit commit, invalid/no-match, timeout, field reset, Home/End/Tab/Escape, scroll recovery");
                slint::quit_event_loop().unwrap();
            }
            _ => unreachable!(),
        }
        stage.set(stage.get() + 1);
    });
    ui.run()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = std::env::var("SMPL_CALENDAR_WAYLAND_PROBE").as_deref() == Ok("1");
    if host {
        assert_eq!(
            std::env::var("HOME")?,
            std::env::var("SMPL_CALENDAR_PROBE_HOME")?
        );
    } else {
        assert_eq!(
            std::env::var("SMPL_CALENDAR_PRIVATE_DISPLAY").as_deref(),
            Ok("1")
        );
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
    }
    smpl_common::init(
        if host {
            "smpl-calendar-nav-fixture-details"
        } else {
            "calendar-editor-probe"
        },
        1100.0,
        700.0,
    )?;
    let ui = MainWindow::new()?;
    ui.set_is_details(true);
    ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
    ui.set_form_title("Private editor fixture".into());
    ui.set_form_year(2026);
    ui.set_form_month(10);
    ui.set_form_day(1);
    ui.set_form_end_year(2026);
    ui.set_form_end_month(10);
    ui.set_form_end_day(1);
    ui.set_form_start_h(21);
    ui.set_form_start_m(0);
    ui.set_form_end_h(10);
    ui.set_form_end_m(0);
    let editor = editor::wire(&ui);
    editor.begin(&ui, None);
    ui.set_show_form(true);
    if std::env::args().any(|arg| arg == "--typeahead") {
        typeahead_probe(&ui, host)?;
        return Ok(());
    }
    let saves = Rc::new(Cell::new(0));
    let saved = saves.clone();
    ui.on_save_event(move || saved.set(saved.get() + 1));
    let stage = Rc::new(Cell::new(0));
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(500), move || {
        let ui = weak.unwrap();
        match stage.get() {
            0 => { snapshot(&ui, "initial", host); assert!(!ui.get_form_valid()); }
            1 => { click(&ui, 468.0, 312.0); }
            2 => { snapshot(&ui, "start-popup", host); }
            3 => { click(&ui, 468.0, 490.0); }
            4 => {
                assert_eq!((ui.get_form_start_h(), ui.get_form_start_m()), (21, 10));
                assert_eq!((ui.get_form_end_h(), ui.get_form_end_m()), (21, 30));
                assert!(ui.get_form_valid(), "{}", ui.get_form_error());
                click(&ui, 468.0, 354.0);
            }
            5 => { snapshot(&ui, "end-popup", host); }
            6 => { click(&ui, 468.0, 442.0); }
            7 => {
                assert_eq!(ui.get_form_end_m(), 25);
                assert!(ui.get_form_valid(), "manual fifteen-minute interval is valid");
                click(&ui, 468.0, 354.0);
            }
            8 => {
                for _ in 0..3 { key(&ui, slint::platform::Key::UpArrow); }
                key(&ui, slint::platform::Key::Return);
            }
            9 => {
                assert_eq!(ui.get_form_end_m(), 10);
                assert!(!ui.get_form_valid());
                assert!(!ui.get_form_error().is_empty());
                click(&ui, 696.0, 625.0);
            }
            10 => {
                assert_eq!(saves.get(), 0, "invalid interval must disable Create");
                ui.set_form_start_h(23); ui.set_form_start_m(50);
                ui.invoke_form_interval_changed(true);
                assert_eq!((ui.get_form_end_day(), ui.get_form_end_h(), ui.get_form_end_m()), (2, 0, 10));
                assert_eq!(editor.interval(&ui).unwrap().1 - editor.interval(&ui).unwrap().0, chrono::Duration::minutes(20));
                click(&ui, 550.0, 464.0);
            }
            11 => { snapshot(&ui, "repeat-popup", host); }
            12 => { click(&ui, 550.0, 552.0); }
            13 => {
                assert_eq!(ui.get_form_rec_idx(), 2, "Repeat popup must receive input over Reminder");
                click(&ui, 550.0, 519.0);
            }
            14 => { snapshot(&ui, "reminder-popup", host); }
            15 => { key(&ui, slint::platform::Key::DownArrow); key(&ui, slint::platform::Key::Return); }
            16 => {
                assert_eq!(ui.get_form_alert_idx(), 1);
                click(&ui, 550.0, 280.0);
            }
            17 => { snapshot(&ui, "date-popup", host); }
            18 => { click(&ui, 522.0, 374.0); }
            19 => {
                assert_eq!(ui.get_form_day(), 2, "Date popup selection must beat the obscured End field");
                assert_eq!(ui.get_form_end_day(), 3, "date change must batch before adjusting overnight end");
                click(&ui, 550.0, 409.0);
            }
            20 => {
                key(&ui, slint::platform::Key::PageUp);
                key(&ui, slint::platform::Key::Return);
                assert_eq!(ui.get_form_end_month(), 9);
                assert!(!ui.get_form_valid(), "earlier end date is rejected");
                ui.set_form_end_month(10);
                ui.invoke_form_interval_changed(false);
                click(&ui, 550.0, 409.0);
            }
            21 => {
                key(&ui, slint::platform::Key::Escape);
                assert!(ui.get_show_form(), "Escape dismisses a picker, not the editor");
                click(&ui, 468.0, 318.0);
            }
            22 => { click(&ui, 700.0, 200.0); }
            23 => {
                assert!(ui.get_show_form(), "outside-popup click inside dialog retains editor");
                let interval = editor.interval(&ui).unwrap();
                use provider::CalendarProvider;
                let mut store = local_provider::LocalProvider::open().unwrap();
                let saved = store.create_event(models::NewEvent {
                    title: "Overnight roundtrip".into(), description: "".into(),
                    start: interval.0, end: interval.1, all_day: false,
                    recurrence: models::Recurrence::None, recurrence_end: None,
                    color: None, alert_minutes: 0,
                }).unwrap();
                let loaded = store.events_for_day(saved.start.date_naive()).into_iter().find(|v| v.id == saved.id).unwrap();
                ui.set_form_editing_id(loaded.id as i32);
                ui.set_form_end_year(loaded.end.year()); ui.set_form_end_month(loaded.end.month() as i32);
                ui.set_form_end_day(loaded.end.day() as i32);
                ui.set_form_end_h(loaded.end.hour() as i32); ui.set_form_end_m(loaded.end.minute() as i32);
                editor.begin(&ui, Some((loaded.start, loaded.end)));
                assert_eq!(editor.interval(&ui).unwrap(), interval, "overnight edit/reopen must retain end date");
                ui.window().set_size(slint::LogicalSize::new(364.0, 360.0));
                if host {
                    std::fs::write(std::env::var("SMPL_EDITOR_RESIZE_MARKER").unwrap(), "364x360").unwrap();
                }
            }
            24 => {
                if host && ui.window().size().height as f32 / ui.window().scale_factor() > 361.0 {
                    return;
                }
                ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(180.0, 180.0), delta_x: 0.0, delta_y: -600.0,
                });
            }
            25 => {
                println!("short fixture physical size={:?}, scale={}", ui.window().size(), ui.window().scale_factor());
                snapshot(&ui, "short-scrolled", host);
            }
            26 => { click(&ui, 180.0, 286.0); }
            27 => { snapshot(&ui, "short-popup", host); }
            28 => { key(&ui, slint::platform::Key::DownArrow); key(&ui, slint::platform::Key::Return); }
            29 => {
                assert_eq!(ui.get_form_alert_idx(), 2, "short-screen popup remains operable");
                click(&ui, 316.0, 327.0);
            }
            30 => {
                assert_eq!(saves.get(), 1, "short-screen Save remains accessible");
                ui.set_form_all_day(true);
                ui.set_form_end_day(1);
                ui.invoke_form_interval_changed(false);
                let (start, end) = editor.interval(&ui).unwrap();
                assert_eq!((start.hour(), start.minute(), end.hour(), end.minute()), (0, 0, 23, 59));
                assert_eq!(start.date_naive(), end.date_naive());
                ui.set_form_all_day(false);
                ui.invoke_form_interval_changed(true);
                assert_eq!((ui.get_form_end_day(), ui.get_form_end_h(), ui.get_form_end_m()), (3, 0, 10));
            }
            _ => {
                println!("PASS: native overlapping picker pointer/keyboard selection, validation, overnight edit roundtrip, short-screen scroll");
                slint::quit_event_loop().unwrap();
            }
        }
        stage.set(stage.get() + 1);
    });
    ui.run()?;
    Ok(())
}
