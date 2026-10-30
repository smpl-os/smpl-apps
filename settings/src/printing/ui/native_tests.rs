use super::*;
use crate::instance::{self, Request};
use i_slint_backend_winit::winit::platform::x11::EventLoopBuilderExtX11;
use slint::{
    platform::{PointerEventButton, WindowEvent},
    Model,
};
use std::cell::Cell;
use std::sync::Mutex;

#[derive(Default)]
struct Fixture {
    actions: Mutex<Vec<(i32, Action, String)>>,
    fail: Mutex<bool>,
    held: Mutex<bool>,
    empty_partial: Mutex<bool>,
}

impl Backend for Fixture {
    fn status(&self) -> Result<Snapshot, String> {
        std::thread::sleep(Duration::from_millis(200));
        if *self.fail.lock().unwrap() {
            return Err("Synthetic permission failure; check local CUPS access.".into());
        }
        let mut snapshot =
            super::super::parse_snapshot(include_str!("../fixtures/status-v1.json"), true)?;
        if *self.held.lock().unwrap() {
            snapshot.jobs[0].state = "held".into();
            snapshot.jobs[0].can_hold = false;
            snapshot.jobs[0].can_release = true;
        }
        if *self.empty_partial.lock().unwrap() {
            snapshot.jobs.clear();
            snapshot.active_complete = false;
            snapshot.history_complete = false;
        }
        Ok(snapshot)
    }

    fn action(&self, job: &Job, action: Action) -> Result<(), String> {
        self.actions
            .lock()
            .unwrap()
            .push((job.id, action, job.identity.clone()));
        match action {
            Action::Hold => *self.held.lock().unwrap() = true,
            Action::Release => *self.held.lock().unwrap() = false,
            Action::Cancel => {}
        }
        Ok(())
    }
}

