mod local_provider;
mod models;
mod provider;
mod weather;
mod dashboard;
mod cities;
mod navigation;
mod window_controls;
mod agenda;
mod event_time;
mod editor;

use chrono::{Datelike, Local, NaiveDate, Timelike};
use local_provider::LocalProvider;
use models::{NewEvent, Recurrence};
use provider::CalendarProvider;
use slint::{ModelRc, SharedString, VecModel};
use smpl_common::theme::{self, ThemePalette, ThemeRole};
use std::cell::RefCell;
use std::rc::Rc;

slint::include_modules!();

// ── Check reminder service without overriding user policy ──────────────────────

fn ensure_alertd() {
    // The no-argument launcher only checks the user service. It cannot override
    // a stopped/masked unit or race the service with an unmanaged daemon.
    std::thread::spawn(|| {
        let self_exe = std::env::current_exe().unwrap_or_default();
        let alertd = self_exe
            .parent()
            .unwrap_or(std::path::Path::new("/usr/local/bin"))
            .join("smpl-calendar-alertd");
        if alertd.exists() {
            match std::process::Command::new(&alertd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.wait();
                }
                Err(error) => eprintln!("smpl-calendar: could not start reminder daemon: {error}"),
            }
        }
    });
}

// ── Window sizes are defined in Slint's `global Sizes` — see ui/main.slint ───
// Rust reads them via ui.global::<Sizes>() after MainWindow::new().

// ── Calendar state ─────────────────────────────────────────────────────────────

struct CalState {
    year:         i32,
    month:        u32,
    selected_day: u32,
    provider:     LocalProvider,
}

impl CalState {
    fn new() -> anyhow::Result<Self> {
        let now = Local::now();
        Ok(Self {
            year:         now.year(),
            month:        now.month(),
            selected_day: now.day(),
            provider:     LocalProvider::open()?,
        })
    }
}

// ── Date helpers ───────────────────────────────────────────────────────────────

fn days_in_month(year: i32, month: u32) -> u32 {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    };
    next.and_then(|d| d.pred_opt()).map(|d| d.day()).unwrap_or(30)
}

const MONTH_NAMES: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December",
];

fn month_name(year: i32, month: u32) -> String {
    format!("{} {}", MONTH_NAMES[(month - 1) as usize], year)
}

/// Map UI dropdown index to alert minutes.
fn alert_minutes_from_idx(idx: i32) -> i32 {
    match idx {
        1 => 15,
        2 => 30,
        3 => 60,
        4 => 480,
        5 => 1440,
        _ => 0,
    }
}

/// Map alert minutes back to UI dropdown index.
fn alert_idx_from_minutes(mins: i32) -> i32 {
    match mins {
        15   => 1,
        30   => 2,
        60   => 3,
        480  => 4,
        1440 => 5,
        _    => 0,
    }
}

/// Format "Wednesday, March 18, 2026"
fn format_day_label(year: i32, month: u32, day: u32) -> String {
    use chrono::Weekday;
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 1).unwrap());
    let weekday = match date.weekday() {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    };
    let today = Local::now().date_naive();
    let prefix = if date == today { "Today" } else { weekday };
    format!(
        "{}, {} {}, {}",
        prefix,
        MONTH_NAMES[(month - 1) as usize],
        day,
        year
    )
}

// ── Build the 42-cell grid for a given month ───────────────────────────────────

fn build_day_cells(
    year: i32,
    month: u32,
    selected_day: u32,
    month_events: &[models::Event],
) -> Vec<DayCell> {
    build_day_cells_at(year, month, selected_day, month_events, Local::now())
}

