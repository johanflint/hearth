use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, NumberProperty, Property, PropertyType, Unit};
use crate::extensions::unsigned_ints_ext::MirekConversions;
use crate::hue::controller::CONTROLLER_ID;
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

            let mut properties = HashMap::with_capacity(4);

            let on_property: Box<dyn Property> = Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, false, Some(light.id.clone()), light.on.on));
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

            light.color_temperature.map(|temperature| {
                let mirek_value = temperature.mirek.unwrap_or(temperature.mirek_schema.mirek_minimum);

                let color_temperature_property: Box<dyn Property> = Box::new(
                    NumberProperty::builder("colorTemperature".to_string(), PropertyType::ColorTemperature, false)
                        .external_id(light.id.clone())
                        .unit(Unit::Kelvin)
                        .positive_int(
                            mirek_value.max(temperature.mirek_schema.mirek_minimum).mirek_to_kelvin(),
                            Some(temperature.mirek_schema.mirek_maximum.mirek_to_kelvin()), // Not a bug: mirek is inverse to K
                            Some(temperature.mirek_schema.mirek_minimum.mirek_to_kelvin()),
                        )
                        .build(),
                );

                properties.insert(color_temperature_property.name().to_owned(), color_temperature_property);
            });

            light.color.map(|color| {
                let brightness_property: Box<dyn Property> = Box::new(ColorProperty::new(
                    "color".to_string(),
                    PropertyType::Color,
                    false,
                    Some(light.id.clone()),
                    CartesianCoordinate::new(color.xy.x, color.xy.y),
                    color.gamut.map(|mut g| g.take_gamut()),
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
                controller_id: Some(CONTROLLER_ID),
            })
        })
        .collect()
}

#[derive(Error, Debug)]
pub enum MapLightsError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::Gamut;
    use crate::hue::domain::{HueResponse, Metadata, ProductData};
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn test_map_lights() -> Result<(), MapLightsError> {
        let json = include_str!("../../tests/resources/hue_light_response.json");

        let response = serde_json::from_str::<HueResponse<LightGet>>(json).unwrap();

        let device_get = DeviceGet {
            id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(),
            product_data: ProductData {
                model_id: "LCT007".to_string(),
                manufacturer_name: "Signify Netherlands B.V.".to_string(),
                product_name: "Hue color lamp".to_string(),
                product_archetype: "sultan_bulb".to_string(),
                certified: false,
                software_version: "67.116.3".to_string(),
                hardware_platform_type: Some("100b-104".to_string()),
            },
            metadata: Metadata {
                name: "Lamp".to_string(),
                archetype: "sultan_bulb".to_string(),
            },
        };

        let mut device_map = HashMap::from([(device_get.id.clone(), device_get)]);
        let result = map_lights(response.data, &mut device_map)?;

        let on_property: Box<dyn Property> = Box::new(BooleanProperty::new(
            "on".to_string(),
            PropertyType::On,
            false,
            Some("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string()),
            false,
        ));

        let brightness_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Percentage)
                .float(58.89, Some(2.0), Some(100.0))
                .build(),
        );

        let color_temperature_property: Box<dyn Property> = Box::new(
            NumberProperty::builder("colorTemperature".to_string(), PropertyType::ColorTemperature, false)
                .external_id("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string())
                .unit(Unit::Kelvin)
                .positive_int(6535, Some(2000), Some(6535))
                .build(),
        );

        let color_property: Box<dyn Property> = Box::new(ColorProperty::new(
            "color".to_string(),
            PropertyType::Color,
            false,
            Some("43e4f3a7-8b35-4b0c-a2ba-e6ca8f4c099b".to_string()),
            CartesianCoordinate::new(0.4851, 0.4331),
            Some(Gamut::new(
                CartesianCoordinate::new(0.675, 0.322),
                CartesianCoordinate::new(0.409, 0.518),
                CartesianCoordinate::new(0.167, 0.04),
            )),
        ));

        assert_eq!(
            result[0],
            Device {
                id: "ab917a9a-a7d5-4853-9518-75909236a182".to_string(),
                r#type: DeviceType::Light,
                manufacturer: "Signify Netherlands B.V.".to_string(),
                model_id: "LCT007".to_string(),
                product_name: "Hue color lamp".to_string(),
                name: "Lamp".to_string(),
                properties: HashMap::from([
                    (on_property.name().to_string(), on_property),
                    (brightness_property.name().to_string(), brightness_property),
                    (color_temperature_property.name().to_string(), color_temperature_property),
                    (color_property.name().to_string(), color_property),
                ]),
                external_id: None,
                address: None,
                controller_id: Some("hue"),
            }
        );

        Ok(())
    }
}
