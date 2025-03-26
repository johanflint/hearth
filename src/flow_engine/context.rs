use crate::domain::device::Device;
use crate::store::DeviceMap;
use std::collections::HashMap;
use tokio::sync::RwLockReadGuard;

#[derive(Default, Debug)]
pub struct Context {
    devices: DeviceMap,
}

impl Context {
    pub fn new(devices: DeviceMap) -> Self {
        Context { devices }
    }

    pub async fn read_devices(&self) -> RwLockReadGuard<'_, HashMap<String, Device>> {
        self.devices.read().await
    }
}