fn build_day_cells_at(
    year: i32,
    month: u32,
    selected_day: u32,
    month_events: &[models::Event],
    now: chrono::DateTime<Local>,
) -> Vec<DayCell> {
    use chrono::Datelike;
    use std::collections::HashSet;

    let today = now.date_naive();
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let first_col = first.weekday().num_days_from_monday() as i32;
    let mut seen = HashSet::new();
    let unique_events: Vec<_> = month_events.iter()
        .filter(|event| seen.insert((event.id, event.start.timestamp()))).collect();

    (0..42)
        .map(|i| {
            let date = first + chrono::Duration::days(i64::from(i - first_col));
            let row = i / 7;
            let col = i % 7;
            let other_month = date.month() != month || date.year() != year;
            let evs: Vec<_> = unique_events.iter().copied().filter(|event| {
                event.start.date_naive() == date || (event.start.date_naive() < date
                    && (event.end.date_naive() > date || (event.end.date_naive() == date
                        && event.end.time() > chrono::NaiveTime::MIN)))
            }).collect();
            let count = evs.len();
            let first_upcoming = evs.iter().position(|event| event.end > now).unwrap_or(count);
            // Adjacent dates navigate into their month before exposing event actions.
            let events = evs.iter().filter(|_| !other_month).map(|event| GridEvent {
                id: event.id as i32,
                title: event.title.clone().into(),
                time_label: if event.all_day {
                    SharedString::default()
                } else {
                    format!("{:02}:{:02}", event.start.hour(), event.start.minute()).into()
                },
            }).collect::<Vec<_>>();

            DayCell {
                day: date.day() as i32, row, col,
                is_today:    date == today,
                is_selected: !other_month && date.day() == selected_day,
                has_events:  count > 0,
                event_count: count as i32,
                is_other_month: other_month,
                month_offset: if !other_month { 0 } else if date < first { -1 } else { 1 },
                events: ModelRc::from(Rc::new(VecModel::from(events))),
                first_upcoming: if other_month { 0 } else { first_upcoming as i32 },
            }
        })
        .collect()
}

// ── Build the event list for the selected day ─────────────────────────────────

fn build_event_items(
    events: &[models::Event],
    date: NaiveDate,
    now: chrono::DateTime<Local>,
) -> Vec<CalEvent> {
    events
        .iter()
        .map(|ev| {
            let time_label = if ev.all_day {
                "All day".to_string()
            } else {
                format!(
                    "{:02}:{:02} \u{2013} {:02}:{:02}",
                    ev.start.hour(),
                    ev.start.minute(),
                    ev.end.hour(),
                    ev.end.minute()
                )
            };
            let day = date.day() as i32;
            let is_past = ev.end <= now;
            CalEvent {
                id:               ev.id as i32,
                title:            ev.title.clone().into(),
                description:      ev.description.clone().into(),
                time_label:       time_label.into(),
                has_recurrence:   ev.recurrence != Recurrence::None,
                recurrence_label: SharedString::from(ev.recurrence.display()),
                start_hour:       ev.start.hour() as i32,
                start_min:        ev.start.minute() as i32,
                end_hour:         ev.end.hour() as i32,
                end_min:          ev.end.minute() as i32,
                all_day:          ev.all_day,
                recurrence_idx:   ev.recurrence.to_index(),
                is_past,
                is_ongoing:       agenda::is_ongoing(ev, date, now),
                is_now_marker:    false,
                section_label:    SharedString::default(),
                day,
            }
        })
        .collect()
}

// ── Full UI refresh ────────────────────────────────────────────────────────────

fn refresh_ui(ui: &MainWindow, state: &CalState) {
    refresh_ui_for(ui, state, agenda::Update::Refresh);
}

