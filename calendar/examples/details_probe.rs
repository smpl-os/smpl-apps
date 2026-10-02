//! Private native details fixture. Never opens a database or user configuration.
use i_slint_backend_winit::WinitWindowAccessor;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::Cell;
use std::io::{BufRead, Write};
use std::rc::Rc;

slint::include_modules!();
#[path = "../src/window_controls.rs"]
mod window_controls;

fn click(ui: &MainWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
    ] {
        ui.window().dispatch_event(event);
    }
}

struct Measurement {
    size: slint::LogicalSize,
    shown: i32,
    first: i32,
    next_row: i32,
    x: f32,
    awaiting_release: bool,
}

fn begin_measure(ui: &MainWindow, last_id: &Cell<i32>, host: bool) -> Measurement {
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    if let Some(image) = (!host).then(|| ui.window().take_snapshot().ok()).flatten() {
        std::fs::create_dir_all("target/native-details-probe").unwrap();
        let path = format!(
            "target/native-details-probe/{}x{}.ppm",
            image.width(),
            image.height()
        );
        let mut file = std::fs::File::create(path).unwrap();
        write!(file, "P6\n{} {}\n255\n", image.width(), image.height()).unwrap();
        for pixel in image.as_slice() {
            file.write_all(&[pixel.r, pixel.g, pixel.b]).unwrap();
        }
    }
    let shown = ui.invoke_details_visible_count(0);
    let first = ui.invoke_details_first_visible(0);
    assert_eq!(ui.global::<DetailsGrid>().invoke_capacity(70.0, 12), 1);
    assert_eq!(
        ui.global::<DetailsGrid>()
            .invoke_first_visible(12, 12, 3, true),
        9
    );
    let x = if size.width >= 760.0 { 241.0 } else { 10.0 };
    assert!(shown > 0);
    last_id.set(-1);
    click(ui, x, 112.0);
    Measurement {
        size,
        shown,
        first,
        next_row: 0,
        x,
        awaiting_release: true,
    }
}

