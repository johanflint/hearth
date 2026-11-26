use crate::domain::Weekday;
use crate::domain::Weekday::{Friday, Monday, Saturday, Sunday, Thursday, Tuesday, Wednesday};
use chrono::{Datelike, TimeZone};

pub trait ToWeekday {
    fn to_weekday(&self) -> Weekday;
}

impl<Tz: TimeZone> ToWeekday for chrono::DateTime<Tz> {
    fn to_weekday(&self) -> Weekday {
        match self.weekday() {
            chrono::Weekday::Mon => Monday,
            chrono::Weekday::Tue => Tuesday,
            chrono::Weekday::Wed => Wednesday,
            chrono::Weekday::Thu => Thursday,
            chrono::Weekday::Fri => Friday,
            chrono::Weekday::Sat => Saturday,
            chrono::Weekday::Sun => Sunday,
        }
    }
}
