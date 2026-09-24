use crate::domain::device::{Device, DeviceType};
use crate::hue::controller::CONTROLLER_ID;
use crate::hue::domain::{ButtonGet, DeviceGet};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

pub fn map_remotes(buttons: Vec<ButtonGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapRemotesError> {
    // Get a set of devices from the separate buttons
    let remotes: HashSet<&str> = buttons.iter().map(|b| b.owner.rid.as_str()).collect();

    remotes.into_iter()
        .map(|device_id| {
            let device_get = device_map
                .remove(device_id)
                .ok_or_else(|| MapRemotesError::UnknownDevice { device_id: device_id.to_string() })?;

            Ok(Device {
                id: device_get.id,
                r#type: DeviceType::Remote,
                manufacturer: device_get.product_data.manufacturer_name,
                model_id: device_get.product_data.model_id,
                product_name: device_get.product_data.product_name,
                name: device_get.metadata.name,
                properties: HashMap::new(),
                external_id: None,
                address: None,
                controller_id: Some(CONTROLLER_ID),
            })
        })
        .collect()
}

#[derive(Error, Debug)]
pub enum MapRemotesError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
}
