use crate::domain::device::Device;
use crate::domain::events::Event;
use std::collections::HashMap;
use tokio::sync::mpsc::Receiver;
use tracing::{debug, info, instrument};

#[derive(Debug)]
pub struct Store {
    devices: HashMap<String, Device>,
    rx: Receiver<Event>,
}

impl Store {
    pub fn new(rx: Receiver<Event>) -> Self {
        Store { devices: HashMap::new(), rx }
    }

    #[instrument(skip(self))]
    pub async fn listen(&mut self) {
        while let Some(event) = self.rx.recv().await {
            debug!("🔵 Received event: {:?}", event);
            match event {
                Event::DiscoveredDevices(discovered_devices) => {
                    let num_devices = discovered_devices.len();
                    debug!("🔵 Registring {} device(s)...", num_devices);
                    self.devices
                        .extend(discovered_devices.into_iter().map(|device| (device.id.clone(), device)));
                    info!("🔵 Registring {} device(s)... OK", num_devices);
                }
            }
        }
    }
}
