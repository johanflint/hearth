use crate::api::ApiState;
use crate::api::test_support::create_state_with_snapshot;
use crate::domain::Device;
use crate::store::StoreSnapshot;
use crate::test_support::DeviceBuilder;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) fn device(id: &str, name: &str) -> Device {
    Device { name: name.to_string(), ..DeviceBuilder::new(id).build() }
}

/// The API state with the devices in the store
pub(super) fn state_with_devices(devices: Vec<Device>) -> ApiState {
    let devices: HashMap<_, _> = devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect();
    create_state_with_snapshot(StoreSnapshot { devices: Arc::new(devices), ..Default::default() })
}
