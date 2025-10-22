use crate::domain::GeoLocation;
use crate::store::StoreSnapshot;
use chrono::{DateTime, Local};
use sunrise::{Coordinates, SolarDay, SolarEvent};

#[derive(Default, Debug)]
pub struct Context {
    snapshot: StoreSnapshot,
    now: DateTime<Local>,
    location: GeoLocation,
}

impl Context {
    pub fn new(snapshot: StoreSnapshot, location: GeoLocation) -> Self {
        Context {
            snapshot,
            now: Local::now(),
            location,
        }
    }

    pub fn new_with_now(snapshot: StoreSnapshot, now: DateTime<Local>, location: GeoLocation) -> Self {
        Context { snapshot, now, location }
    }

    pub fn snapshot(&self) -> &StoreSnapshot {
        &self.snapshot
    }

    pub fn now(&self) -> DateTime<Local> {
        self.now
    }

    pub fn sunrise(&self) -> DateTime<Local> {
        self.solar_event(SolarEvent::Sunrise)
    }

    pub fn sunset(&self) -> DateTime<Local> {
        self.solar_event(SolarEvent::Sunset)
    }

    fn solar_event(&self, event: SolarEvent) -> DateTime<Local> {
        let date = self.now.date_naive();

        // The expect is fine as GeoLocation is validated during deserialization
        let coordinates = Coordinates::new(self.location.latitude, self.location.longitude).expect("valid coordinates");
        SolarDay::new(coordinates, date)
            .with_altitude(self.location.altitude)
            .event_time(event)
            .with_timezone(&Local)
    }
}
