use crate::domain::device::Device;
use crate::domain::events::Event;
use crate::domain::property::{BooleanProperty, NumberProperty, Property, PropertyError};
use std::any::type_name;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use tokio::sync::{RwLock, watch};
use tracing::{debug, info, instrument, warn};

pub type DeviceMap = Arc<RwLock<HashMap<String, Device>>>;

#[derive(Debug)]
pub struct Store {
    devices: Arc<RwLock<HashMap<String, Device>>>,
    rx: Receiver<Event>,
    notifier_tx: WatchSender<DeviceMap>,
    notifier_rx: WatchReceiver<DeviceMap>,
}

impl Store {
    pub fn new(rx: Receiver<Event>) -> Self {
        let devices = Arc::new(RwLock::new(HashMap::new()));
        let (notifier_tx, notifier_rx) = watch::channel::<DeviceMap>(devices.clone());

        Store {
            devices,
            rx,
            notifier_tx,
            notifier_rx,
        }
    }

    pub fn notifier(&self) -> WatchReceiver<DeviceMap> {
        self.notifier_rx.clone()
    }

    #[instrument(skip(self))]
    pub async fn listen(&mut self) {
        while let Some(event) = self.rx.recv().await {
            debug!("🔵 Received event: {:?}", event);
            match event {
                Event::DiscoveredDevices(discovered_devices) => {
                    let num_devices = discovered_devices.len();
                    debug!("🔵 Registring {} device(s)...", num_devices);
                    let mut write_guard = self.devices.write().await;

                    write_guard.extend(discovered_devices.into_iter().map(|device| (device.id.clone(), device)));
                    info!("🔵 Registring {} device(s)... OK", num_devices);

                    self.notifier_tx.send(self.devices.clone()).unwrap_or_default();
                }
                Event::BooleanPropertyChanged {
                    device_id,
                    property_id,
                    value,
                } => {
                    self.reduce_property_changed_event(&device_id, &property_id, |property: &mut BooleanProperty| {
                        return property.set_value(value).map(|_| ());
                    })
                    .await;
                }
                Event::NumberPropertyChanged {
                    device_id,
                    property_id,
                    value,
                } => {
                    self.reduce_property_changed_event(&device_id.clone(), &property_id.clone(), move |property: &mut NumberProperty| {
                        return property.set_value(value).map(|_| ());
                    })
                    .await;
                }
            }
        }
    }

    #[inline(always)]
    async fn reduce_property_changed_event<F, T>(&mut self, device_id: &str, property_id: &str, set_value: F)
    where
        F: FnOnce(&mut T) -> Result<(), PropertyError>,
        T: Property + 'static,
    {
        let mut write_guard = self.devices.write().await;

        let Some(device) = write_guard.get_mut(device_id) else {
            #[rustfmt::skip]
            warn!(device_id, "⚠️ Received property changed event for unknown device '{}'", device_id);
            return;
        };

        let Some(property) = device.properties.get_mut(property_id) else {
            #[rustfmt::skip]
            warn!(device_id = device.id,"⚠️ Unknown property '{}' for device '{}'", property_id, device.name);
            return;
        };

        let previous_value = property.value_string();
        let Some(downcast_property) = property.as_any_mut().downcast_mut::<T>() else {
            #[rustfmt::skip]
            warn!(device_id, "⚠️ Expected {} property for property '{}'", type_name::<T>(), &property_id);
            return;
        };

        if let Err(err) = set_value(downcast_property) {
            #[rustfmt::skip]
            warn!(device_id = device.id, "⚠️ Could not set value for property '{}': {}", property_id, err);
            return;
        }

        info!(
            device_id = device.id,
            "🟢 Updated device '{}', set '{}' to '{}', was '{}'",
            device.name,
            property.name(),
            property.value_string(),
            previous_value
        );
    }
}
