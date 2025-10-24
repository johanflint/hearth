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
    pub fn builder() -> ContextBuilder {
        ContextBuilder::default()
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

#[derive(Default, Debug)]
pub struct ContextBuilder {
    snapshot: Option<StoreSnapshot>,
    now: Option<DateTime<Local>>,
    location: Option<GeoLocation>,
}

impl ContextBuilder {
    pub fn snapshot(mut self, snapshot: StoreSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    pub fn now(mut self, now: DateTime<Local>) -> Self {
        self.now = Some(now);
        self
    }

    pub fn location(mut self, location: GeoLocation) -> Self {
        self.location = Some(location);
        self
    }

    pub fn build(self) -> Context {
        Context {
            snapshot: self.snapshot.unwrap_or_default(),
            now: self.now.unwrap_or_else(Local::now),
            location: self.location.unwrap_or_default(),
        }
    }
}