fn refresh_ui_for(ui: &MainWindow, state: &CalState, update: agenda::Update) {
    let year  = state.year;
    let month = state.month;
    let day   = state.selected_day;

    // The six-week grid includes dates from both neighboring months.
    let adjacent = [
        if month == 1 { (year - 1, 12) } else { (year, month - 1) },
        (year, month),
        if month == 12 { (year + 1, 1) } else { (year, month + 1) },
    ];
    let month_events: Vec<_> = adjacent.into_iter()
        .flat_map(|(year, month)| state.provider.events_for_month(year, month)).collect();

    let cells = build_day_cells(year, month, day, &month_events);
    let cell_model = VecModel::from(cells);
    ui.set_day_cells(ModelRc::from(Rc::new(cell_model)));

    // Day events (selected day — used for the slide-in day panel)
    let date = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 1).unwrap());
    let day_events = state.provider.events_for_day(date);
    let now = Local::now();
    let ev_items = build_event_items(&day_events, date, now);
    if !ui.get_is_details() {
        let mut last_all_day = None;
        let compact_items: Vec<_> = agenda::rows(&day_events, date, now)
            .into_iter()
            .map(|row| {
                let mut item = match row {
                    agenda::Row::Event(index) => ev_items[index].clone(),
                    agenda::Row::Now => CalEvent {
                        id: -1,
                        title: format!("Now {:02}:{:02}", now.hour(), now.minute()).into(),
                        is_now_marker: true,
                        ..CalEvent::default()
                    },
                };
                if last_all_day != Some(item.all_day) {
                    item.section_label = if item.all_day { "All day" } else { "Timed events" }.into();
                    last_all_day = Some(item.all_day);
                }
                item
            })
            .collect();
        ui.set_compact_agenda(ModelRc::from(Rc::new(VecModel::from(compact_items))));
    }
    let ev_model = VecModel::from(ev_items);
    ui.set_day_events(ModelRc::from(Rc::new(ev_model)));

    // Labels
    ui.set_month_name(month_name(year, month).into());
    ui.set_year(year);
    ui.set_month(month as i32);
    ui.set_selected_day(day as i32);
    ui.set_selected_date_label(format_day_label(year, month, day).into());

    // Current time for "now" line in the day panel
    let today = now.date_naive();
    let selected_date = NaiveDate::from_ymd_opt(year, month, day)
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 1).unwrap());
    ui.set_is_today_selected(selected_date == today);
    ui.set_current_hour(now.hour() as i32);
    ui.set_current_min(now.minute() as i32);

    if agenda::should_scroll_to_now(
        date,
        today,
        update,
        !ui.get_is_details() && !ui.get_show_preferences(),
    ) {
        let weak = ui.as_weak();
        // Use the laid-out marker row, never a time-to-pixel estimate.
        slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
            if let Some(ui) = weak.upgrade() {
                if !ui.get_is_details()
                    && !ui.get_show_preferences()
                    && ui.window().is_visible()
                    && !ui.window().is_minimized()
                    && ui.get_is_today_selected()
                    && ui.get_year() == date.year()
                    && ui.get_month() == date.month() as i32
                    && ui.get_selected_day() == date.day() as i32
                {
                    ui.set_agenda_scroll_pending(true);
                    ui.set_agenda_scroll_request(ui.get_agenda_scroll_request().wrapping_add(1));
                }
            }
        });
    }
}

// ── Apply smplOS theme ─────────────────────────────────────────────────────────

