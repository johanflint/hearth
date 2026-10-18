use crate::domain::GeoLocation;
use crate::flow_engine::solar_event::{EventTime, SolarEvent, solar_event_time};
use crate::preset_registry::PresetRegistry;
use crate::store::{PropertyChange, StoreSnapshot};
use chrono::{DateTime, Local};
use std::sync::Arc;

#[derive(Default, Debug)]
pub struct Context {
    snapshot: StoreSnapshot,
    changed: Option<PropertyChange>,
    preset_registry: Arc<PresetRegistry>,
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

    /// The property that changed in the event this context was built for.
    /// Only ever `Some` for reactive flows, always `None` for scheduled
    /// flows, which have no single triggering event.
    pub fn changed(&self) -> Option<&PropertyChange> {
        self.changed.as_ref()
    }

    pub fn preset_registry(&self) -> Arc<PresetRegistry> {
        self.preset_registry.clone()
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
    changed: Option<PropertyChange>,
    preset_registry: Option<Arc<PresetRegistry>>,
    now: Option<DateTime<Local>>,
    location: Option<GeoLocation>,
}

impl ContextBuilder {
    pub fn snapshot(mut self, snapshot: StoreSnapshot) -> Self {
        self.snapshot = Some(snapshot);
        self
    }

    pub fn changed(mut self, changed: Option<PropertyChange>) -> Self {
        self.changed = changed;
        self
    }

    pub fn preset_registry(mut self, preset_registry: Arc<PresetRegistry>) -> Self {
        self.preset_registry = Some(preset_registry);
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
            changed: self.changed,
            preset_registry: self.preset_registry.unwrap_or_default(),
            now: self.now.unwrap_or_else(Local::now),
            location: self.location.unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Preset, Value, VersionedPreset};

    fn registry_with_preset() -> Arc<PresetRegistry> {
        let preset = VersionedPreset {
            preset: Arc::new(Preset { id: "warm".to_string(), name: "Warm".to_string(), value: Value::Boolean(true) }),
            revision: 0,
        };
        Arc::new(PresetRegistry::new(vec![preset]))
    }

    #[test]
    fn builder_uses_the_given_preset_registry() {
        let registry = registry_with_preset();

        let context = Context::builder().preset_registry(Arc::clone(&registry)).build();

        assert!(Arc::ptr_eq(&context.preset_registry(), &registry));
        assert!(context.preset_registry().by_id("warm").is_some());
    }

    #[test]
    fn builder_defaults_to_an_empty_preset_registry() {
        let context = Context::builder().build();

        assert!(context.preset_registry().by_id("warm").is_none());
    }

    #[test]
    fn default_context_has_an_empty_preset_registry() {
        assert!(Context::default().preset_registry().by_id("warm").is_none());
    }
}
