use crate::domain::device::Device;
use crate::domain::events::Event;
use crate::domain::property::{BooleanProperty, ColorProperty, NumberProperty};
use crate::property_changed_reducer::reduce_property_changed_event;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use tokio::sync::{RwLock, watch};
use tracing::{debug, info, instrument};

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
                Event::BooleanPropertyChanged { device_id, property_id, value } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id, &property_id, |property: &mut BooleanProperty| {
                        property.set_value(value)
                    })
                    .await
                    .unwrap_or_default();
                }
                Event::NumberPropertyChanged { device_id, property_id, value } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id.clone(), &property_id.clone(), move |property: &mut NumberProperty| {
                        property.set_value(value)
                    })
                    .await
                    .unwrap_or_default();
                }
                Event::ColorPropertyChanged { device_id, property_id, xy, gamut } => {
                    reduce_property_changed_event(&mut self.devices.clone(), &device_id, &property_id, |property: &mut ColorProperty| {
                        property.set_value(xy, gamut)
                    })
                    .await
                    .unwrap_or_default();
                }
            }
        }
    }
}
