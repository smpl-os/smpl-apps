//! Isolated real-FemtoVG UI fixture; run through calendar/tests/run_dashboard_probe.py.
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::Cell, rc::Rc, time::Duration};

#[path = "../src/cities.rs"]
mod cities;
#[path = "../src/dashboard.rs"]
mod dashboard;
#[allow(dead_code)]
#[path = "../src/local_provider.rs"]
mod local_provider;
#[allow(dead_code)]
#[path = "../src/models.rs"]
mod models;
#[allow(dead_code)]
#[path = "../src/navigation.rs"]
mod navigation;
#[allow(dead_code)]
#[path = "../src/provider.rs"]
mod provider;
#[path = "../src/weather.rs"]
mod weather;

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
    slint::Timer::single_shot(Duration::from_millis(80), move || {
        weak.unwrap()
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wayland_probe = std::env::var("SMPL_CALENDAR_WAYLAND_PROBE").as_deref() == Ok("1");
    if wayland_probe {
        assert_eq!(
            std::env::var("HOME")?,
            std::env::var("SMPL_CALENDAR_PROBE_HOME")?
        );
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_some());
    } else {
        assert_eq!(
            std::env::var("SMPL_CALENDAR_PRIVATE_DISPLAY").as_deref(),
            Ok("1")
        );
        assert!(std::env::var("DISPLAY")?.starts_with("127.0.0.1:"));
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
    }
    let output = std::path::PathBuf::from(std::env::var("SMPL_CALENDAR_PROBE_OUTPUT")?);
    let navigation_parent = std::env::args().any(|arg| arg == "--navigation-runtime");
    let navigation_child =
        std::env::var_os("SMPL_CALENDAR_NAV_MARKER").is_some() && !navigation_parent;
    let _compact_lock = if navigation_child {
        Some(
            if wayland_probe {
                navigation::acquire_compact_named("calendar-nav-fixture")?
            } else {
                navigation::acquire_compact()?
            }
            .expect("fixture compact instance must be unique"),
        )
    } else {
        None
    };
    smpl_common::init(
        if navigation_parent {
            if wayland_probe {
                "smpl-calendar-nav-fixture-details"
            } else {
                "smpl-calendar-details"
            }
        } else if navigation_child {
            if wayland_probe {
                "smpl-calendar-nav-fixture"
            } else {
                "smpl-calendar"
            }
        } else {
            "smpl-calendar-probe"
        },
        if navigation_parent { 1100.0 } else { 364.0 },
        if navigation_parent { 700.0 } else { 650.0 },
    )?;
    let ui = MainWindow::new()?;
    if navigation_parent {
        ui.set_is_details(true);
        ui.set_is_standalone(true);
        ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
    } else {
        ui.window().set_size(slint::LogicalSize::new(364.0, 650.0));
    }
    if navigation_parent || navigation_child {
        use provider::CalendarProvider;
        let mut events = local_provider::LocalProvider::open()?;
        if navigation_child {
            let saved = events.events_for_day(chrono::Local::now().date_naive());
            assert!(saved
                .iter()
                .any(|event| event.title == "Navigation fixture saved event"));
            let marker = std::env::var("SMPL_CALENDAR_NAV_MARKER")?;
            slint::Timer::single_shot(Duration::from_millis(300), move || {
                std::fs::write(marker, std::process::id().to_string()).unwrap();
            });
            slint::Timer::single_shot(
                Duration::from_secs(if wayland_probe { 30 } else { 8 }),
                || {
                    slint::quit_event_loop().unwrap();
                },
            );
            ui.run()?;
            return Ok(());
        }
        let now = chrono::Local::now();
        events.create_event(models::NewEvent {
            title: "Navigation fixture saved event".into(),
            description: "Isolated fixture".into(),
            start: now,
            end: now + chrono::Duration::hours(1),
            all_day: false,
            recurrence: models::Recurrence::None,
            recurrence_end: None,
            color: None,
            alert_minutes: 0,
        })?;
        ui.set_is_details(true);
        ui.set_is_standalone(true);
        ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
        if wayland_probe {
            navigation::wire_back_for_class(&ui, "smpl-calendar-nav-fixture");
        } else {
            navigation::wire_back(&ui);
        }
        ui.set_show_form(true);
        ui.set_form_title("Unsaved fixture edit".into());
        let weak = ui.as_weak();
        ui.on_cancel_form(move || weak.unwrap().set_show_form(false));
        let weak = ui.as_weak();
        let navigation_snapshot = output.join("navigation-before.ppm");
        slint::Timer::single_shot(Duration::from_millis(1800), move || {
            let ui = weak.unwrap();
            ui.invoke_close_details();
            assert!(
                ui.get_show_form(),
                "Back must preserve an open event editor"
            );
            assert_eq!(ui.get_form_title(), "Unsaved fixture edit");
            assert!(ui.get_navigation_status().contains("Save or cancel"));
            ui.invoke_cancel_form();
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(200), move || {
                let ui = weak.unwrap();
                if !wayland_probe {
                    let image = ui.window().take_snapshot().unwrap();
                    assert_eq!((image.width(), image.height()), (1100, 700));
                    println!(
                        "navigation fixture dimensions={}x{}",
                        image.width(),
                        image.height()
                    );
                    let mut bytes =
                        format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                    for pixel in image.as_slice() {
                        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                    }
                    std::fs::write(navigation_snapshot, bytes).unwrap();
                }
                click(
                    &ui,
                    ui.window().size().width as f32 / ui.window().scale_factor() - 55.0,
                    25.0,
                );
            });
        });
        let failure_marker = std::env::var_os("SMPL_CALENDAR_NAV_FAILURE_MARKER");
        let expect_failure = failure_marker.is_some();
        if let Some(marker) = failure_marker {
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_secs(4), move || {
                let ui = weak.unwrap();
                assert!(ui.window().is_visible());
                assert!(!ui.get_navigation_busy());
                assert!(ui
                    .get_navigation_status()
                    .contains("Could not return to calendar"));
                std::fs::write(marker, ui.get_navigation_status().as_str()).unwrap();
                slint::Timer::single_shot(Duration::from_secs(1), || {
                    slint::quit_event_loop().unwrap();
                });
            });
        }
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_secs(7), move || {
            let ui = weak.unwrap();
            panic!(
                "Back did not dismiss details: busy={} form={} visible={} status={}",
                ui.get_navigation_busy(),
                ui.get_show_form(),
                ui.window().is_visible(),
                ui.get_navigation_status()
            )
        });
        ui.run()?;
        assert_eq!(ui.get_navigation_status().is_empty(), !expect_failure);
        if expect_failure {
            println!("navigation: failed Back kept details visible with an explicit error");
        } else {
            println!("navigation: native Back click opened a compact process; open editor and saved event preserved");
        }
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--cache-runtime") {
        use slint::Model;
        let city = cities::City {
            name: "Fixture".into(),
            label: "Fixture only".into(),
            latitude: 0.0,
            longitude: 0.0,
            timezone: "Etc/UTC".into(),
        };
        cities::save_settings(&cities::Settings {
            cities: vec![city.clone()],
            ..Default::default()
        })?;
        let now = chrono::Utc::now();
        let config = weather::Config {
            latitude: city.latitude,
            longitude: city.longitude,
            location: city.label,
            timezone: city.timezone,
            temperature_unit: weather::TemperatureUnit::Celsius,
        };
        let forecast = weather::Forecast {
            fetched_at: now,
            utc_offset_seconds: 0,
            stale: false,
            days: (0..7)
                .map(|day| weather::ForecastDay {
                    date: now.date_naive() + chrono::Duration::days(day),
                    min: 10.0,
                    max: 20.0,
                    precipitation_probability: 25,
                    weather_code: 3,
                })
                .collect(),
        };
        weather::save_cache(&config, &forecast)?;
        let _runtime = dashboard::start(&ui);
        assert_eq!(
            ui.get_forecast().row_count(),
            7,
            "cache must be visible before first frame"
        );
        assert!(ui.get_weather_status().starts_with("Cached"));
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(1200), move || {
            let ui = weak.unwrap();
            assert_eq!(ui.get_forecast().row_count(), 7);
            assert!(
                ui.get_weather_status().starts_with("Cached"),
                "no loading flash"
            );
            println!("cached runtime: seven days present before first frame and after timer tick");
            slint::quit_event_loop().unwrap();
        });
        ui.run()?;
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--runtime") {
        let _runtime = dashboard::start(&ui);
        use slint::Model;
        assert_eq!(ui.get_clocks().row_count(), 1);
        assert_eq!(ui.get_forecast().row_count(), 0);
        assert_eq!(ui.get_weather_status(), "Weather not configured");
        ui.invoke_set_fahrenheit(true);
        assert!(ui.get_fahrenheit());
        assert_eq!(
            cities::load_settings()?.unwrap().temperature_unit,
            weather::TemperatureUnit::Fahrenheit
        );
        ui.set_city_query("x".into());
        ui.invoke_search_city();
        assert!(!ui.get_city_busy());
        assert!(ui.get_city_status().contains("at least two"));
        ui.invoke_add_city(99);
        assert!(ui.get_city_status().contains("no longer available"));
        let before = ui.get_clocks().row_data(0).unwrap().time;
        ui.set_show_preferences(true);
        let weak = ui.as_weak();
        slint::Timer::single_shot(Duration::from_millis(2200), move || {
            let ui = weak.unwrap();
            assert_ne!(before, ui.get_clocks().row_data(0).unwrap().time);
            assert!(
                !ui.get_animate_weather(),
                "preferences pause weather animations"
            );
            assert_eq!(ui.get_forecast().row_count(), 0);
            println!("runtime: local seconds advanced, settings persisted, invalid actions surfaced, no configured weather");
            slint::quit_event_loop().unwrap();
        });
        ui.run()?;
        return Ok(());
    }
    ui.set_month_name("February 2028".into());
    ui.set_selected_date_label("Today, February 29, 2028".into());
    ui.set_day_cells(ModelRc::new(VecModel::from(
        (0..42)
            .map(|i| DayCell {
                day: (i + 1) % 29 + 1,
                row: i / 7,
                col: i % 7,
                is_today: i == 29,
                is_selected: i == 29,
                has_events: i % 4 == 0 || i == 29,
                event_count: if i % 4 == 0 || i == 29 { 2 } else { 0 },
                is_other_month: i < 2 || i >= 38,
                month_offset: if i < 2 { -1 } else if i >= 38 { 1 } else { 0 },
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_weather_location("Forecast fixture · °C".into());
    ui.set_weather_status("Open-Meteo · Updated 10:24 · Rain chance".into());
    ui.set_forecast(ModelRc::new(VecModel::from(
        (0..7)
            .map(|i| ForecastDay {
                day: ["Today", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][i].into(),
                high: "24°".into(),
                low: "12°".into(),
                rain: "30%".into(),
                condition: i as i32,
                description: "Synthetic condition".into(),
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_animate_weather(true);
    if std::env::args().any(|arg| arg == "--rings") {
        assert!(!wayland_probe, "Ring screenshots require the private display");
        let stage = Rc::new(Cell::new(0));
        let weak = ui.as_weak();
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, Duration::from_millis(600), move || {
            let ui = weak.unwrap();
            let step = stage.get();
            let image = ui.window().take_snapshot().unwrap();
            let mut bytes = format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
            for pixel in image.as_slice() { bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]); }
            std::fs::write(output.join(format!("date-rings-{step}.ppm")), bytes).unwrap();
            if step == 0 {
                ui.global::<Theme>().set_bg(slint::Color::from_rgb_u8(240, 244, 250));
                ui.global::<Theme>().set_fg(slint::Color::from_rgb_u8(28, 35, 48));
                ui.global::<Theme>().set_fg_dim(slint::Color::from_rgb_u8(60, 70, 90));
                ui.global::<Theme>().set_accent(slint::Color::from_rgb_u8(25, 75, 150));
            } else if step == 1 {
                ui.set_is_details(true);
                ui.window().set_size(slint::LogicalSize::new(1100.0, 700.0));
            } else if step == 2 {
                ui.global::<Theme>().set_bg(slint::Color::from_rgb_u8(30, 30, 46));
                ui.global::<Theme>().set_fg(slint::Color::from_rgb_u8(205, 214, 244));
                ui.global::<Theme>().set_fg_dim(slint::Color::from_rgb_u8(166, 173, 200));
                ui.global::<Theme>().set_accent(slint::Color::from_rgb_u8(137, 180, 250));
            } else {
                println!("PASS: compact/details date rings rendered in light/dark themes, including selected/today/adjacent-month dates");
                slint::quit_event_loop().unwrap();
            }
            stage.set(step + 1);
        });
        ui.run()?;
        return Ok(());
    }
    let details_opened = Rc::new(Cell::new(0));
    let opened = details_opened.clone();
    ui.on_open_details(move || opened.set(opened.get() + 1));
    let stage = Rc::new(Cell::new(0usize));
    let stage_next = stage.clone();
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(500),
        move || {
            let ui = weak.unwrap();
            let stage = stage_next.get();
            if stage > 0 {
                let image = ui.window().take_snapshot().unwrap();
                assert_eq!(
                    (image.width(), image.height()),
                    (364, if stage > 9 { 480 } else { 650 })
                );
                let mut ppm =
                    format!("P6\n{} {}\n255\n", image.width(), image.height()).into_bytes();
                for pixel in image.as_slice() {
                    ppm.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(output.join(format!("stage-{stage}.ppm")), ppm).unwrap();
                let pixels = image.as_slice();
                assert!(
                    pixels.iter().any(|p| p.a == 255),
                    "opaque text/icon interiors"
                );
                if stage < 5 {
                    assert!(
                        pixels.iter().any(|p| p.a < 250),
                        "native background transparency"
                    );
                }
                println!(
                    "stage={stage} snapshot={}x{} native-femtovg",
                    image.width(),
                    image.height()
                );
            }
            if stage == 11 {
                slint::quit_event_loop().unwrap();
                return;
            }
            let count = (stage + 1).min(3);
            ui.set_clocks(ModelRc::new(VecModel::from(
                (0..count)
                    .map(|i| ClockItem {
                        name: ["Local time", "San Francisco", "Tokyo"][i].into(),
                        time: format!("{}:24:{:02}", 10 + i * 3, stage).into(),
                        date: ["Tue, 29 Feb", "Tue, 29 Feb", "Wed, 1 Mar"][i].into(),
                        zone: ["UTC", "PST", "JST"][i].into(),
                    })
                    .collect::<Vec<_>>(),
            )));
            if stage == 3 {
                click(&ui, 345.0, 18.0);
            }
            if stage == 4 {
                assert_eq!(
                    details_opened.get(),
                    1,
                    "outline button retains details action"
                );
                click(&ui, 313.0, 18.0);
                ui.set_configured_cities(ModelRc::new(VecModel::from(vec![
                    "London, United Kingdom".into(),
                    "San Francisco, United States".into(),
                    "Tokyo, Japan".into(),
                ])));
            }
            if stage == 5 {
                assert!(ui.get_show_preferences(), "settings icon opens preferences");
                click(&ui, 312.0, 30.0);
            }
            if stage == 6 {
                assert!(!ui.get_show_preferences(), "Done returns to calendar");
                ui.global::<Theme>()
                    .set_bg(slint::Color::from_rgb_u8(240, 244, 250));
                ui.global::<Theme>()
                    .set_fg(slint::Color::from_rgb_u8(28, 35, 48));
                ui.global::<Theme>()
                    .set_fg_dim(slint::Color::from_rgb_u8(60, 70, 90));
                ui.global::<Theme>()
                    .set_accent(slint::Color::from_rgb_u8(25, 75, 150));
                ui.global::<Theme>().set_opacity(1.0);
            }
            if stage == 7 {
                ui.set_forecast(ModelRc::default());
                ui.set_weather_status("Weather unavailable".into());
                ui.set_weather_message("No cached forecast. Check your connection.".into());
            }
            if stage == 8 {
                ui.set_weather_status("Weather not configured".into());
                ui.set_weather_message("Choose a primary city in Clock & weather settings.".into());
            }
            if stage == 9 {
                ui.window().set_size(slint::LogicalSize::new(364.0, 480.0));
            }
            if stage == 10 {
                ui.set_show_preferences(true);
            }
            stage_next.set(stage + 1);
        },
    );
    ui.run()?;
    Ok(())
}
