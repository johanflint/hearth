use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{DateTimeProperty, EnumProperty, Property, PropertyError, PropertyType};
use crate::hue::controller::CONTROLLER_ID;
use crate::hue::domain::{ButtonGet, DeviceGet};
use std::collections::HashMap;
use thiserror::Error;

pub fn map_remotes(mut buttons: Vec<ButtonGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapRemotesError> {
    // Get a set of devices from the separate buttons
    let mut buttons_by_device: HashMap<String, Vec<&mut ButtonGet>> = HashMap::new();
    for button in &mut buttons {
        buttons_by_device.entry(button.owner.rid.clone()).or_default().push(button);
    }
    // Sort buttons by control id
    for buttons in buttons_by_device.values_mut() {
        buttons.sort_by_key(|b| b.metadata.control_id)
    }

    buttons_by_device.into_iter()
        .map(|(device_id, buttons)| {
            let device_get = device_map
                .remove(&device_id)
                .ok_or_else(|| MapRemotesError::UnknownDevice { device_id: device_id.to_string() })?;

            let mut properties: HashMap<String, Box<dyn Property>> = HashMap::with_capacity(2);

            for button in buttons {
                let report = button.button.button_report.take();
                let last_changed = report.as_ref().map(|report| report.updated);
                let value = report.map(|report| report.event);
                let allowed_values = std::mem::take(&mut button.button.event_values);

                let button_property = Box::new(EnumProperty::new(
                    format!("button{}", button.metadata.control_id),
                    PropertyType::Button,
                    true,
                    Some(button.id.clone()),
                    value,
                    allowed_values,
                ).map_err(|source| MapRemotesError::InvalidButtonProperty {
                    device_id: device_id.clone(),
                    button_id: button.id.clone(),
                    source,
                })?);
                properties.insert(button_property.name().to_owned(), button_property);

                let button_last_changed_property = Box::new(DateTimeProperty::new(
                    format!("button{}LastChanged", button.metadata.control_id),
                    PropertyType::ButtonLastChanged,
                    true,
                    None,
                    last_changed,
                ));
                properties.insert(button_last_changed_property.name().to_owned(), button_last_changed_property);
            }

            Ok(Device {
                id: device_get.id,
                r#type: DeviceType::Remote,
                manufacturer: device_get.product_data.manufacturer_name,
                model_id: device_get.product_data.model_id,
                product_name: device_get.product_data.product_name,
                name: device_get.metadata.name,
                properties,
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
    #[error("invalid button property for button '{button_id}' on device '{device_id}': {source}")]
    InvalidButtonProperty { device_id: String, button_id: String, #[source] source: PropertyError },
}
