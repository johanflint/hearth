use crate::domain::GeoLocation;
use crate::flow_engine::WeekdayCondition;
use crate::flow_engine::expression::ToWeekday;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use std::ops::Add;
use std::str::FromStr;
use sunrise::{Coordinates, SolarDay};

#[derive(Clone, PartialEq, Debug)]
pub enum Schedule {
    Cron(String),
    Sunrise { when: WeekdayCondition, offset: i64 },
    Sunset { when: WeekdayCondition, offset: i64 },
}

#[derive(Debug)]
enum SolarEvent {
    Sunrise,
    Sunset,
}

impl Schedule {
    /// Returns an iterator which will return each `DateTime` that matches the schedule starting at the specified date and time.
    pub fn after<Z>(&self, from: DateTime<Z>, location: GeoLocation) -> ScheduleIterator<Z>
    where
        Z: TimeZone,
    {
        match self {
            Schedule::Cron(expression) => {
                let schedule = cron::Schedule::from_str(expression).unwrap_or_else(|_| panic!("invalid cron expression '{}'", expression));
                let iterator = schedule.after_owned(from);
                ScheduleIterator::Cron(iterator)
            }
            Schedule::Sunrise { when, offset } => ScheduleIterator::SunEvent(SunEventIterator::new(location, from, when.clone(), *offset, SolarEvent::Sunrise)),
            Schedule::Sunset { when, offset } => ScheduleIterator::SunEvent(SunEventIterator::new(location, from, when.clone(), *offset, SolarEvent::Sunset)),
        }
    }

    /// Returns an iterator which will return each `DateTime` that matches the schedule starting at the current time.
    pub fn upcoming<Z>(&self, timezone: Z, location: GeoLocation) -> ScheduleIterator<Z>
    where
        Z: TimeZone,
    {
        let now = Utc::now().with_timezone(&timezone);
        self.after(now, location)
    }
}

pub enum ScheduleIterator<Z>
where
    Z: TimeZone,
{
    Cron(cron::OwnedScheduleIterator<Z>),
    SunEvent(SunEventIterator<Z>),
}

impl<Z> Iterator for ScheduleIterator<Z>
where
    Z: TimeZone,
{
    type Item = DateTime<Z>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ScheduleIterator::Cron(iterator) => iterator.next(),
            ScheduleIterator::SunEvent(iter) => iter.next(),
        }
    }
}

pub struct SunEventIterator<Z>
where
    Z: TimeZone,
{
    coordinates: Coordinates,
    altitude: f64,
    current: DateTime<Z>,
    when: WeekdayCondition,
    offset: i64,
    solar_event: SolarEvent,
}

impl<Z> SunEventIterator<Z>
where
    Z: TimeZone,
{
    fn new(location: GeoLocation, current: DateTime<Z>, when: WeekdayCondition, offset: i64, solar_event: SolarEvent) -> Self {
        let coordinates = Coordinates::new(location.latitude, location.longitude).expect("valid coordinates");
        Self {
            coordinates,
            altitude: location.altitude,
            current,
            when,
            offset,
            solar_event,
        }
    }

    fn event_time(&self, date: NaiveDate) -> DateTime<Z>
    where
        Z: TimeZone,
    {
        let event = match self.solar_event {
            SolarEvent::Sunrise => sunrise::SolarEvent::Sunrise,
            SolarEvent::Sunset => sunrise::SolarEvent::Sunset,
        };

        SolarDay::new(self.coordinates, date)
            .with_altitude(self.altitude)
            .event_time(event)
            .with_timezone(&self.current.timezone())
            .add(Duration::seconds(self.offset))
    }
}

