use crate::domain::device::Device;
use crate::domain::events::Event;
use crate::domain::property::{BooleanProperty, ColorProperty, NumberProperty};
use crate::property_changed_reducer::reduce_property_changed_event;
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
                }
                Event::BooleanPropertyChanged { device_id, property_id, value } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id, &property_id, |property: &mut BooleanProperty| {
                        property.set_value(value)
                    })
                    .unwrap_or_default();
                }
                Event::NumberPropertyChanged { device_id, property_id, value } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id.clone(), &property_id.clone(), move |property: &mut NumberProperty| {
                        property.set_value(value)
                    })
                    .unwrap_or_default();
                }
                Event::ColorPropertyChanged { device_id, property_id, xy, gamut } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id, &property_id, |property: &mut ColorProperty| {
                        property.set_value(xy, gamut)
                    })
                    .unwrap_or_default();
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