fn apply_theme(ui: &MainWindow, palette: &ThemePalette) {
    let t = Theme::get(ui);
    t.set_bg(palette.bg);
    t.set_fg(palette.fg);
    t.set_fg_dim(palette.fg_dim);
    t.set_accent(palette.accent);
    t.set_bg_light(palette.bg_light);
    t.set_bg_lighter(palette.bg_lighter);
    t.set_danger(palette.danger);
    t.set_success(palette.success);
    t.set_warning(palette.warning);
    t.set_info(palette.info);
    t.set_opacity(palette.opacity);
}

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() -> Result<(), slint::PlatformError> {
    for arg in std::env::args() {
        if arg == "--font-license" {
            println!("{}", include_str!("../ui/assets/DSEG-LICENSE.txt"));
            return Ok(());
        }
        if arg == "-v" || arg == "--version" {
            println!("smpl-calendar v{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
    }

    // Start in details mode if --details flag given.
    //
    // Two distinct modes with two distinct Wayland app_ids:
    //   • `smpl-calendar`         — compact popup (bottom-right, floating,
    //                               click-outside dismiss, non-resizable).
    //   • `smpl-calendar-details` — standalone Teams-style full calendar
    //                               (centered, floating, resizable, no
    //                               click-outside dismiss). Spawned as a
    //                               separate process when the "Details"
    //                               button is clicked in the compact popup.
    //
    // Splitting the modes across two windows avoids the in-place resize
    // dance (which was fragile on Wayland / Hyprland 0.55 — the Lua parser
    // rejects `hyprctl dispatch resizewindowpixel/movewindowpixel …,class:…`).
    let start_details = std::env::args().any(|a| a == "--details");
    let _compact_instance = if !start_details {
        match navigation::acquire_compact().map_err(|e| slint::PlatformError::Other(format!("{e:#}")))? {
            Some(lock) => Some(lock),
            None => {
                if navigation::focus_compact().map_err(|e| slint::PlatformError::Other(format!("{e:#}")))? {
                    return Ok(());
                }
                return Err(slint::PlatformError::Other("Calendar is already open; switch to its existing window.".into()));
            }
        }
    } else { None };
    let app_id: &'static str = if start_details { "smpl-calendar-details" } else { "smpl-calendar" };
    // Init the backend at the right initial size so Hyprland's windowrule
    // for the correct app_id can size and place the window on first map.
    let (init_w, init_h) = if start_details { (1100.0, 700.0) } else { (364.0, 650.0) };
    smpl_common::init(app_id, init_w, init_h)?;

    // Start the reminder daemon (stays alive for the session)
    ensure_alertd();

    let ui   = MainWindow::new()?;

    // Read the single source of truth from Slint
    let compact_w = ui.global::<Sizes>().get_compact_w();
    let compact_h = ui.global::<Sizes>().get_compact_h();
    let details_w = ui.global::<Sizes>().get_details_w();
    let details_h = ui.global::<Sizes>().get_details_h();

    // Select the layout before applying the initial native size.
    if start_details {
        ui.set_is_details(true);
        ui.set_is_standalone(true);
        ui.window().set_size(slint::LogicalSize::new(details_w, details_h));
    } else {
        ui.window().set_size(slint::LogicalSize::new(compact_w, compact_h));
    }
    let state = Rc::new(RefCell::new(
        CalState::new().expect("failed to open calendar database"),
    ));

    let ui_weak = ui.as_weak();
    let _theme_timer = theme::watch(ThemeRole::Popup, move |palette| {
        if let Some(ui) = ui_weak.upgrade() {
            apply_theme(&ui, palette);
        }
    });

    refresh_ui_for(&ui, &state.borrow(), agenda::Update::Open);
    let _dashboard = if !start_details { Some(dashboard::start(&ui)) } else { None };
    let calendar_tick = slint::Timer::default();
    let weak = ui.as_weak();
    let tick_state = state.clone();
    calendar_tick.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(60), move || {
        if let Some(ui) = weak.upgrade() {
            if ui.window().is_visible() && !ui.window().is_minimized()
                && !ui.get_show_preferences() && !ui.get_show_form() {
                refresh_ui(&ui, &tick_state.borrow());
            }
        }
    });

    if !start_details {
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(100), move || {
            use i_slint_backend_winit::WinitWindowAccessor;
            if let Some(ui) = weak.upgrade() {
                let available = ui.window().with_winit_window(|window| {
                    window.current_monitor().map(|monitor| {
                        monitor.size().height as f64 / monitor.scale_factor() - 64.0
                    })
                }).flatten();
                if let Some(height) = available {
                    ui.window().set_size(slint::LogicalSize::new(
                        compact_w, compact_h.min(height.max(320.0) as f32),
                    ));
                }
            }
        });
    }

    // Launch time is captured up-front so both the on_close startup guard
    // (below) and the on_open_details spawn timing can reason about it.
    // The 1500ms close-guard still applies to both compact and standalone
    // details windows: Hyprland's map animations produce phantom
    // wl_pointer.enter → click events on some setups.
    let launch_time = std::time::Instant::now();

    // ── prev-month ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        ui.on_prev_month(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if s.month == 1 {
                s.month = 12;
                s.year -= 1;
            } else {
                s.month -= 1;
            }
            // Clamp selected day to valid range
            s.selected_day = s.selected_day.min(days_in_month(s.year, s.month));
            drop(s);
            refresh_ui_for(&ui, &state.borrow(), agenda::Update::Selection);
        });
    }

    // ── next-month ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        ui.on_next_month(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            if s.month == 12 {
                s.month = 1;
                s.year += 1;
            } else {
                s.month += 1;
            }
            s.selected_day = s.selected_day.min(days_in_month(s.year, s.month));
            drop(s);
            refresh_ui_for(&ui, &state.borrow(), agenda::Update::Selection);
        });
    }

    // ── select-day ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        ui.on_select_day(move |day| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            s.selected_day = day as u32;
            drop(s);
            refresh_ui_for(&ui, &state.borrow(), agenda::Update::Selection);
            // Auto-open day panel in details mode when a day is clicked
            if ui.get_is_details() {
                ui.set_show_day_panel(true);
            }
        });
    }

    // ── open-details ──────────────────────────────────────────────────────────
    // Compact-mode only. Spawns a second instance of ourselves with `--details`
    // — that instance uses the `smpl-calendar-details` app_id and gets its own
    // Hyprland windowrule (float + center + resizable). The compact popup is
    // closed so we don't leave two calendar windows on screen. If we're already
    // the details instance, this callback is a no-op (button isn't visible).
    {
        let ui_weak = ui.as_weak();
        ui.on_open_details(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            if ui.get_is_details() { return; }

            // Spawn the standalone details window. Use the current-exe path
            // rather than PATH lookup so a locally-installed dev build launches
            // the same binary the user actually clicked from.
            let exe = std::env::current_exe()
                .unwrap_or_else(|_| std::path::PathBuf::from("smpl-calendar"));
            match std::process::Command::new(&exe).arg("--details").spawn() {
                Ok(_) => {
                    // Dismiss the compact popup after a short delay so the mouse-up
                    // from the Details click has drained and the new window has
                    // begun mapping. Using Timer keeps the caller (Slint tick)
                    // clean; the actual exit happens on the main loop.
                    slint::Timer::single_shot(std::time::Duration::from_millis(120), || {
                        std::process::exit(0);
                    });
                }
                Err(e) => {
                    eprintln!("[calendar] failed to spawn '{}' --details: {e}", exe.display());
                }
            }
        });
    }

    navigation::wire_back(&ui);

    // ── navigate-to-month-day (click other-month cell) ────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        ui.on_navigate_to_month_day(move |offset, day| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            match offset {
                -1 => {
                    if s.month == 1 { s.month = 12; s.year -= 1; }
                    else { s.month -= 1; }
                }
                1 => {
                    if s.month == 12 { s.month = 1; s.year += 1; }
                    else { s.month += 1; }
                }
                _ => {}
            }
            s.selected_day = (day as u32).min(days_in_month(s.year, s.month));
            drop(s);
            refresh_ui_for(&ui, &state.borrow(), agenda::Update::Selection);
            if ui.get_is_details() {
                ui.set_show_day_panel(true);
            }
        });
    }

    let editor = editor::wire(&ui);

    // ── new-event ─────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        let editor = editor.clone();
        ui.on_new_event(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let s = state.borrow();
            let (s_year, s_month, s_day) = (s.year, s.month, s.selected_day);
            let date_str = format!("{:04}-{:02}-{:02}", s_year, s_month, s_day);
            drop(s);
            ui.set_form_editing_id(-1);
            ui.set_form_title("".into());
            ui.set_form_desc("".into());
            ui.set_form_start_h(9);
            ui.set_form_start_m(0);
            ui.set_form_end_h(10);
            ui.set_form_end_m(0);
            ui.set_form_all_day(false);
            ui.set_form_rec_idx(0);
            ui.set_form_alert_idx(0);
            ui.set_form_date_str(date_str.into());
            ui.set_form_date_invalid(false);
            // Mirror into the split ints the DatePicker widget is bound to.
            // form-date-str is kept in sync as a safety net for the invalid
            // banner and any legacy read path.
            ui.set_form_year(s_year);
            ui.set_form_month(s_month as i32);
            ui.set_form_day(s_day as i32);
            ui.set_form_end_year(s_year);
            ui.set_form_end_month(s_month as i32);
            ui.set_form_end_day(s_day as i32);
            editor.begin(&ui, None);
            ui.set_show_form(true);
        });
    }

    // ── edit-event ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        let editor = editor.clone();
        ui.on_edit_event(move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let s = state.borrow();
            let date = NaiveDate::from_ymd_opt(s.year, s.month, s.selected_day)
                .unwrap_or_else(|| NaiveDate::from_ymd_opt(s.year, s.month, 1).unwrap());
            let events = s.provider.events_for_day(date);
            if let Some(ev) = events.iter().find(|e| e.id == id as i64) {
                ui.set_form_editing_id(id);
                ui.set_form_title(ev.title.clone().into());
                ui.set_form_desc(ev.description.clone().into());
                ui.set_form_start_h(ev.start.hour() as i32);
                ui.set_form_start_m(ev.start.minute() as i32);
                ui.set_form_end_h(ev.end.hour() as i32);
                ui.set_form_end_m(ev.end.minute() as i32);
                ui.set_form_all_day(ev.all_day);
                ui.set_form_rec_idx(ev.recurrence.to_index());
                ui.set_form_alert_idx(alert_idx_from_minutes(ev.alert_minutes));
                ui.set_form_date_str(ev.start.format("%Y-%m-%d").to_string().into());
                ui.set_form_date_invalid(false);
                // Split-int mirror for the DatePicker widget.
                ui.set_form_year(ev.start.year());
                ui.set_form_month(ev.start.month() as i32);
                ui.set_form_day(ev.start.day() as i32);
                ui.set_form_end_year(ev.end.year());
                ui.set_form_end_month(ev.end.month() as i32);
                ui.set_form_end_day(ev.end.day() as i32);
                editor.begin(&ui, Some((ev.start, ev.end)));
                ui.set_show_form(true);
            }
        });
    }

    // ── delete-event ──────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        ui.on_delete_event(move |id| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = state.borrow_mut();
            let _ = s.provider.delete_event(id as i64);
            drop(s);
            refresh_ui(&ui, &state.borrow());
        });
    }

    // ── save-event ────────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        let state   = state.clone();
        let editor = editor.clone();
        ui.on_save_event(move || {
            let Some(ui) = ui_weak.upgrade() else { return };

            let title = ui.get_form_title().to_string();
            if title.trim().is_empty() {
                ui.set_form_error("Enter an event title.".into());
                return;
            }

            let all_day   = ui.get_form_all_day();
            let rec_idx   = ui.get_form_rec_idx();
            let alert_idx = ui.get_form_alert_idx();
            let editing_id = ui.get_form_editing_id();

            let (start, end) = match editor.interval(&ui) {
                Ok(interval) => interval,
                Err(error) => {
                    ui.set_form_valid(false);
                    ui.set_form_error(format!("{error:#}").into());
                    return;
                }
            };
            let date = start.date_naive();

            let mut s = state.borrow_mut();

            let recurrence = Recurrence::from_index(rec_idx);

            let result = if editing_id < 0 {
                // Create
                let new_ev = NewEvent {
                    title,
                    description:    ui.get_form_desc().to_string(),
                    start,
                    end,
                    all_day,
                    recurrence,
                    recurrence_end: None,
                    color:          None,
                    alert_minutes:  alert_minutes_from_idx(alert_idx),
                };
                s.provider.create_event(new_ev).map(|_| ())
            } else {
                // Update — look up on the currently-viewed date (original day before
                // the user might have changed the date field) then apply new values.
                let orig_date = NaiveDate::from_ymd_opt(s.year, s.month, s.selected_day)
                    .unwrap_or_else(|| NaiveDate::from_ymd_opt(s.year, s.month, 1).unwrap());
                let events = s.provider.events_for_day(orig_date);
                if let Some(mut ev) = events.into_iter().find(|e| e.id == editing_id as i64) {
                    ev.title       = title;
                    ev.description = ui.get_form_desc().to_string();
                    ev.start       = start;
                    ev.end         = end;
                    ev.all_day     = all_day;
                    ev.recurrence  = recurrence;
                    ev.alert_minutes = alert_minutes_from_idx(alert_idx);
                    s.provider.update_event(ev)
                } else {
                    Err(anyhow::anyhow!("This event no longer exists. Close the editor and reopen it."))
                }
            };
            if let Err(error) = result {
                eprintln!("[calendar] Could not save event: {error:#}");
                ui.set_form_error(format!("Could not save event: {error:#}").into());
                return;
            }

            // Navigate to the saved date so the user sees their event
            let update = if (s.year, s.month, s.selected_day) != (date.year(), date.month(), date.day()) {
                agenda::Update::Selection
            } else {
                agenda::Update::Refresh
            };
            s.year         = date.year();
            s.month        = date.month();
            s.selected_day = date.day();
            drop(s);
            ui.set_show_form(false);
            refresh_ui_for(&ui, &state.borrow(), update);
        });
    }

    // ── cancel-form ───────────────────────────────────────────────────────────
    {
        let ui_weak = ui.as_weak();
        ui.on_cancel_form(move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            ui.set_show_form(false);
        });
    }

    // ── close ─────────────────────────────────────────────────────────────────
    // Startup guard: ignore close() for the first 1.5s after launch.
    // Hyprland’s move windowrule slides the window under the cursor on map,
    // causing phantom wl_pointer.enter events that Slint interprets as clicks.
    ui.on_close({
        move || {
            if launch_time.elapsed() < std::time::Duration::from_millis(1500) {
                return;
            }
            std::process::exit(0);
        }
    });

    // ── window drag (works on X11 + Wayland, no float rule needed) ───────────────
    // Receives (dx, dy) deltas from Slint's moved event
    // (self.mouse-x - self.pressed-x) in logical pixels.
    // set_position() is standard Slint API, no Wayland serial or compositor
    // drag protocol needed — identical to how the settings app does it.
    {
        let ui_weak = ui.as_weak();
        ui.on_move_window(move |dx, dy| {
            let Some(ui) = ui_weak.upgrade() else { return };
            let scale = ui.window().scale_factor();
            let pos   = ui.window().position();
            ui.window().set_position(slint::WindowPosition::Physical(
                slint::PhysicalPosition::new(
                    pos.x + (dx * scale) as i32,
                    pos.y + (dy * scale) as i32,
                ),
            ));
        });
    }

    let _window_controls = window_controls::wire(&ui);

    ui.run()
}

