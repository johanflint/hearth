use crate::domain::device::Device;
use crate::domain::events::Event;
use crate::domain::property::{BooleanProperty, ColorProperty, DateTimeProperty, EnumProperty, NumberProperty, Property, PropertyLocator};
use crate::metrics::{Metric, ResultOutcomeLabel};
use crate::property_changed_reducer::reduce_property_changed_event;
use metrics::{counter, gauge};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, instrument};

pub type DeviceMap = HashMap<String, Arc<Device>>;

#[derive(Default, Clone, Debug)]
pub struct StoreSnapshot {
    pub devices: Arc<DeviceMap>,
}

// Delivered in event order to the reactive flow listener. Unlike the watch
// channel, this queue preserves intermediate snapshots while its receiver
// remains active. Only reactive flows need this, scheduled flows just read
// the current state from the StoreSnapshot.
#[derive(Clone, Debug)]
pub struct ReactiveUpdate {
    pub snapshot: StoreSnapshot,
    pub changed: Option<PropertyChange>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropertyChange {
    pub device_id: String,
    pub property_id: String,
}

#[derive(Debug)]
pub struct Store {
    devices: DeviceMap,
    rx: Receiver<Event>,
    notifier_tx: WatchSender<StoreSnapshot>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
    reactive_tx: Sender<ReactiveUpdate>,
}

/// Capacity of the reactive update channel, see `ReactiveUpdate`.
///
/// Because `Store::listen()` awaits the `reactive_tx.send()`, filling this
/// channel deliberately blocks the store from processing further events
/// (and transitively, any observer from delivering them) rather than dropping
/// reactive updates. If reactive flows are consistently slower than incoming
/// events, raise this value or investigate the slow flow(s).
const REACTIVE_CHANNEL_CAPACITY: usize = 32;

impl Store {
    pub fn new(rx: Receiver<Event>) -> (Self, Receiver<ReactiveUpdate>) {
        let devices = HashMap::new();
        let snapshot = StoreSnapshot { devices: Arc::new(devices.clone()) };
        let (notifier_tx, notifier_rx) = watch::channel::<StoreSnapshot>(snapshot);
        let (reactive_tx, reactive_rx) = mpsc::channel::<ReactiveUpdate>(REACTIVE_CHANNEL_CAPACITY);

        let store = Store {
            devices,
            rx,
            notifier_tx,
            notifier_rx,
            reactive_tx,
        };
        // Returning the receiver makes is impossible to forget to wire it
        (store, reactive_rx)
    }

    pub fn notifier(&self) -> WatchReceiver<StoreSnapshot> {
        self.notifier_rx.clone()
    }

    #[instrument(skip(self))]
    pub async fn listen(&mut self) {
        while let Some(event) = self.rx.recv().await {
            let changed = self.apply_event(event);
            let snapshot = StoreSnapshot {
                devices: Arc::new(self.devices.clone()),
            };
            self.notifier_tx.send(snapshot.clone()).unwrap_or_default();
            if let Err(e) = self.reactive_tx.send(ReactiveUpdate { snapshot, changed }).await {
                error!("⚠️ Unable to deliver reactive update, reactive flows will not run for this change: {}", e);
            }
            info!("🔄 Updated store");
        }
    }

    fn apply_event(&mut self, event: Event) -> Option<PropertyChange> {
        debug!("🔵 Received event: {:?}", event);

        match event {
            Event::DiscoveredDevices(discovered_devices) => {
                let num_devices = discovered_devices.len();
                debug!("🔵 Registring {} new device(s)...", num_devices);
                self.devices.extend(discovered_devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))));
                info!("🔵 Registring {} new device(s)... OK", num_devices);

                counter!(Metric::StoreDeviceDiscoveries.name()).increment(num_devices as u64);
                gauge!(Metric::StoreDeviceCount.name()).set(self.devices.len() as f64);
                None
            }
            Event::BooleanPropertyChanged { device_id, property_id, value } => {
                let result = reduce_property_changed_event(&mut self.devices, &device_id, &property_id, |property: &mut BooleanProperty| {
                    property.set_value(value)
                });
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "boolean", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange { device_id, property_id })
            }
            Event::DateTimePropertyChanged { device_id, property_id, value } => {
                let Some(resolved_property_id) = resolve_property_id::<DateTimeProperty>(&self.devices, &device_id, &property_id) else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve date time property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "date_time", "result" => "failure").increment(1);
                    return None
                };

                let result = reduce_property_changed_event(&mut self.devices, &device_id, &resolved_property_id, |property: &mut DateTimeProperty| {
                    property.set_value(value)
                });
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "date_time", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange { device_id, property_id: resolved_property_id })
            }
            Event::ColorPropertyChanged { device_id, property_id, xy, gamut } => {
                let result = reduce_property_changed_event(&mut self.devices, &device_id, &property_id, |property: &mut ColorProperty| {
                    property.set_value(xy, gamut)
                });
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "color", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange { device_id, property_id })
            }
            Event::NumberPropertyChanged { device_id, property_id, value } => {
                let result = reduce_property_changed_event(&mut self.devices, &device_id.clone(), &property_id.clone(), move |property: &mut NumberProperty| {
                    property.set_value(value)
                });
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "number", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange { device_id, property_id })
            }
            Event::EnumPropertyChanged { device_id, property_id, value } => {
                let Some(resolved_property_id) = resolve_property_id::<EnumProperty>(&self.devices, &device_id, &property_id) else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve enum property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "enum", "result" => "failure").increment(1);
                    return None
                };
                let result = reduce_property_changed_event(&mut self.devices, &device_id.clone(), &resolved_property_id, move |property: &mut EnumProperty| {
                    property.set_value(value)
                });
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "enum", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange { device_id, property_id: resolved_property_id })
            }
        }
    }
}

