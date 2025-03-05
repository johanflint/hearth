use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, Property, PropertyType};
use crate::hue::device_response::DeviceGet;
use crate::hue::light_response::LightGet;
use std::collections::HashMap;
use thiserror::Error;

pub fn map_lights(lights: Vec<LightGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapLightsError> {
    lights
        .into_iter()
        .map(|light| {
            let device_get = device_map
                .remove(&light.owner.rid)
                .ok_or_else(|| MapLightsError::UnknownDevice { device_id: light.owner.rid })?;

            let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
                "on".to_string(),
                PropertyType::On,
                false,
                Some(light.id),
                light.on.on,
            ));

            let mut properties = HashMap::with_capacity(1);
            properties.insert(on_property.name().to_owned(), on_property);

            Ok(Device {
                id: device_get.id,
                r#type: DeviceType::Light,
                manufacturer: device_get.product_data.manufacturer_name,
                model_id: device_get.product_data.model_id,
                product_name: device_get.product_data.product_name,
                name: device_get.metadata.name,
                properties,
                external_id: None,
                address: None,
            })
        })
        .collect()
}

#[derive(Error, Debug)]
pub enum MapLightsError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
}
