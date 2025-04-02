use crate::domain::device::Device;
use crate::store::DeviceMap;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, RwLockReadGuard};

#[derive(Default, Debug)]
pub struct Context {
    devices: DeviceMap,
}

impl Context {
    pub fn new(devices: DeviceMap) -> Self {
        Context { devices }
    }

    pub async fn read_devices(&self) -> RwLockReadGuard<'_, HashMap<String, Arc<RwLock<Device>>>> {
        self.devices.read().await
    }
}