fn advance_measure(
    ui: &MainWindow,
    measurement: &mut Measurement,
    last_id: &Cell<i32>,
    last_day: &Cell<i32>,
) -> Option<i32> {
    let Measurement {
        size,
        shown,
        first,
        next_row,
        x,
        awaiting_release,
    } = measurement;
    if *awaiting_release {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerReleased {
                position: slint::LogicalPosition::new(*x, 112.0 + *next_row as f32 * 17.0),
                button: slint::platform::PointerEventButton::Left,
            });
        *awaiting_release = false;
        return None;
    }
    // Flickable defers press/release delivery. Observe each hit on the next
    // native-loop tick rather than recursively pumping timers from a timer.
    if *next_row < *shown {
        assert_eq!(
            last_id.get(),
            1000 + *first + *next_row,
            "Rendered row {} was missing or pointed at the wrong event",
            next_row
        );
        *next_row += 1;
        last_id.set(-1);
        last_day.set(-1);
        click(ui, *x, 112.0 + *next_row as f32 * 17.0);
        *awaiting_release = true;
        return None;
    }
    assert_eq!(
        last_day.get(),
        1,
        "Overflow did not select the complete day"
    );
    assert!(
        ui.get_show_day_panel(),
        "Overflow did not reveal the day panel"
    );
    ui.set_show_day_panel(false);
    println!(
        "DETAILS {}",
        serde_json::json!({
            "width": size.width, "height": size.height, "shown": *shown,
            "first": *first, "hidden": 12 - *shown, "maximized": ui.get_details_maximized(),
            "real_row_clicks": *shown, "overflow_action": true,
        })
    );
    Some(*shown)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host_flag = std::env::var("SMPL_CALENDAR_WAYLAND_PROBE").as_deref() == Ok("1");
    let host = host_flag || std::env::args().any(|arg| arg == "--host");
    let output = if host {
        let home = std::env::var("HOME")?;
        let private_home = std::env::var("SMPL_CALENDAR_PROBE_HOME")?;
        if !host_flag || home != private_home || !std::path::Path::new(&private_home).is_absolute()
        {
            return Err(
                "Host probe requires its explicit guard and matching absolute private HOME".into(),
            );
        }
        Some(std::path::PathBuf::from(std::env::var(
            "SMPL_CALENDAR_PROBE_OUTPUT",
        )?))
    } else {
        if std::env::var("SMPL_DETAILS_PROBE_ISOLATED").as_deref() != Ok("1") {
            return Err("Use only an isolated private-display fixture".into());
        }
        None
    };
    let app_id = if host {
        "smpl-calendar-nav-fixture-details"
    } else {
        "smpl-calendar-details-probe"
    };
    smpl_common::init(app_id, 1100.0, 700.0)?;
    let ui = MainWindow::new()?;
    ui.set_is_details(true);
    ui.set_month_name("October 2026 · fixture".into());
    let events = ModelRc::from(Rc::new(VecModel::from(
        (0..12)
            .map(|index| GridEvent {
                id: 1000 + index,
                title: format!("Actual fixture event {index}").into(),
                time_label: format!("{:02}:00", 8 + index).into(),
            })
            .collect::<Vec<_>>(),
    )));
    let cells = (0..42)
        .map(|index| DayCell {
            day: index + 1,
            row: index / 7,
            col: index % 7,
            events: if index == 0 {
                events.clone()
            } else {
                ModelRc::default()
            },
            event_count: if index == 0 { 12 } else { 0 },
            has_events: index == 0,
            is_today: index == 0,
            first_upcoming: if index == 0 { 6 } else { 0 },
            ..DayCell::default()
        })
        .collect::<Vec<_>>();
    ui.set_day_cells(ModelRc::from(Rc::new(VecModel::from(cells))));
    let last_id = Rc::new(Cell::new(-1));
    let observed = last_id.clone();
    ui.on_edit_event(move |id| observed.set(id));
    let last_day = Rc::new(Cell::new(-1));
    let observed = last_day.clone();
    ui.on_select_day(move |day| observed.set(day));
    let _window_controls = window_controls::wire(&ui);
    ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
    ui.show()?;

    let (sender, receiver) = std::sync::mpsc::channel();
    if host {
        std::thread::spawn(move || {
            for command in std::io::stdin().lock().lines().map_while(Result::ok) {
                if sender.send(command).is_err() {
                    break;
                }
            }
        });
    }
    let stage = Cell::new(0);
    let initial = Cell::new(0);
    let small = Cell::new(0);
    let pending = std::cell::RefCell::new(None::<Measurement>);
    let previous_status = std::cell::RefCell::new(String::new());
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(200), move || {
        let ui = weak.upgrade().unwrap();
        if host {
            let had_pending = pending.borrow().is_some();
            for command in receiver.try_iter().take(8) {
                match command.trim() {
                    "maximize" if !ui.get_details_maximized() => ui.invoke_toggle_details_maximized(),
                    "restore" if ui.get_details_maximized() => ui.invoke_toggle_details_maximized(),
                    "measure" if pending.borrow().is_none() => {
                        *pending.borrow_mut() = Some(begin_measure(&ui, &last_id, true));
                    }
                    "quit" => { slint::quit_event_loop().unwrap(); }
                    _ => {}
                }
            }
            let size = ui.window().size().to_logical(ui.window().scale_factor());
            let status = serde_json::json!({
                "pid": std::process::id(), "phase": "ready",
                "width": size.width, "height": size.height,
                "shown": ui.invoke_details_visible_count(0),
                "first": ui.invoke_details_first_visible(0),
                "maximized": ui.get_details_maximized(),
                "native_maximized": ui.window().with_winit_window(|window| window.is_maximized()),
            }).to_string();
            if *previous_status.borrow() != status {
                println!("DETAILS_STATE {status}");
                std::io::stdout().flush().unwrap();
                if let Some(output) = &output {
                    let staging = output.with_extension("details-next");
                    std::fs::write(&staging, &status).unwrap();
                    std::fs::rename(&staging, output).unwrap();
                }
                *previous_status.borrow_mut() = status;
            }
            let finished = if had_pending {
                pending.borrow_mut().as_mut()
                    .and_then(|measurement| advance_measure(&ui, measurement, &last_id, &last_day))
            } else { None };
            if finished.is_some() { *pending.borrow_mut() = None; }
            return;
        }
        if stage.get() >= 3 {
                let native = ui.window().with_winit_window(|window| window.is_maximized()).unwrap();
                assert_eq!(ui.get_details_maximized(), native, "Maximize label diverged from native state");
                if native { ui.invoke_toggle_details_maximized(); }
                println!("PASS: responsive actual event rows, overflow clicks, and native maximize-state synchronization");
                slint::quit_event_loop().unwrap();
                return;
        }
        if pending.borrow().is_none() {
            *pending.borrow_mut() = Some(begin_measure(&ui, &last_id, false));
            return;
        }
        let finished = pending.borrow_mut().as_mut()
            .and_then(|measurement| advance_measure(&ui, measurement, &last_id, &last_day));
        if let Some(shown) = finished {
            *pending.borrow_mut() = None;
            match stage.get() {
                0 => {
                    initial.set(shown);
                    assert_eq!(shown, 3);
                    ui.window().set_size(slint::LogicalSize::new(500.0, 360.0));
                }
                1 => {
                    small.set(shown);
                    assert!(shown < initial.get());
                    ui.window().set_size(slint::LogicalSize::new(1600.0, 1100.0));
                }
                _ => {
                    assert!(shown > initial.get() && shown > 3);
                    ui.invoke_toggle_details_maximized();
                }
            }
            stage.set(stage.get() + 1);
        }
    });
    slint::run_event_loop()?;
    Ok(())
}
