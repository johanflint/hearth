use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, DateTimeProperty, NumberProperty, Property, PropertyType, Unit};
use crate::hue::controller::CONTROLLER_ID;
use crate::hue::domain::{DeviceGet, LightLevelGet, MotionGet};
use std::collections::HashMap;
use thiserror::Error;

pub fn map_motion_sensors(sensors: Vec<MotionGet>, light_levels: Vec<LightLevelGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapMotionSensorsError> {
    let mut light_level_map: HashMap<String, LightLevelGet> = light_levels.into_iter().map(|light_level| (light_level.owner.rid.clone(), light_level)).collect();
    sensors
        .into_iter()
        .map(|sensor| {
            let device_get = device_map
                .remove(&sensor.owner.rid)
                .ok_or_else(|| MapMotionSensorsError::UnknownDevice { device_id: sensor.owner.rid.clone() })?;
            let light_level = light_level_map
                .remove(&sensor.owner.rid)
                .ok_or_else(|| MapMotionSensorsError::MissingLightLevel { device_id: sensor.owner.rid.clone() })?;

            let mut properties: HashMap<String, Box<dyn Property>> = HashMap::with_capacity(6);

            let enabled_property = Box::new(BooleanProperty::new("enabled".to_string(), PropertyType::Enabled, false, Some(sensor.id.clone()), sensor.enabled));
            properties.insert(enabled_property.name().to_owned(), enabled_property);

            let motion_property = Box::new(BooleanProperty::new("motion".to_string(), PropertyType::Motion, true, Some(sensor.id.clone()), sensor.motion.motion_report.as_ref().is_some_and(|r| r.motion)));
            properties.insert(motion_property.name().to_owned(), motion_property);

            let motion_changed_property = Box::new(DateTimeProperty::new(
                "motionLastChanged".to_string(),
                PropertyType::MotionLastChanged,
                true,
                None,
                sensor.motion.motion_report.map(|r| r.changed),
            ));
            properties.insert(motion_changed_property.name().to_owned(), motion_changed_property);

            // Depends on the PIR sensitivity setting, so it's a hardware capability and without units
            let sensitivity_property = Box::new(
                NumberProperty::builder("sensitivity".to_string(), PropertyType::MotionSensitivity, false)
                    .external_id(sensor.id.clone())
                    .unit(Unit::None)
                    .positive_int(Some(sensor.sensitivity.sensitivity), Some(0), Some(sensor.sensitivity.sensitivity_max))
                    .build(),
            );
            properties.insert(sensitivity_property.name().to_owned(), sensitivity_property);

            let (light_level_lx, light_level_changed) = match light_level.light.light_level_report {
                Some(report) if report.light_level == 0 => (0.0, Some(report.changed)),
                Some(report) => (10f64.powf((report.light_level as f64 - 1.0) / 10_000.0), Some(report.changed)),
                None => (0.0, None),
            };

            let light_level_property = Box::new(
                NumberProperty::builder("illuminance".to_string(), PropertyType::Illuminance, true)
                    .unit(Unit::Lux)
                    .float(Some(light_level_lx), Some(0.0), None)
                    .build(),
            );
            properties.insert(light_level_property.name().to_owned(), light_level_property);

            let light_level_changed_property = Box::new(DateTimeProperty::new(
                "illuminanceLastChanged".to_string(),
                PropertyType::IlluminanceLastChanged,
                true,
                None,
                light_level_changed,
            ));
            properties.insert(light_level_changed_property.name().to_owned(), light_level_changed_property);

            Ok(Device {
                id: device_get.id,
                r#type: DeviceType::MotionSensor,
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
pub enum MapMotionSensorsError {
    #[error("unknown device '{device_id}'")]
    UnknownDevice { device_id: String },
    #[error("missing light level for device '{device_id}'")]
    MissingLightLevel { device_id: String },
}
