//! Native shared-editor interaction test, run only on an isolated Xvfb display.
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use i_slint_backend_winit::winit::platform::x11::EventLoopBuilderExtX11;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::buttons::Buttons;
use super::{devices, testing};

slint::slint! {
    import { AssignActionEditor } from "../../ui/main.slint";
    export component MouseEditorProbe inherits Window {
        width: 500px;
        height: 420px;
        background: #20242b;
        in property <[string]> categories;
        in property <int> category;
        in property <[string]> choices;
        in property <int> choice;
        in property <int> mode;
        in property <int> heartbeat;
        callback select-category(int);
        callback select-choice(int);
        AssignActionEditor {
            x: 10px; y: 10px; width: 480px;
            action-names: root.categories;
            action-index: root.category;
            choice-names: root.choices;
            choice-index: root.choice;
            mode: root.mode;
            select-action(index) => { root.select-category(index); }
            select-choice(index) => { root.select-choice(index); }
        }
        Text { x: 10px; y: 390px; text: "Status heartbeat " + root.heartbeat; color: #ffffff; }
    }
}

fn render(ui: &MouseEditorProbe, buttons: &Buttons) {
    let view = buttons.view();
    let strings = |items: Vec<String>| -> ModelRc<SharedString> {
        Rc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>())).into()
    };
    ui.set_categories(strings(view.action_names));
    ui.set_category(view.action_index);
    ui.set_choices(strings(view.choice_names));
    ui.set_choice(view.choice_index);
    ui.set_mode(view.editor_mode);
}

fn click(ui: &MouseEditorProbe, x: f32, y: f32) {
    click_window(ui.window(), x, y);
}

fn click_window(window: &slint::Window, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
    window.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
}

fn init_private_backend() {
    assert_eq!(std::env::var("SMPL_MOUSE_PROBE_PRIVATE_DISPLAY").as_deref(), Ok("1"));
    assert!(std::env::var("DISPLAY").unwrap().starts_with("127.0.0.1:"));
    let mut event_loop = i_slint_backend_winit::winit::event_loop::EventLoop::with_user_event();
    event_loop.with_any_thread(true);
    let backend = i_slint_backend_winit::Backend::builder()
        .with_renderer_name("femtovg")
        .with_event_loop_builder(event_loop)
        .build().unwrap();
    slint::platform::set_platform(Box::new(backend)).unwrap();
}

#[test]
#[ignore = "Requires authenticated private Xvfb; run tests/run_mouse_editor_probe.py"]
fn mouse_editor_native_dropdown_selection_survives_status_polling() {
    init_private_backend();
    let dir = testing::temp_dir("native-editor");
    let mut buttons = Buttons::load(devices::builtin(), dir.join("mouse.json"));
    buttons.show_razer();
    buttons.confirm_add("org.kde.kdenlive");
    buttons.set_kdenlive(true);
    buttons.set_live_actions(vec![("monitor_pause".into(), "Pause playback".into()), ("edit_undo".into(), "Undo".into())]);
    let buttons = Rc::new(RefCell::new(buttons));
    let ui = MouseEditorProbe::new().unwrap();
    render(&ui, &buttons.borrow());
    assert_eq!(ui.get_categories().row_count(), 7);
    let weak = ui.as_weak();
    let state = buttons.clone();
    ui.on_select_category(move |index| {
        let ui = weak.upgrade().unwrap();
        state.borrow_mut().select_action(index as usize);
        render(&ui, &state.borrow());
    });
    let weak = ui.as_weak();
    let state = buttons.clone();
    ui.on_select_choice(move |index| {
        let ui = weak.upgrade().unwrap();
        state.borrow_mut().select_choice(index as usize);
        render(&ui, &state.borrow());
    });
    let heartbeat = Timer::default();
    let weak = ui.as_weak();
    heartbeat.start(TimerMode::Repeated, Duration::from_millis(125), move || {
        let ui = weak.upgrade().unwrap();
        ui.set_heartbeat(ui.get_heartbeat() + 1);
    });
    let phase = Rc::new(Cell::new(0));
    let timer_phase = phase.clone();
    let timer = Timer::default();
    let weak = ui.as_weak();
    let state = buttons.clone();
    timer.start(TimerMode::Repeated, Duration::from_millis(1100), move || {
        let ui = weak.upgrade().unwrap();
        match timer_phase.get() {
            0 => click(&ui, 250., 26.),
            1 => {
                assert!(ui.get_heartbeat() >= 8);
                click(&ui, 250., 124.); // Category row 2: keyboard shortcut.
                assert_eq!(ui.get_category(), 2, "Native popup lost its choices during polling");
                assert_eq!(ui.get_mode(), 1);
                click(&ui, 250., 26.);
            }
            2 => {
                click(&ui, 250., 252.); // Category row 6: Kdenlive API.
                assert_eq!(ui.get_category(), 6);
                assert_eq!(ui.get_mode(), 2);
                assert_eq!(ui.get_choices().row_count(), 2);
                click(&ui, 250., 66.); // Open concrete-action dropdown.
            }
            3 => {
                click(&ui, 250., 122.); // Concrete-action row 1: Undo.
                assert_eq!(ui.get_choice(), 1, "Concrete action selection did not survive native popup/polling");
                assert!(state.borrow_mut().apply(""));
                assert!(std::fs::read_to_string(dir.join("mouse.json")).unwrap().contains("\"action\": \"edit_undo\""));
                println!("Native category and Kdenlive action popups stayed usable across {} status updates", ui.get_heartbeat());
                slint::quit_event_loop().unwrap();
            }
            _ => unreachable!(),
        }
        timer_phase.set(timer_phase.get() + 1);
    });
    ui.run().unwrap();
    assert_eq!(phase.get(), 4);
}

