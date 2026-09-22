use crate::domain::device::Device;
use crate::domain::events::Event;
use crate::domain::property::{BooleanProperty, ColorProperty, DateTimeProperty, NumberProperty};
use crate::metrics::{Metric, ResultOutcomeLabel};
use crate::property_changed_reducer::reduce_property_changed_event;
use metrics::{counter, gauge};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::watch;
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use tracing::{debug, info, instrument};

pub type DeviceMap = HashMap<String, Arc<Device>>;

#[derive(Default, Clone, Debug)]
pub struct StoreSnapshot {
    pub devices: Arc<DeviceMap>,
}

#[derive(Debug)]
pub struct Store {
    devices: DeviceMap,
    rx: Receiver<Event>,
    notifier_tx: WatchSender<StoreSnapshot>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
}

impl Store {
    pub fn new(rx: Receiver<Event>) -> Self {
        let devices = HashMap::new();
        let snapshot = StoreSnapshot { devices: Arc::new(devices.clone()) };
        let (notifier_tx, notifier_rx) = watch::channel::<StoreSnapshot>(snapshot);

        Store {
            devices,
            rx,
            notifier_tx,
            notifier_rx,
        }
    }

    pub fn notifier(&self) -> WatchReceiver<StoreSnapshot> {
        self.notifier_rx.clone()
    }

    #[instrument(skip(self))]
    pub async fn listen(&mut self) {
        while let Some(event) = self.rx.recv().await {
            debug!("🔵 Received event: {:?}", event);
            match event {
                Event::DiscoveredDevices(discovered_devices) => {
                    let num_devices = discovered_devices.len();
                    debug!("🔵 Registring {} new device(s)...", num_devices);
                    self.devices.extend(discovered_devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))));
                    info!("🔵 Registring {} new device(s)... OK", num_devices);

                    counter!(Metric::StoreDeviceDiscoveries.name()).increment(num_devices as u64);
                    gauge!(Metric::StoreDeviceCount.name()).set(self.devices.len() as f64);
                }
                Event::BooleanPropertyChanged { device_id, property_id, value } => {
                    let result = reduce_property_changed_event(&mut self.devices, &device_id, &property_id, |property: &mut BooleanProperty| {
                        property.set_value(value)
                    });
                    counter!(Metric::StorePropertyChanges.name(),  "property_type" => "boolean", "result" => result.metric_label()).increment(1);
                }
                Event::DateTimePropertyChanged { device_id, property_id, value } => {
                    let result = reduce_property_changed_event(&mut self.devices, &device_id, &property_id, |property: &mut DateTimeProperty| {
                        property.set_value(value)
                    });
                    counter!(Metric::StorePropertyChanges.name(),  "property_type" => "date_time", "result" => result.metric_label()).increment(1);
                }
                Event::ColorPropertyChanged { device_id, property_id, xy, gamut } => {
                    let result = reduce_property_changed_event(&mut self.devices, &device_id, &property_id, |property: &mut ColorProperty| {
                        property.set_value(xy, gamut)
                    });
                    counter!(Metric::StorePropertyChanges.name(),  "property_type" => "color", "result" => result.metric_label()).increment(1);
                }
                Event::NumberPropertyChanged { device_id, property_id, value } => {
                    let result = reduce_property_changed_event(&mut self.devices, &device_id.clone(), &property_id.clone(), move |property: &mut NumberProperty| {
                        property.set_value(value)
                    });
                    counter!(Metric::StorePropertyChanges.name(),  "property_type" => "number", "result" => result.metric_label()).increment(1);
                }
            }

            let snapshot = StoreSnapshot {
                devices: Arc::new(self.devices.clone()),
            };
            self.notifier_tx.send(snapshot).unwrap_or_default();
            info!("🔄 Updated store");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::DeviceBuilder;
    use tokio::sync::mpsc::channel;

    #[tokio::test]
    async fn property_change_is_persisted_in_the_store() {
        let (tx, rx) = channel::<Event>(8);
        let mut store = Store::new(rx);
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
}
