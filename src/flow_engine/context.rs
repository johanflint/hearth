use crate::domain::GeoLocation;
use crate::flow_engine::solar_event::{EventTime, SolarEvent, solar_event_time};
use crate::store::StoreSnapshot;
use chrono::{DateTime, Local};

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

    pub fn sunrise(&self) -> EventTime {
        solar_event_time(SolarEvent::Sunrise, self.now, &self.location)
    }

    pub fn sunset(&self) -> EventTime {
        solar_event_time(SolarEvent::Sunset, self.now, &self.location)
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