struct MouseBackendProbe {
    status: Arc<Mutex<serde_json::Value>>,
}

fn setup_steps() -> Vec<String> {
    let mut steps = ["btn-left", "btn-right", "btn-middle", "wheel-up", "wheel-down", "wheel-tilt-left", "wheel-tilt-right"]
        .into_iter().map(str::to_string).collect::<Vec<_>>();
    steps.extend((1..=12).map(|n| format!("grid-{n}")));
    steps.extend(["btn-dpi-up", "btn-dpi-down", "btn-scroll-mode"].into_iter().map(str::to_string));
    steps
}

fn advance_setup(status: &mut serde_json::Value, skip: bool) {
    if skip && status["setup"]["phase"] == "captured" {
        status["setup"]["captured"] = (status["setup"]["captured"].as_u64().unwrap() - 1).into();
    }
    let next = status["setup"]["step"].as_u64().unwrap() as usize;
    let steps = setup_steps();
    if next == steps.len() {
        status["setup"]["phase"] = "review".into();
        status["setup"]["target"] = "".into();
    } else {
        status["setup"]["step"] = (next + 1).into();
        status["setup"]["target"] = steps[next].clone().into();
        status["setup"]["phase"] = "idle".into();
    }
}

fn private_status() -> Arc<Mutex<serde_json::Value>> {
    Arc::new(Mutex::new(serde_json::json!({
        "apiVersion": 1, "connected": true, "ready": true, "error": "", "notice": "",
        "configPath": super::Paths::standard().assignments.display().to_string(),
        "learned": [], "unlearned": [], "identify": false, "learning": "",
        "lastSlot": "", "lastInput": "", "sequence": 0, "focusWarning": "",
        "kdenliveApi": true, "kdenliveAvailable": false, "kdenliveProfile": false,
        "calibrating": false, "guidedSetup": true
    })))
}

#[zbus::interface(name = "org.smplos.MouseRemapper1")]
impl MouseBackendProbe {
    fn status(&self) -> String {
        self.status.lock().unwrap().to_string()
    }

    fn set_identify(&self, enabled: bool) {
        let mut status = self.status.lock().unwrap();
        status["identify"] = enabled.into();
        status["learning"] = "".into();
        status["calibrating"] = false.into();
        status["setup"] = serde_json::json!({"active": false});
    }

    fn calibrate(&self) {
        let mut status = self.status.lock().unwrap();
        status["identify"] = true.into();
        status["learning"] = "grid-1".into();
        status["calibrating"] = true.into();
    }

    fn start_setup(&self) {
        let mut status = self.status.lock().unwrap();
        status["identify"] = true.into();
        status["setup"] = serde_json::json!({
            "active": true, "phase": "idle", "target": "btn-left", "step": 1, "total": setup_steps().len(),
            "captured": 0, "rows": setup_steps().iter().map(|slot| format!("{slot}: skipped")).collect::<Vec<_>>()
        });
        status["notice"] = "Click Capture next press, move over the diagram, then press and release the highlighted control.".into();
    }

    fn arm_setup(&self) {
        self.status.lock().unwrap()["setup"]["phase"] = "armed".into();
    }

    fn next_setup(&self) {
        advance_setup(&mut self.status.lock().unwrap(), false);
    }

    fn skip_setup(&self) {
        advance_setup(&mut self.status.lock().unwrap(), true);
    }