impl<Z> Iterator for SunEventIterator<Z>
where
    Z: TimeZone,
{
    type Item = DateTime<Z>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let date = self.current.clone();
            self.current += Duration::days(1);
            if !self.when.included_days().contains(&date.to_weekday()) {
                continue;
            }
            let event_time = self.event_time(date.date_naive());
            return Some(event_time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Weekday::*;
    use chrono::{Datelike, NaiveDate};
    use pretty_assertions::assert_eq;

    #[test]
    fn test_schedule_with_cron() {
        let schedule = Schedule::Cron("0 0 20 * * *".to_string());
        let now = Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let upcoming = schedule.after(now, location()).take(5).collect::<Vec<_>>();

        // Notice that the nanoseconds are passed, that's done to ensure that the occurence is exactly at 20:00:00Z on the hour without subsecond drift
        let first = NaiveDate::from_ymd_opt(2000, 8, 4).and_then(|dt| dt.and_hms_nano_opt(20, 0, 0, 0)).unwrap().and_utc();
        assert_eq!(upcoming[0], first);
        assert_eq!(upcoming[1], first.with_day(5).unwrap());
        assert_eq!(upcoming[2], first.with_day(6).unwrap());
        assert_eq!(upcoming[3], first.with_day(7).unwrap());
        assert_eq!(upcoming[4], first.with_day(8).unwrap());
    }

    #[test]
    fn test_schedule_with_sunrise_event() {
        let schedule = Schedule::Sunrise {
            when: WeekdayCondition::Range { start: Wednesday, end: Friday },
            offset: 0,
        };

        let now = Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap(); // A Friday
        let upcoming = schedule.after(now, location()).take(5).collect::<Vec<_>>();

        assert_eq!(upcoming[0], Utc.with_ymd_and_hms(2000, 08, 04, 04, 10, 14).unwrap());
        assert_eq!(upcoming[1], Utc.with_ymd_and_hms(2000, 08, 09, 04, 18, 11).unwrap());
        assert_eq!(upcoming[2], Utc.with_ymd_and_hms(2000, 08, 10, 04, 19, 48).unwrap());
        assert_eq!(upcoming[3], Utc.with_ymd_and_hms(2000, 08, 11, 04, 21, 24).unwrap());
        assert_eq!(upcoming[4], Utc.with_ymd_and_hms(2000, 08, 16, 04, 29, 30).unwrap());

        assert_eq!(upcoming[0].to_weekday(), Friday);
        assert_eq!(upcoming[1].to_weekday(), Wednesday);
        assert_eq!(upcoming[2].to_weekday(), Thursday);
        assert_eq!(upcoming[3].to_weekday(), Friday);
        assert_eq!(upcoming[4].to_weekday(), Wednesday);
    }

    #[test]
    fn test_schedule_with_sunset_event() {
        let schedule = Schedule::Sunset {
            when: WeekdayCondition::Range { start: Wednesday, end: Friday },
            offset: 0,
        };

        let now = Utc.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap(); // A Friday
        let upcoming = schedule.after(now, location()).take(5).collect::<Vec<_>>();

        assert_eq!(upcoming[0], Utc.with_ymd_and_hms(2000, 08, 04, 19, 26, 57).unwrap());
        assert_eq!(upcoming[1], Utc.with_ymd_and_hms(2000, 08, 09, 19, 17, 55).unwrap());
        assert_eq!(upcoming[2], Utc.with_ymd_and_hms(2000, 08, 10, 19, 16, 02).unwrap());
        assert_eq!(upcoming[3], Utc.with_ymd_and_hms(2000, 08, 11, 19, 14, 08).unwrap());
        assert_eq!(upcoming[4], Utc.with_ymd_and_hms(2000, 08, 16, 19, 04, 17).unwrap());

        assert_eq!(upcoming[0].to_weekday(), Friday);
        assert_eq!(upcoming[1].to_weekday(), Wednesday);
        assert_eq!(upcoming[2].to_weekday(), Thursday);
        assert_eq!(upcoming[3].to_weekday(), Friday);
        assert_eq!(upcoming[4].to_weekday(), Wednesday);
    }

    fn location() -> GeoLocation {
        GeoLocation {
            latitude: 51.8615899,
            longitude: 4.3580323,
            altitude: 0.0,
        }
    }
}
