use crate::{event_time, MainWindow};
use anyhow::{Context, Result};
use chrono::{DateTime, Datelike, Local, NaiveDate, Timelike};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};
#[path = "time_jump.rs"]
mod time_jump;

type Interval = (DateTime<Local>, DateTime<Local>);

#[derive(Clone, Default)]
pub struct Editor {
    known: Rc<RefCell<Option<Interval>>>,
}

fn date(year: i32, month: i32, day: i32) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).context("Choose a valid date")
}

impl Editor {
    pub fn begin(&self, ui: &MainWindow, original: Option<Interval>) {
        *self.known.borrow_mut() = original;
        self.validate(ui, false);
    }

    pub fn interval(&self, ui: &MainWindow) -> Result<Interval> {
        let start_date = date(ui.get_form_year(), ui.get_form_month(), ui.get_form_day())?;
        let end_date = if ui.get_form_all_day() {
            start_date
        } else {
            date(
                ui.get_form_end_year(),
                ui.get_form_end_month(),
                ui.get_form_end_day(),
            )?
        };
        event_time::resolve_interval(
            start_date,
            ui.get_form_start_h(),
            ui.get_form_start_m(),
            end_date,
            ui.get_form_end_h(),
            ui.get_form_end_m(),
            ui.get_form_all_day(),
            *self.known.borrow(),
        )
    }

    fn validate(&self, ui: &MainWindow, adjust: bool) {
        let result = (|| -> Result<()> {
            if adjust && !ui.get_form_all_day() {
                let known = *self.known.borrow();
                let start = event_time::resolve_local(
                    date(ui.get_form_year(), ui.get_form_month(), ui.get_form_day())?,
                    ui.get_form_start_h(),
                    ui.get_form_start_m(),
                    known.map(|v| v.0),
                )?;
                let end = date(
                    ui.get_form_end_year(),
                    ui.get_form_end_month(),
                    ui.get_form_end_day(),
                )
                .and_then(|date| {
                    event_time::resolve_local(
                        date,
                        ui.get_form_end_h(),
                        ui.get_form_end_m(),
                        known.map(|v| v.1),
                    )
                })?;
                let end = event_time::auto_end(start, Some(end))?;
                ui.set_form_end_year(end.year());
                ui.set_form_end_month(end.month() as i32);
                ui.set_form_end_day(end.day() as i32);
                ui.set_form_end_h(end.hour() as i32);
                ui.set_form_end_m(end.minute() as i32);
                *self.known.borrow_mut() = Some((start, end));
            }
            self.interval(ui)?;
            Ok(())
        })();
        ui.set_form_valid(result.is_ok());
        ui.set_form_error(match result {
            Ok(()) => "".into(),
            Err(error) => format!("{error:#}").into(),
        });
    }
}

pub fn wire(ui: &MainWindow) -> Editor {
    ui.global::<crate::TimeJump>()
        .on_is_input(|text| time_jump::is_input(&text));
    ui.global::<crate::TimeJump>()
        .on_edit(|query, input, backspace| {
            let result = time_jump::edit(&query, &input, backspace);
            crate::TimeJumpMatch {
                query: result.query.into(),
                index: result.index,
                message: result.message.into(),
            }
        });
    let editor = Editor::default();
    let state = editor.clone();
    let weak = ui.as_weak();
    ui.on_form_interval_changed(move |adjust| {
        if let Some(ui) = weak.upgrade() {
            state.validate(&ui, adjust);
        }
    });
    editor
}