    fn retry_setup(&self) {
        let mut status = self.status.lock().unwrap();
        if status["setup"]["phase"] == "captured" {
            status["setup"]["captured"] = (status["setup"]["captured"].as_u64().unwrap() - 1).into();
        }
        status["setup"]["phase"] = "idle".into();
    }

    fn save_setup(&self) {
        let mut status = self.status.lock().unwrap();
        status["setup"]["active"] = false.into();
        status["saved"] = true.into();
    }

    fn cancel_setup(&self) {
        let mut status = self.status.lock().unwrap();
        status["setup"]["active"] = false.into();
        status["identify"] = false.into();
    }
}

#[test]
#[ignore = "Requires authenticated private Xvfb; run tests/run_mouse_editor_probe.py"]
fn mouse_identify_native_selects_diagram() {
    init_private_backend();
    let status = private_status();
    let _server = zbus::blocking::connection::Builder::session().unwrap()
        .name("org.smplos.MouseRemapper1").unwrap()
        .serve_at("/org/smplos/MouseRemapper1", MouseBackendProbe { status: status.clone() }).unwrap()
        .build().unwrap();
    let ui = crate::MainWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(900., 560.));
    ui.set_active_tab(12);
    ui.set_highlight_setting("Mouse Buttons".into());
    super::ui::install(&ui);
    let mut slots: Vec<String> = (1..=12).map(|n| format!("grid-{n}")).collect();
    slots.extend(["btn-left", "btn-right", "btn-middle", "wheel-up", "wheel-down",
        "wheel-tilt-left", "wheel-tilt-right", "btn-dpi-up", "btn-dpi-down", "btn-scroll-mode",
        "btn-back", "btn-forward", "btn-dpi"].into_iter().map(str::to_string));
    let expected_phases = slots.len() + 6;
    let phase = Rc::new(Cell::new(0usize));
    let timer_phase = phase.clone();
    let timer = Timer::default();
    let weak = ui.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(600), move || {
        let ui = weak.upgrade().unwrap();
        let phase = timer_phase.get();
        if phase == 0 {
            assert!(ui.get_ms_backend_ready(), "Production UI did not load private D-Bus status");
            assert!(!ui.get_ms_calibrating());
            click_window(ui.window(), 760., 96.);
        } else if phase == 1 {
            assert!(ui.get_ms_calibrating(), "Visible Mouse calibration button did not start calibration at 900x560");
            assert_selected(&ui, "grid-1");
            ui.invoke_ms_identify(true);
        } else if phase <= slots.len() + 1 {
            assert!(ui.get_ms_identifying());
            if phase > 2 {
                assert_selected(&ui, &slots[phase - 3]);
            }
            let mut status = status.lock().unwrap();
            status["lastSlot"] = slots[phase - 2].clone().into();
            status["lastInput"] = format!("private test input {phase}").into();
            status["sequence"] = (phase as u64).into();
        } else if phase == slots.len() + 2 {
            assert_selected(&ui, slots.last().unwrap());
            click_window(ui.window(), 760., 96.);
        } else if phase == slots.len() + 3 {
            assert!(ui.get_ms_calibrating());
            assert_selected(&ui, "grid-1");
            let mut status = status.lock().unwrap();
            status["learning"] = "grid-7".into();
            status["lastSlot"] = "grid-6".into();
            status["sequence"] = 100.into();
        } else if phase == slots.len() + 4 {
            assert_selected(&ui, "grid-7"); // Guide highlights the next position, not the last raw press.
            ui.invoke_ms_identify(false);
        } else {
            assert!(!ui.get_ms_identifying());
            assert!(!ui.get_ms_calibrating());
            println!("Production MainWindow visible calibration button worked at 900x560; Identify selected all 12 grid positions, extra buttons, wheel/tilt directions; calibration highlighted its next slot and exited cleanly.");
            slint::quit_event_loop().unwrap();
        }
        timer_phase.set(phase + 1);
    });
    ui.run().unwrap();
    assert_eq!(phase.get(), expected_phases, "Production Identify fixture did not complete");
}

fn assert_selected(ui: &crate::MainWindow, slot: &str) {
    let selected: Vec<_> = ui.get_ms_buttons().iter().filter(|button| button.selected).collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id.as_str(), slot, "Live Identify must select the diagram, not only display status text");
}