fn click(ui: &MainWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn action(ui: &MainWindow, action: &str) {
    let job = ui.get_print_jobs().row_data(0).unwrap();
    ui.invoke_print_action(job.id, job.printer_id, job.identity, action.into());
}

#[test]
#[ignore = "Requires authenticated private Xvfb; run python tests/run_printing_probe.py"]
fn printing_native_queue_workflow() {
    assert_eq!(
        std::env::var("SMPL_SETTINGS_PROBE_PRIVATE_DISPLAY").as_deref(),
        Ok("1")
    );
    assert!(std::env::var("DISPLAY").unwrap().starts_with("127.0.0.1:"));
    let mut event_loop = i_slint_backend_winit::winit::event_loop::EventLoop::with_user_event();
    event_loop.with_any_thread(true);
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name("skia-software")
        .with_event_loop_builder(event_loop)
        .build()
        .unwrap();
    slint::platform::set_platform(Box::new(backend)).unwrap();
    let ui = MainWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(900., 560.));
    ui.set_active_tab(13);
    let fixture = Arc::new(Fixture::default());
    let _printing = install_with(&ui, fixture.clone());
    let phase = Rc::new(Cell::new(0));
    let timer = Timer::default();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(500), move || {
        let ui = weak.upgrade().unwrap();
        if ui.get_print_busy() {
            return;
        }
        let step = phase.get();
        match step {
            0 => {
                assert!(ui.get_print_ready());
                assert_eq!(ui.get_print_printers().row_count(), 2);
                assert_eq!(ui.get_print_jobs().row_count(), 1);
                assert!(ui.get_print_warning().contains("VPN"));
                assert!(ui.get_print_warning().contains("History is incomplete"));
                action(&ui, "cancel");
                assert!(!ui.get_print_confirmation().is_empty(), "busy={}, tab={}, status={}", ui.get_print_busy(), ui.get_active_tab(), ui.get_print_status());
                assert!(fixture.actions.lock().unwrap().is_empty());
            }
            1 => {
                ui.window().dispatch_event(WindowEvent::KeyPressed { text: slint::platform::Key::Escape.into() });
                ui.window().dispatch_event(WindowEvent::KeyReleased { text: slint::platform::Key::Escape.into() });
                assert!(ui.get_print_confirmation().is_empty());
                assert!(fixture.actions.lock().unwrap().is_empty());
                action(&ui, "cancel");
            }
            2 => {
                // Keep job is on the left of the centered, production dialog.
                click(&ui, 335., 345.);
                assert!(ui.get_print_confirmation().is_empty(), "Keep job must dismiss the native dialog");
                assert!(fixture.actions.lock().unwrap().is_empty());
                action(&ui, "cancel");
            }
            3 => {
                click(&ui, 560., 345.);
            }
            4 => {
                assert!(ui.get_print_confirmation().is_empty());
                assert_eq!(fixture.actions.lock().unwrap().len(), 1);
                assert_eq!(fixture.actions.lock().unwrap()[0].0, 41);
                assert_eq!(fixture.actions.lock().unwrap()[0].1, Action::Cancel);
                instance::apply(&ui, &Request::from_launch(Some(13), "Print History"));
            }
            5 => {
                assert!(ui.get_print_history());
                assert_eq!(ui.get_print_jobs().row_count(), 1);
                assert!(ui.get_print_jobs().row_data(0).unwrap().detail.contains("Old_queue"));
                assert!(!ui.get_print_jobs().row_data(0).unwrap().can_cancel);
                *fixture.fail.lock().unwrap() = true;
                ui.invoke_print_refresh();
            }
            6 => {
                assert!(!ui.get_print_ready());
                assert!(ui.get_print_status().contains("last known"));
                assert_eq!(ui.get_print_jobs().row_count(), 1);
                action(&ui, "cancel");
                assert!(ui.get_print_confirmation().is_empty());
                *fixture.fail.lock().unwrap() = false;
                instance::apply(&ui, &Request::from_launch(Some(13), "Print Queue"));
                ui.window().set_size(slint::LogicalSize::new(500., 350.));
            }
            7 => {
                assert!(ui.get_print_ready());
                assert!(!ui.get_print_history());
                let pixels = ui.window().take_snapshot().unwrap();
                assert_eq!((pixels.width(), pixels.height()), (500, 350));
                action(&ui, "cancel");
                ui.set_active_tab(0);
            }
            8 => {
                assert!(ui.get_print_confirmation().is_empty());
                assert_eq!(fixture.actions.lock().unwrap().len(), 1);
                ui.set_active_tab(13);
            }
            9 => {
                action(&ui, "hold");
            }
            10 => {
                assert!(ui.get_print_jobs().row_data(0).unwrap().can_release);
                assert!(!ui.get_print_jobs().row_data(0).unwrap().can_hold);
                assert_eq!(fixture.actions.lock().unwrap()[1].1, Action::Hold);
                action(&ui, "release");
            }
            11 => {
                assert!(ui.get_print_jobs().row_data(0).unwrap().can_hold);
                assert!(!ui.get_print_jobs().row_data(0).unwrap().can_release);
                assert_eq!(fixture.actions.lock().unwrap()[2].1, Action::Release);
                let job = ui.get_print_jobs().row_data(0).unwrap();
                ui.invoke_print_action(job.id, job.printer_id, "stale-identity".into(), "hold".into());
                assert!(ui.get_print_status().contains("selected job changed"));
                assert_eq!(fixture.actions.lock().unwrap().len(), 3);
                *fixture.empty_partial.lock().unwrap() = true;
                ui.invoke_print_refresh();
            }
            12 => {
                assert!(ui.get_print_ready());
                assert_eq!(ui.get_print_jobs().row_count(), 0);
                assert!(!ui.get_print_jobs_complete());
                assert!(ui.get_print_warning().contains("Active job visibility is incomplete"));
                println!("Production Printing page: distinct queues, native Keep/Confirm/Escape, exact cancellation, hold/release readback, stale-identity refusal, history deep links, stale/error and incomplete-empty handling, minimum-size rendering and navigation dismissal verified with synthetic data.");
                slint::quit_event_loop().unwrap();
            }
            _ => panic!("unexpected fixture phase"),
        }
        phase.set(step + 1);
    });
    ui.run().unwrap();
}
