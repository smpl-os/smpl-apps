use crate::{cities, weather, ClockItem, ForecastDay, MainWindow};
use chrono::{DateTime, Local, Utc};
use chrono_tz::Tz;
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

const RETRY: Duration = Duration::from_secs(5 * 60);
const SEARCH_COOLDOWN: Duration = Duration::from_secs(5);

enum Update {
    Search(String, Result<cities::SearchResults, String>),
    Forecast(u64, Result<weather::Forecast, String>, Option<String>),
}

struct State {
    settings: cities::Settings,
    settings_error: Option<String>,
    results: Vec<cities::City>,
    forecast: Option<weather::Forecast>,
    generation: u64,
    fetching: bool,
    next_refresh: Instant,
    cached: bool,
    weather_error: Option<String>,
    cache_warning: bool,
    weather_minute: i64,
    search_cancelled: Arc<AtomicBool>,
    next_search: Instant,
}

fn report(ui: &MainWindow, message: impl Into<String>) {
    let message = message.into();
    eprintln!("[calendar] {message}");
    ui.set_city_status(message.into());
}

fn update_clocks(ui: &MainWindow, settings: &cities::Settings) {
    let now = Utc::now();
    let clocks = if settings.cities.is_empty() {
        vec![clock_item("Local time", None, now)]
    } else {
        settings
            .cities
            .iter()
            .map(|city| {
                // Settings validation guarantees an IANA timezone, including after load.
                clock_item(
                    &city.label,
                    Some(city.timezone.parse().expect("validated timezone")),
                    now,
                )
            })
            .collect()
    };
    let model = ui.get_clocks();
    if model.row_count() == clocks.len() {
        for (index, clock) in clocks.into_iter().enumerate() {
            model.set_row_data(index, clock);
        }
    } else {
        ui.set_clocks(ModelRc::new(VecModel::from(clocks)));
    }
}

fn update_settings(ui: &MainWindow, state: &State) {
    ui.set_configured_cities(ModelRc::new(VecModel::from(city_display_labels(
        &state.settings.cities,
    ))));
    ui.set_primary_city(state.settings.primary as i32);
    ui.set_fahrenheit(state.settings.temperature_unit == weather::TemperatureUnit::Fahrenheit);
    update_clocks(ui, &state.settings);
}

fn city_display_labels(cities: &[cities::City]) -> Vec<slint::SharedString> {
    cities
        .iter()
        .map(|city| {
            let ambiguous = !city.label.contains(',')
                || cities
                    .iter()
                    .filter(|other| other.label == city.label)
                    .count()
                    > 1;
            if ambiguous {
                format!(
                    "{}\n{:.4}, {:.4}",
                    city.label, city.latitude, city.longitude
                )
                .into()
            } else {
                city.label.clone().into()
            }
        })
        .collect()
}

fn weather_config(settings: &cities::Settings) -> Option<weather::Config> {
    settings
        .cities
        .get(settings.primary)
        .map(|city| weather::Config {
            latitude: city.latitude,
            longitude: city.longitude,
            location: city.label.clone(),
            timezone: city.timezone.clone(),
            temperature_unit: settings.temperature_unit,
        })
}

fn weather_key(
    settings: &cities::Settings,
) -> Option<(f64, f64, String, String, weather::TemperatureUnit)> {
    settings.cities.get(settings.primary).map(|city| {
        (
            city.latitude,
            city.longitude,
            city.label.clone(),
            city.timezone.clone(),
            settings.temperature_unit,
        )
    })
}

fn condition_index(condition: weather::Condition) -> i32 {
    use weather::Condition::*;
    match condition {
        Clear => 0,
        PartlyCloudy => 1,
        Cloudy => 2,
        Drizzle | Rain => 3,
        Snow => 4,
        Fog => 5,
        Thunderstorm => 6,
    }
}