#[test]
#[ignore = "Requires authenticated private Xvfb; run tests/run_mouse_editor_probe.py"]
fn mouse_setup_native_visible_workflow() {
    init_private_backend();
    let status = private_status();
    let _server = zbus::blocking::connection::Builder::session().unwrap()
        .name("org.smplos.MouseRemapper1").unwrap()
        .serve_at("/org/smplos/MouseRemapper1", MouseBackendProbe { status: status.clone() }).unwrap()
        .build().unwrap();
    let ui = crate::MainWindow::new().unwrap();
    ui.window().set_size(slint::LogicalSize::new(900., 560.));
    ui.set_active_tab(12);
    ui.set_highlight_setting("Mouse Buttons".into());
    super::ui::install(&ui);
    let stage = Rc::new(Cell::new(0));
    let current = Rc::new(Cell::new(0usize));
    let retried = Rc::new(Cell::new(false));
    let timer_stage = stage.clone();
    let timer = Timer::default();
    let weak = ui.as_weak();
    let started = std::time::Instant::now();
    timer.start(TimerMode::Repeated, Duration::from_millis(350), move || {
        let ui = weak.upgrade().unwrap();
        assert!(started.elapsed() < Duration::from_secs(75),
            "Visible setup workflow stalled at stage {}, step {}, phase {}, status {}",
            timer_stage.get(), current.get(), ui.get_ms_setup_phase(), ui.get_ms_backend_status());
        match timer_stage.get() {
            0 => {
                if !ui.get_ms_backend_ready() { return; }
                click_window(ui.window(), 590., 96.);
                timer_stage.set(1);
            }
            1 => {
                if !ui.get_ms_setup_active() || ui.get_ms_setup_phase() != "idle" { return; }
                let step = status.lock().unwrap()["setup"]["step"].as_u64().unwrap() as usize;
                if step != current.get() + 1 { return; }
                assert_selected(&ui, &setup_steps()[current.get()]);
                click_window(ui.window(), 190., 493.);
                timer_stage.set(2);
            }
            2 => {
                if ui.get_ms_setup_phase() != "armed" { return; }
                let mut status = status.lock().unwrap();
                let target = setup_steps()[current.get()].clone();
                status["setup"]["phase"] = "captured".into();
                status["setup"]["captured"] = (status["setup"]["captured"].as_u64().unwrap() + 1).into();
                status["setup"]["rows"][current.get()] = format!("{target}: private input source/code captured").into();
                status["notice"] = format!("Captured {target}. Next or Try again; nothing saved until final review.").into();
                timer_stage.set(3);
            }
            3 => {
                if ui.get_ms_setup_phase() != "captured" { return; }
                if !retried.get() {
                    retried.set(true);
                    click_window(ui.window(), 370., 493.);
                    timer_stage.set(1);
                    return;
                }
                if current.get() == setup_steps().len() - 1 {
                    click_window(ui.window(), 500., 493.); // Skip a captured control: keep its existing mapping.
                    timer_stage.set(4);
                } else {
                    click_window(ui.window(), 620., 493.);
                    current.set(current.get() + 1);
                    timer_stage.set(1);
                }
            }
            4 => {
                if ui.get_ms_setup_phase() != "review" { return; }
                assert_eq!(ui.get_ms_setup_captured(), 21);
                assert_eq!(ui.get_ms_setup_rows().row_count(), 22);
                assert!(status.lock().unwrap().get("saved").is_none(), "Setup saved before explicit review confirmation");
                click_window(ui.window(), 250., 493.);
                timer_stage.set(5);
            }
            5 => {
                if ui.get_ms_setup_active() { return; }
                assert_eq!(status.lock().unwrap()["saved"], true);
                click_window(ui.window(), 590., 96.);
                timer_stage.set(6);
            }
            6 => {
                if !ui.get_ms_setup_active() { return; }
                click_window(ui.window(), 745., 493.);
                timer_stage.set(7);
            }
            7 => {
                if ui.get_ms_setup_active() { return; }
                assert!(!ui.get_ms_identifying());
                click_window(ui.window(), 590., 96.);
                timer_stage.set(8);
            }
            8 => {
                if !ui.get_ms_setup_active() { return; }
                ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed {
                    text: slint::platform::Key::Escape.into(),
                });
                ui.window().dispatch_event(slint::platform::WindowEvent::KeyReleased {
                    text: slint::platform::Key::Escape.into(),
                });
                timer_stage.set(9);
            }
            9 => {
                if ui.get_ms_setup_active() { return; }
                assert!(!ui.get_ms_identifying());
                println!("Visible 900x560 setup: all 22 positions highlighted, capture/next/retry/skip/review/save/cancel clicked, Escape cancelled.");
                timer_stage.set(10);
                slint::quit_event_loop().unwrap();
            }
            _ => unreachable!(),
        }
    });
    ui.run().unwrap();
    assert_eq!(stage.get(), 10);
}
