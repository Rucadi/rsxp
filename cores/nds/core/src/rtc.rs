// SPDX-License-Identifier: GPL-3.0-or-later
use chrono::{Datelike, Local, TimeZone, Timelike};
use dust_core::rtc::{Backend, Date, Time};
#[derive(Default)]
pub struct Clock {
    offset: i64,
}
impl Backend for Clock {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn get_time(&mut self) -> Time {
        self.get_date_time().1
    }
    fn get_date_time(&mut self) -> (Date, Time) {
        let now = Local::now() + chrono::Duration::seconds(self.offset);
        (
            Date {
                years_since_2000: (now.year() - 2000).rem_euclid(100) as u8,
                month: now.month() as u8,
                day: now.day() as u8,
                days_from_sunday: now.weekday().num_days_from_sunday() as u8,
            },
            Time {
                hour: now.hour() as u8,
                minute: now.minute() as u8,
                second: now.second() as u8,
            },
        )
    }
    fn set_date_time(&mut self, (date, time): (Date, Time)) {
        if let Some(value) = Local
            .with_ymd_and_hms(
                2000 + date.years_since_2000 as i32,
                date.month as u32,
                date.day as u32,
                time.hour as u32,
                time.minute as u32,
                time.second as u32,
            )
            .single()
        {
            self.offset = value.timestamp() - Local::now().timestamp();
        }
    }
}
