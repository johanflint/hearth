use crate::domain::GeoLocation;
use chrono::{DateTime, Local};
use sunrise::{Coordinates, SolarDay};

/// Depth (degrees below horizon) used to disambiguate polar day from polar
/// night when SolarDay::event_time returns None for Sunrise/Sunset.
/// Must exceed Earth's axial tilt (~23.44°) — the maximum possible solar
/// declination — so it reliably separates both cases at any latitude short
/// of the literal geographic pole.
const POLAR_DISCRIMINATOR_DEPRESSION_DEG: f64 = 24.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolarEvent {
    Sunrise,
    Sunset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventTime {
    At(DateTime<Local>),
    PolarDay,
    PolarNight,
}

pub fn solar_event_time(event: SolarEvent, now: DateTime<Local>, location: &GeoLocation) -> EventTime {
    let date = now.date_naive();
    let coordinates = Coordinates::new(location.latitude, location.longitude).expect("valid coordinates");
    let solar_day = SolarDay::new(coordinates, date)
        .with_altitude(location.altitude);

    solar_day.event_time(to_solar_event(event))
        .map(|time| EventTime::At(time.with_timezone(&Local)))
        .unwrap_or_else(|| {
            let deep_horizon_crossing_exists = solar_day.event_time(sunrise::SolarEvent::Elevation { elevation: POLAR_DISCRIMINATOR_DEPRESSION_DEG.to_radians(), morning: true }).is_some();
            if deep_horizon_crossing_exists {
                EventTime::PolarNight
            } else {
                EventTime::PolarDay
            }
        })
}

fn to_solar_event(event: SolarEvent) -> sunrise::SolarEvent {
    match event {
        SolarEvent::Sunrise => sunrise::SolarEvent::Sunrise,
        SolarEvent::Sunset => sunrise::SolarEvent::Sunset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone, Utc};
    use pretty_assertions::assert_eq;

    #[test]
    fn solar_event_time_returns_sunrise_on_a_normal_day() {
        let now = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunrise, now, &location());
        assert_eq!(result, EventTime::At(Utc.with_ymd_and_hms(2000, 8, 4, 4, 10, 14).unwrap().with_timezone(&Local)));
    }

    #[test]
    fn solar_event_time_returns_sunset_on_a_normal_day() {
        let now = Local.with_ymd_and_hms(2000, 8, 4, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunset, now, &location());
        assert_eq!(result, EventTime::At(Utc.with_ymd_and_hms(2000, 8, 4, 19, 26, 57).unwrap().with_timezone(&Local)));
    }

    #[test]
    fn solar_event_time_returns_polar_night_for_sunrise_when_the_sun_never_rises() {
        let now = Local.with_ymd_and_hms(2000, 2, 13, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunrise, now, &polar_location());
        assert_eq!(result, EventTime::PolarNight);
    }

    #[test]
    fn solar_event_time_returns_polar_night_for_sunset_when_the_sun_never_rises() {
        let now = Local.with_ymd_and_hms(2000, 2, 13, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunset, now, &polar_location());
        assert_eq!(result, EventTime::PolarNight);
    }

    #[test]
    fn solar_event_time_returns_polar_day_for_sunrise_when_the_sun_never_sets() {
        let now = Local.with_ymd_and_hms(2000, 6, 21, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunrise, now, &polar_location());
        assert_eq!(result, EventTime::PolarDay);
    }

    #[test]
    fn solar_event_time_returns_polar_day_for_sunset_when_the_sun_never_sets() {
        let now = Local.with_ymd_and_hms(2000, 6, 21, 12, 0, 0).unwrap();
        let result = solar_event_time(SolarEvent::Sunset, now, &polar_location());
        assert_eq!(result, EventTime::PolarDay);
    }

    fn location() -> GeoLocation {
        GeoLocation { latitude: 51.8615899, longitude: 4.3580323, altitude: 0.0 }
    }

    fn polar_location() -> GeoLocation {
        // Longyearbyen, Svalbard: polar night from mid-October to mid-February, polar day mid-April to late-August.
        GeoLocation { latitude: 78.2232, longitude: 15.6267, altitude: 0.0 }
    }
}
