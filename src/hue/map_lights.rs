use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, Gamut, NumberProperty, Property, PropertyType, Unit};
use crate::hue::domain::{DeviceGet, LightGet};
use std::collections::HashMap;
use thiserror::Error;

pub fn map_lights(lights: Vec<LightGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapLightsError> {
    lights
        .into_iter()
        .map(|light| {
            let device_get = device_map
                .remove(&light.owner.rid)
                .ok_or_else(|| MapLightsError::UnknownDevice { device_id: light.owner.rid })?;

            let mut properties = HashMap::with_capacity(3);

            let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
                "on".to_string(),
                PropertyType::On,
                false,
                Some(light.id.clone()),
                light.on.on,
            ));
            properties.insert(on_property.name().to_owned(), on_property);

            light.dimming.map(|dimming| {
                let brightness_property: Box<dyn Property> = Box::new(
                    NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                        .external_id(light.id.clone())
                        .unit(Unit::Percentage)
                        .float(dimming.brightness, dimming.min_dim_level.or(Some(0.0)), Some(100.0))
                        .build(),
                );
                properties.insert(brightness_property.name().to_owned(), brightness_property);
            });

            light.color.map(|color| {
                let brightness_property: Box<dyn Property> = Box::new(ColorProperty::new(
                    "color".to_string(),
                    PropertyType::Color,
                    false,
                    Some(light.id.clone()),
                    CartesianCoordinate::new(color.xy.x, color.xy.y),
                    color.gamut.map(|g| {
                        Gamut::new(
                            CartesianCoordinate::new(g.red.x, g.red.y),
                            CartesianCoordinate::new(g.green.x, g.green.y),
                            CartesianCoordinate::new(g.blue.x, g.blue.y),
                        )
                    }),
                ));
                properties.insert(brightness_property.name().to_owned(), brightness_property);
            });

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