fn update_weather(ui: &MainWindow, state: &State) {
    let Some(city) = state.settings.cities.get(state.settings.primary) else {
        ui.set_forecast(ModelRc::default());
        ui.set_weather_location("7-day forecast".into());
        ui.set_weather_status(if state.settings_error.is_some() {
            "Preferences unavailable".into()
        } else {
            "Weather not configured".into()
        });
        ui.set_weather_message(
            state
                .settings_error
                .clone()
                .unwrap_or_else(|| "Choose your primary city in Clock & weather settings.".into())
                .into(),
        );
        return;
    };
    ui.set_weather_location(
        format!(
            "{} · {}",
            city.name,
            state.settings.temperature_unit.symbol()
        )
        .into(),
    );
    let today = city
        .local_datetime(Utc::now())
        .expect("validated timezone")
        .date_naive();
    let days = state
        .forecast
        .as_ref()
        .map(|forecast| {
            forecast
                .days
                .iter()
                .filter(|day| day.date >= today)
                .map(|day| {
                    let condition = day.condition().expect("validated weather code");
                    ForecastDay {
                        day: if day.date == today {
                            "Today".into()
                        } else {
                            day.date.format("%a").to_string().into()
                        },
                        high: format!("{:.0}°", day.max).into(),
                        low: format!("{:.0}°", day.min).into(),
                        rain: format!("{}%", day.precipitation_probability).into(),
                        condition: condition_index(condition),
                        description: format!(
                            "{}: {}, high {:.0}, low {:.0}, rain chance {}%",
                            day.date,
                            condition.label(),
                            day.max,
                            day.min,
                            day.precipitation_probability
                        )
                        .into(),
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let count = days.len();
    ui.set_forecast(ModelRc::new(VecModel::from(days)));
    if let Some(forecast) = state.forecast.as_ref().filter(|_| count > 0) {
        let stale = forecast.stale
            || Utc::now()
                .signed_duration_since(forecast.fetched_at)
                .num_hours()
                >= 1;
        let kind = if state.weather_error.is_some() {
            "Stale / offline"
        } else if stale {
            "Stale"
        } else if state.cached {
            "Cached"
        } else {
            "Updated"
        };
        let time = forecast.fetched_at.with_timezone(&Local).format("%H:%M");
        ui.set_weather_status(
            format!(
                "{kind} {time} · Open-Meteo · Rain chance{}{}",
                if count < 7 { " · Partial" } else { "" },
                if state.cache_warning {
                    " · Cache unavailable"
                } else {
                    ""
                },
            )
            .into(),
        );
    } else if state.fetching {
        ui.set_weather_status("Loading · Open-Meteo".into());
        ui.set_weather_message(format!("Loading forecast for {}…", city.name).into());
    } else {
        ui.set_weather_status("Weather unavailable · Open-Meteo".into());
        ui.set_weather_message(
            state
                .weather_error
                .clone()
                .unwrap_or_else(|| {
                    "No current cached forecast. Check your connection; retrying in 5 minutes."
                        .into()
                })
                .into(),
        );
    }
}

fn begin_weather_request(state: &mut State) -> Option<(u64, weather::Config)> {
    let config = weather_config(&state.settings)?;
    if state.fetching || Instant::now() < state.next_refresh {
        return None;
    }
    state.fetching = true;
    Some((state.generation, config))
}

fn launch_weather(state: &mut State, sender: &Sender<Update>) {
    let Some((generation, config)) = begin_weather_request(state) else {
        return;
    };
    let sender = sender.clone();
    std::thread::spawn(move || {
        let result = weather::fetch(&config).map_err(|e| format!("Forecast unavailable: {e:#}"));
        let cache_error = result.as_ref().ok().and_then(|forecast| {
            weather::save_cache(&config, forecast)
                .err()
                .map(|e| format!("Could not cache forecast: {e:#}"))
        });
        let _ = sender.send(Update::Forecast(generation, result, cache_error));
    });
}

fn apply_cached(
    state: &mut State,
    cache: Result<Option<weather::Forecast>, String>,
    now: DateTime<Utc>,
) {
    state.forecast = None;
    state.cached = false;
    state.weather_error = None;
    state.cache_warning = false;
    state.next_refresh = Instant::now();
    match cache {
        Ok(Some(forecast)) => {
            state.next_refresh += forecast.refresh_after(now);
            state.forecast = Some(forecast);
            state.cached = true;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("[calendar] {error}");
            state.weather_error = Some(error);
        }
    }
}

fn restore_cached(state: &mut State) {
    let cache = weather_config(&state.settings).map_or(Ok(None), |config| {
        weather::load_cache(&config).map_err(|e| format!("Forecast cache: {e:#}"))
    });
    apply_cached(state, cache, Utc::now());
}

fn finish_forecast(
    state: &mut State,
    result: Result<weather::Forecast, String>,
    cache_warning: bool,
) {
    match result {
        Ok(forecast) => {
            state.next_refresh = Instant::now() + forecast.refresh_after(Utc::now());
            state.cached = false;
            state.forecast = Some(forecast);
            state.weather_error = None;
            state.cache_warning = cache_warning;
        }
        Err(error) => {
            eprintln!("[calendar] {error}");
            state.weather_error = Some(error);
            state.next_refresh = Instant::now() + RETRY;
        }
    }
}

fn change_settings(
    ui: &MainWindow,
    shared: &Rc<RefCell<State>>,
    sender: &Sender<Update>,
    change: impl FnOnce(&mut cities::Settings) -> anyhow::Result<()>,
) {
    let mut state = shared.borrow_mut();
    let result = (|| -> anyhow::Result<cities::Settings> {
        if let Some(error) = &state.settings_error {
            anyhow::bail!("{error}");
        }
        let current = cities::load_settings()?.unwrap_or_default();
        anyhow::ensure!(
            serde_json::to_vec(&current)? == serde_json::to_vec(&state.settings)?,
            "Preferences changed outside this window. Reopen the calendar before editing.",
        );
        let mut next = state.settings.clone();
        change(&mut next)?;
        cities::save_settings(&next)?;
        Ok(next)
    })();
    match result {
        Ok(next) => {
            let weather_changed = weather_key(&state.settings) != weather_key(&next);
            state.settings = next;
            ui.set_city_status("Saved. Choose Primary to change the forecast city.".into());
            if weather_changed {
                state.generation += 1;
                // Coalesce changes behind the one bounded worker; discard its old generation.
                restore_cached(&mut state);
                launch_weather(&mut state, sender);
            }
            update_settings(ui, &state);
            update_weather(ui, &state);
        }
        Err(error) => report(ui, format!("Could not save preferences: {error:#}")),
    }
}

pub fn start(ui: &MainWindow) -> slint::Timer {
    let (settings, settings_error) = match cities::load_settings() {
        Ok(settings) => (settings.unwrap_or_default(), None),
        Err(error) => {
            let message = format!("Could not read calendar preferences: {error:#}");
            report(ui, &message);
            (cities::Settings::default(), Some(message))
        }
    };
    let shared = Rc::new(RefCell::new(State {
        settings,
        settings_error,
        results: vec![],
        forecast: None,
        generation: 0,
        fetching: false,
        next_refresh: Instant::now(),
        cached: false,
        weather_error: None,
        cache_warning: false,
        weather_minute: Utc::now().timestamp() / 60,
        search_cancelled: Arc::new(AtomicBool::new(false)),
        next_search: Instant::now(),
    }));
    let (sender, receiver) = mpsc::channel();
    update_settings(ui, &shared.borrow());
    restore_cached(&mut shared.borrow_mut());
    launch_weather(&mut shared.borrow_mut(), &sender);
    update_weather(ui, &shared.borrow());

    let weak = ui.as_weak();
    let search_sender = sender.clone();
    let search_state = shared.clone();
    ui.on_search_city(move || {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_city_busy() {
            return;
        }
        let query = ui.get_city_query().trim().to_string();
        if let Err(error) = cities::validate_query(&query) {
            report(&ui, error.to_string());
            return;
        }
        let mut state = search_state.borrow_mut();
        if Instant::now() < state.next_search {
            report(&ui, "Please wait a few seconds before searching again.");
            return;
        }
        state.next_search = Instant::now() + SEARCH_COOLDOWN;
        state.results.clear();
        state.search_cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = state.search_cancelled.clone();
        ui.set_city_busy(true);
        ui.set_city_results(ModelRc::default());
        ui.set_city_status(format!("Searching Open-Meteo for {query}…").into());
        let sender = search_sender.clone();
        std::thread::spawn(move || {
            let result = cities::search(&query, &cancelled)
                .map_err(|e| format!("City search unavailable: {e:#}"));
            let _ = sender.send(Update::Search(query, result));
        });
    });
    let weak = ui.as_weak();
    let state = shared.clone();
    ui.on_cancel_city_search(move || {
        if let Some(ui) = weak.upgrade() {
            let mut state = state.borrow_mut();
            state.search_cancelled.store(true, Ordering::Relaxed);
            state.results.clear();
            ui.set_city_results(ModelRc::default());
            ui.set_city_status(if ui.get_city_busy() {
                "Search cancelled; waiting for the current request to finish.".into()
            } else {
                "Press Search to find this city online.".into()
            });
        }
    });
    let weak = ui.as_weak();
    let state = shared.clone();
    let tx = sender.clone();
    ui.on_add_city(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let city = state.borrow().results.get(index as usize).cloned();
        if let Some(city) = city {
            change_settings(&ui, &state, &tx, |s| s.add_city(city));
        } else {
            report(&ui, "City result is no longer available. Search again.");
        }
    });
    let weak = ui.as_weak();
    let state = shared.clone();
    let tx = sender.clone();
    ui.on_remove_city(move |index| {
        if let Some(ui) = weak.upgrade() {
            change_settings(&ui, &state, &tx, |s| s.remove_city(index as usize));
        }
    });
    let weak = ui.as_weak();
    let state = shared.clone();
    let tx = sender.clone();
    ui.on_select_primary(move |index| {
        if let Some(ui) = weak.upgrade() {
            change_settings(&ui, &state, &tx, |s| s.set_primary(index as usize));
        }
    });
    let weak = ui.as_weak();
    let state = shared.clone();
    let tx = sender.clone();
    ui.on_set_fahrenheit(move |fahrenheit| {
        if let Some(ui) = weak.upgrade() {
            change_settings(&ui, &state, &tx, |s| {
                s.temperature_unit = if fahrenheit {
                    weather::TemperatureUnit::Fahrenheit
                } else {
                    weather::TemperatureUnit::Celsius
                };
                Ok(())
            });
        }
    });

    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut state = shared.borrow_mut();
            let mut weather_changed = false;
            while let Ok(update) = receiver.try_recv() {
                weather_changed = true;
                match update {
                    Update::Search(query, result) => {
                        ui.set_city_busy(false);
                        if state.search_cancelled.load(Ordering::Relaxed)
                            || ui.get_city_query().trim() != query {
                            state.results.clear();
                            ui.set_city_results(ModelRc::default());
                            ui.set_city_status("Search cancelled. Press Search when ready.".into());
                            continue;
                        }
                        match result {
                            Ok(results) => {
                                let description = if results.cities.is_empty() {
                                    format!("No close cities found for {query}. Check the first letters or add a country.")
                                } else if results.suggested {
                                    format!("Suggested matches for {query}. Check city, region and country before adding.")
                                } else {
                                    format!("Results for {query}. Select Add (maximum 3 clocks).")
                                };
                                ui.set_city_status(
                                    if let Some(warning) = results.warning {
                                        eprintln!("[calendar] {warning}");
                                        format!("{description} {warning}")
                                    } else { description
                                    }
                                    .into(),
                                );
                                ui.set_city_results(ModelRc::new(VecModel::from(
                                    city_display_labels(&results.cities),
                                )));
                                state.results = results.cities;
                            }
                            Err(error) => {
                                state.results.clear();
                                report(&ui, error);
                            }
                        }
                    }
                    Update::Forecast(generation, result, cache_error) => {
                        state.fetching = false;
                        if generation != state.generation {
                            continue;
                        }
                        finish_forecast(&mut state, result, cache_error.is_some());
                        if let Some(error) = cache_error {
                            report(&ui, error);
                        }
                    }
                }
            }
            let visible = ui.window().is_visible() && !ui.window().is_minimized();
            ui.set_animate_weather(visible && !ui.get_show_preferences());
            if visible {
                update_clocks(&ui, &state.settings);
                if Instant::now() >= state.next_refresh
                    && !state.fetching
                    && !state.settings.cities.is_empty()
                {
                    launch_weather(&mut state, &sender);
                    weather_changed = true;
                }
                let minute = Utc::now().timestamp() / 60;
                if weather_changed || minute != state.weather_minute {
                    update_weather(&ui, &state);
                    state.weather_minute = minute;
                }
            }
        },
    );
    timer
}

fn clock_item(name: &str, timezone: Option<Tz>, now: DateTime<Utc>) -> ClockItem {
    let (time, date, zone) = if let Some(timezone) = timezone {
        let local = now.with_timezone(&timezone);
        (
            local.format("%H:%M:%S").to_string(),
            local.format("%a, %-d %b").to_string(),
            local.format("%Z").to_string(),
        )
    } else {
        let local = now.with_timezone(&Local);
        (
            local.format("%H:%M:%S").to_string(),
            local.format("%a, %-d %b").to_string(),
            local.format("%Z").to_string(),
        )
    };
    ClockItem {
        name: name.into(),
        time: time.into(),
        date: date.into(),
        zone: zone.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn state() -> State {
        State {
            settings: cities::Settings {
                cities: vec![cities::City {
                    name: "Lafayette".into(),
                    label: "Lafayette, Louisiana, United States".into(),
                    latitude: 30.22409,
                    longitude: -92.01984,
                    timezone: "America/Chicago".into(),
                }],
                ..Default::default()
            },
            settings_error: None,
            results: vec![],
            forecast: None,
            generation: 0,
            fetching: false,
            next_refresh: Instant::now(),
            cached: false,
            weather_error: None,
            cache_warning: false,
            weather_minute: 0,
            search_cancelled: Arc::new(AtomicBool::new(false)),
            next_search: Instant::now(),
        }
    }

    fn forecast(now: DateTime<Utc>, age_minutes: i64) -> weather::Forecast {
        weather::Forecast {
            fetched_at: now - chrono::Duration::minutes(age_minutes),
            utc_offset_seconds: 0,
            stale: age_minutes >= 60,
            days: (0..7)
                .map(|day| weather::ForecastDay {
                    date: now.date_naive() + chrono::Duration::days(day),
                    min: 10.0,
                    max: 20.0,
                    precipitation_probability: 25,
                    weather_code: 3,
                })
                .collect(),
        }
    }

    #[test]
    fn restart_with_fresh_cache_does_not_start_network_or_reset_expiry() {
        let mut state = state();
        let now = Utc::now();
        apply_cached(&mut state, Ok(Some(forecast(now, 20))), now);
        assert_eq!(state.forecast.as_ref().unwrap().days.len(), 7);
        assert!(state.cached);
        assert!(begin_weather_request(&mut state).is_none());
        let remaining = state.next_refresh.saturating_duration_since(Instant::now());
        assert!(remaining <= Duration::from_secs(40 * 60));
        assert!(remaining > Duration::from_secs(40 * 60 - 2));
    }

    #[test]
    fn expired_cache_is_kept_during_refresh_and_after_failure() {
        let mut state = state();
        let now = Utc::now();
        apply_cached(&mut state, Ok(Some(forecast(now, 61))), now);
        let before = state.forecast.clone().unwrap();
        assert!(begin_weather_request(&mut state).is_some());
        assert!(state.fetching);
        assert!(state.forecast.as_ref().unwrap().stale);
        assert!(begin_weather_request(&mut state).is_none(), "single worker");
        state.fetching = false;
        finish_forecast(&mut state, Err("Offline fixture".into()), false);
        assert_eq!(
            state.forecast.as_ref().unwrap().fetched_at,
            before.fetched_at
        );
        assert_eq!(state.forecast.as_ref().unwrap().days.len(), 7);
        assert_eq!(state.weather_error.as_deref(), Some("Offline fixture"));
        assert!(begin_weather_request(&mut state).is_none(), "bounded retry");
        assert!(state.next_refresh.saturating_duration_since(Instant::now()) <= RETRY);
    }

    #[test]
    fn primary_identity_includes_timezone_and_units_and_miss_clears_old_data() {
        let mut state = state();
        let original = weather_key(&state.settings);
        state.settings.temperature_unit = weather::TemperatureUnit::Fahrenheit;
        assert_ne!(original, weather_key(&state.settings));
        state.settings.temperature_unit = weather::TemperatureUnit::Celsius;
        state.settings.cities[0].timezone = "America/New_York".into();
        assert_ne!(original, weather_key(&state.settings));
        state.settings.cities[0].timezone = "America/Chicago".into();
        state.settings.cities[0].latitude = 40.4167;
        assert_ne!(original, weather_key(&state.settings));
        let now = Utc::now();
        apply_cached(&mut state, Ok(Some(forecast(now, 20))), now);
        apply_cached(&mut state, Ok(None), now);
        assert!(state.forecast.is_none());
        assert!(!state.cached);
        assert!(begin_weather_request(&mut state).is_some());
    }

    #[test]
    fn duplicate_place_labels_include_coordinates_for_explicit_selection() {
        let city = state().settings.cities[0].clone();
        let other = cities::City {
            latitude: 40.4167,
            longitude: -86.87529,
            ..city.clone()
        };
        let labels = city_display_labels(&[city.clone(), other]);
        assert!(labels[0].contains("30.2241, -92.0198"));
        assert!(labels[1].contains("40.4167, -86.8753"));
        assert_eq!(city_display_labels(&[city.clone()])[0], city.label);
    }

    #[test]
    fn seconds_dst_and_city_dates_follow_the_instant() {
        let timezone = Some(chrono_tz::America::New_York);
        let before = clock_item("New York", timezone, at("2026-03-08T06:59:59Z"));
        let after = clock_item("New York", timezone, at("2026-03-08T07:00:00Z"));
        assert_eq!(before.time, "01:59:59");
        assert_eq!(after.time, "03:00:00");
        assert_eq!(before.zone, "EST");
        assert_eq!(after.zone, "EDT");
        let fall = clock_item("New York", timezone, at("2026-11-01T06:00:00Z"));
        assert_eq!(fall.time, "01:00:00");
        assert_eq!(fall.zone, "EST");
        let instant = at("2026-10-02T02:30:00Z");
        let tokyo = clock_item("Tokyo", Some(chrono_tz::Asia::Tokyo), instant);
        let la = clock_item(
            "Los Angeles",
            Some(chrono_tz::America::Los_Angeles),
            instant,
        );
        assert_eq!(tokyo.date, "Fri, 2 Oct");
        assert_eq!(la.date, "Thu, 1 Oct");
    }
}
