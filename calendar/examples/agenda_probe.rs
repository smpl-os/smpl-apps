//! Isolated native regression for one-shot agenda scrolling. No app data access.
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::Cell;
use std::rc::Rc;

slint::include_modules!();

fn rows(label: &str) -> Vec<CalEvent> {
    let mut rows: Vec<_> = (0..24)
        .map(|index| CalEvent {
            id: index,
            title: format!("Fixture event {index}").into(),
            time_label: "09:00 – 10:00".into(),
            is_ongoing: index == 14 || index == 15,
            all_day: index == 0,
            section_label: match index {
                0 => "All day".into(),
                1 => "Timed events".into(),
                _ => "".into(),
            },
            ..CalEvent::default()
        })
        .collect();
    rows.insert(
        16,
        CalEvent {
            id: -1,
            title: label.into(),
            is_now_marker: true,
            ..CalEvent::default()
        },
    );
    rows
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("SMPL_AGENDA_PROBE_PRIVATE_DISPLAY").as_deref() != Ok("1") {
        return Err("Run only with the authenticated private-display fixture runner".into());
    }
    smpl_common::init("smpl-calendar-agenda-probe", 364.0, 650.0)?;
    let ui = MainWindow::new()?;
    ui.set_clocks(ModelRc::from(Rc::new(VecModel::from(vec![ClockItem {
        name: "Local".into(),
        time: "10:30:00".into(),
        date: "Friday".into(),
        zone: "UTC".into(),
    }]))));
    ui.set_is_today_selected(true);
    ui.set_day_events(ModelRc::from(Rc::new(VecModel::from(
        rows("Now 10:30")
            .into_iter()
            .filter(|event| !event.is_now_marker)
            .collect::<Vec<_>>(),
    ))));
    ui.set_compact_agenda(ModelRc::from(Rc::new(VecModel::from(rows("Now 10:30")))));
    ui.show()?;
    ui.window().set_size(slint::LogicalSize::new(364.0, 460.0));
    let stage = Rc::new(Cell::new(0));
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(150), move || {
        let ui = weak.upgrade().unwrap();
        match stage.get() {
            0 => {
                ui.set_agenda_scroll_pending(true);
                ui.set_agenda_scroll_request(1);
            }
            1 => {
                assert!(!ui.get_agenda_scroll_pending(), "Now marker did not consume the scroll request");
                assert!(ui.get_agenda_scroll_y() < -100.0, "Agenda did not scroll to its actual marker row");
                assert!(ui.get_agenda_page_scroll_y() < 0.0, "The current-time pane remained below the window");
                ui.set_agenda_scroll_y(-42.0);
                ui.set_agenda_page_scroll_y(-20.0);
                ui.set_compact_agenda(ModelRc::from(Rc::new(VecModel::from(rows("Now 10:31")))));
            }
            2 => {
                assert!((ui.get_agenda_scroll_y() + 42.0).abs() < 1.0, "Minute/model update stole the user's scroll position");
                assert!((ui.get_agenda_page_scroll_y() + 20.0).abs() < 1.0, "Minute update moved the dashboard page");
                ui.set_agenda_scroll_pending(true);
                ui.set_agenda_scroll_request(2);
            }
            _ => {
                assert!(!ui.get_agenda_scroll_pending());
                assert!(ui.get_agenda_scroll_y() < -100.0, "Selecting today did not scroll again");
                println!("PASS: native agenda open/selection scroll and minute-refresh scroll retention");
                slint::quit_event_loop().unwrap();
            }
        }
        stage.set(stage.get() + 1);
    });
    slint::run_event_loop()?;
    Ok(())
}
