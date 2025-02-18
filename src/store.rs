use crate::domain::device::Device;
use crate::domain::events::Event;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument};

#[derive(Debug)]
pub struct Store {
    devices: Arc<RwLock<HashMap<String, Device>>>,
    rx: Receiver<Event>,
}

impl Store {
    pub fn new(rx: Receiver<Event>) -> Self {
        Store {
            devices: Arc::new(RwLock::new(HashMap::new())),
            rx,
        }
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
                }
            }
        }
    }
}