#[cfg(test)]
mod calendar_grid_tests {
    use super::*;
    use chrono::TimeZone;
    use slint::Model;

    #[test]
    fn event_dates_include_neighbors_and_overlap_without_day_number_aliases() {
        let start = Local.with_ymd_and_hms(2026, 9, 30, 23, 50, 0).single().unwrap();
        let overnight = models::Event {
            id: 1, title: "Fixture".into(), description: String::new(),
            start, end: start + chrono::Duration::minutes(20), all_day: false,
            recurrence: Recurrence::None, recurrence_end: None, color: None, alert_minutes: 0,
        };
        let mut occurrence = overnight.clone();
        occurrence.id = 2;
        occurrence.start = Local.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).single().unwrap();
        occurrence.end = occurrence.start + chrono::Duration::hours(1);
        occurrence.recurrence = Recurrence::Monthly;
        let mut next_occurrence = occurrence.clone();
        next_occurrence.start = Local.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).single().unwrap();
        next_occurrence.end = next_occurrence.start + chrono::Duration::hours(1);
        let mut ends_at_midnight = overnight.clone();
        ends_at_midnight.id = 3;
        ends_at_midnight.start = Local.with_ymd_and_hms(2026, 10, 31, 23, 0, 0).single().unwrap();
        ends_at_midnight.end = ends_at_midnight.start + chrono::Duration::hours(1);
        let events = vec![overnight.clone(), overnight, occurrence, next_occurrence, ends_at_midnight];
        let now = Local.with_ymd_and_hms(2026, 10, 1, 0, 5, 0).single().unwrap();
        let cells = build_day_cells_at(2026, 10, 1, &events, now);
        let find = |offset, day| cells.iter().find(|cell| cell.month_offset == offset && cell.day == day).unwrap();
        assert_eq!(find(-1, 30).event_count, 1);
        assert!(find(-1, 30).has_events);
        assert_eq!(find(0, 1).event_count, 2);
        assert!(find(0, 1).is_selected && find(0, 1).is_today && find(0, 1).has_events);
        assert!(!find(0, 30).has_events);
        assert_eq!(find(0, 31).event_count, 1);
        assert_eq!(find(1, 1).event_count, 1);
        assert!(find(1, 1).has_events);
    }

    #[test]
    fn details_keep_all_real_events_and_the_upcoming_anchor() {
        let now = Local.with_ymd_and_hms(2026, 10, 2, 12, 15, 0).single().unwrap();
        let events: Vec<_> = (0..10).map(|index| {
            let start = Local.with_ymd_and_hms(2026, 10, 2, 8 + index, 0, 0).single().unwrap();
            models::Event {
                id: 100 + i64::from(index),
                title: format!("Real event {index}"),
                description: String::new(),
                start,
                end: start + chrono::Duration::minutes(30),
                all_day: false,
                recurrence: Recurrence::None,
                recurrence_end: None,
                color: None,
                alert_minutes: 0,
            }
        }).collect();
        let cells = build_day_cells_at(2026, 10, 2, &events, now);
        let cell = cells.iter().find(|cell| !cell.is_other_month && cell.day == 2).unwrap();
        assert!(cell.is_today);
        assert_eq!(cell.event_count, 10);
        assert_eq!(cell.events.row_count(), 10);
        assert_eq!(cell.first_upcoming, 4);
        assert_eq!(cell.events.row_data(9).unwrap().id, 109);
        assert_eq!(cell.events.row_data(9).unwrap().title, "Real event 9");
        assert_eq!(cell.events.row_data(9).unwrap().time_label, "17:00");

        let late = now + chrono::Duration::hours(6);
        let cells = build_day_cells_at(2026, 10, 2, &events, late);
        let cell = cells.iter().find(|cell| !cell.is_other_month && cell.day == 2).unwrap();
        assert_eq!(cell.first_upcoming, 10);
        assert_eq!(cell.events.row_count(), 10);
        assert!(cells.iter().filter(|cell| cell.is_other_month)
            .all(|cell| cell.events.row_count() == 0));
    }

    #[test]
    fn every_month_has_all_days_and_six_complete_rows() {
        for year in [1900, 2000, 2026, 2028, 2100] {
            for month in 1..=12 {
                let cells = build_day_cells(year, month, 1, &[]);
                assert_eq!(cells.len(), 42);
                let days: Vec<_> = cells.iter().filter(|d| !d.is_other_month)
                    .map(|d| d.day).collect();
                assert_eq!(days, (1..=days_in_month(year, month) as i32).collect::<Vec<_>>());
                for (i, cell) in cells.iter().enumerate() {
                    assert_eq!((cell.row, cell.col), (i as i32 / 7, i as i32 % 7));
                }
            }
        }
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2028, 2), 29);
        assert_eq!(days_in_month(2100, 2), 28);
    }
}