// Resolves a PropertyLocator (by name, or by the controller-owned external_id of the resource it
// represents) to the property's well-known name, as expected by `reduce_property_changed_event`.
fn resolve_property_id<T: 'static + Property>(devices: &DeviceMap, device_id: &str, locator: &PropertyLocator) -> Option<String> {
    devices.get(device_id)?.resolve_property::<T>(locator).map(|property| property.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::DeviceBuilder;
    use tokio::sync::mpsc::channel;

    #[tokio::test]
    async fn property_change_is_persisted_in_the_store() {
        let (tx, rx) = channel::<Event>(8);
        let (mut store, _reactive_rx) = Store::new(rx);
        let mut notifier = store.notifier();

        tokio::spawn(async move {
            store.listen().await;
        });

        // Discover the device whose "on" property is false
        let device = DeviceBuilder::new("device")
            .with_boolean_property("on", false)
            .build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();
        notifier.changed().await.unwrap();

        // Flip the property to true
        tx.send(Event::BooleanPropertyChanged {
            device_id: "device".to_string(),
            property_id: "on".to_string(),
            value: true,
        }).await.unwrap();
        notifier.changed().await.unwrap();

        let snapshot = notifier.borrow().clone();
        let device = snapshot.devices.get("device").expect("device should exist in snapshot");
        let on_property = device.get_property::<BooleanProperty>("on").expect("property should exist");

        assert_eq!(on_property.value(), true, "expected the store snapshot to reflect the property change");
    }

    #[tokio::test]
    async fn reactive_updates_are_never_merged_even_under_back_to_back_events() {
        // Regression test: every event must produce its own reactive update, in
        // order, with no merging/dropping - even when several events are sent
        // before the store has a chance to process the first one.
        let (tx, rx) = channel::<Event>(8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        let device = DeviceBuilder::new("device")
            .with_boolean_property("on", false)
            .with_boolean_property("motion", false)
            .build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();

        // Sent back-to-back, without awaiting a reactive update in between
        tx.send(Event::BooleanPropertyChanged { device_id: "device".to_string(), property_id: "on".to_string(), value: true }).await.unwrap();
        tx.send(Event::BooleanPropertyChanged { device_id: "device".to_string(), property_id: "motion".to_string(), value: true }).await.unwrap();

        let discovered = reactive_rx.recv().await.expect("expected a reactive update for device discovery");
        assert_eq!(discovered.changed, None);

        let first = reactive_rx.recv().await.expect("expected a reactive update for the first property change");
        assert_eq!(first.changed, Some(PropertyChange { device_id: "device".to_string(), property_id: "on".to_string() }));

        let second = reactive_rx.recv().await.expect("expected a reactive update for the second property change");
        assert_eq!(second.changed, Some(PropertyChange { device_id: "device".to_string(), property_id: "motion".to_string() }));
    }

    #[tokio::test]
    async fn reactive_update_reports_no_change_when_the_event_is_rejected() {
        let (tx, rx) = channel::<Event>(8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        // No device named "unknown" exists, so the reducer rejects this event
        tx.send(Event::BooleanPropertyChanged { device_id: "unknown".to_string(), property_id: "on".to_string(), value: true }).await.unwrap();

        let update = reactive_rx.recv().await.expect("expected a reactive update even for a rejected event");
        assert_eq!(update.changed, None, "a rejected event must not be reported as a property change");
    }

    #[tokio::test]
    async fn reactive_updates_survive_filling_the_channel_beyond_capacity() {
        // Regression test for the channel's core promise: even once the reactive
        // channel (REACTIVE_CHANNEL_CAPACITY slots) fills up completely because the
        // consumer is falling behind, Store::listen just blocks until there's room -
        // no update is ever dropped or merged, and order is preserved.
        let num_events = REACTIVE_CHANNEL_CAPACITY * 2;
        let (tx, rx) = channel::<Event>(num_events + 8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        let device = DeviceBuilder::new("device").with_boolean_property("on", false).build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();

        // Queue well past the reactive channel's capacity before consuming anything
        for i in 0..num_events {
            tx.send(Event::BooleanPropertyChanged { device_id: "device".to_string(), property_id: "on".to_string(), value: i % 2 == 0 }).await.unwrap();
        }

        let discovered = reactive_rx.recv().await.expect("expected the discovery update");
        assert_eq!(discovered.changed, None);

        for i in 0..num_events {
            let update = reactive_rx.recv().await.expect("expected every queued event to eventually arrive, none dropped");
            let value = update.snapshot.devices.get("device").unwrap().get_property::<BooleanProperty>("on").unwrap().value();
            assert_eq!(value, i % 2 == 0, "update #{i} must reflect its own event's value, not a later one merged into it");
        }
    }
}
