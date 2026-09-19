use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, DateTimeProperty, NumberProperty, Property, PropertyType, Unit};
use crate::hue::controller::CONTROLLER_ID;
use crate::hue::domain::{DeviceGet, MotionGet};
use std::collections::HashMap;
use thiserror::Error;

pub fn map_motion_sensors(sensors: Vec<MotionGet>, device_map: &mut HashMap<String, DeviceGet>) -> Result<Vec<Device>, MapMotionSensorsError> {
    sensors
        .into_iter()
        .map(|sensor| {
            let device_get = device_map
                .remove(&sensor.owner.rid)
                .ok_or_else(|| MapMotionSensorsError::UnknownDevice { device_id: sensor.owner.rid })?;

            let mut properties: HashMap<String, Box<dyn Property>> = HashMap::with_capacity(4);

            let enabled_property = Box::new(BooleanProperty::new("enabled".to_string(), PropertyType::Enabled, false, Some(sensor.id.clone()), sensor.enabled));
            properties.insert(enabled_property.name().to_owned(), enabled_property);

            let motion_property = Box::new(BooleanProperty::new("motion".to_string(), PropertyType::Motion, true, None, sensor.motion.motion_report.as_ref().is_some_and(|r| r.motion)));
            properties.insert(motion_property.name().to_owned(), motion_property);

            if let Some(motion_report) = sensor.motion.motion_report {
                let motion_changed_property = Box::new(DateTimeProperty::new("motionLastChanged".to_string(), PropertyType::MotionLastChanged, true, Some(sensor.id.clone()), motion_report.changed));
                properties.insert(motion_changed_property.name().to_owned(), motion_changed_property);
            }

            // Depends on the PIR sensitivity setting, so it's a hardware capability and without units
            let sensitivity_property = Box::new(
                NumberProperty::builder("sensitivity".to_string(), PropertyType::MotionSensitivity, false)
                    .external_id(sensor.id.clone())
                    .unit(Unit::None)
                    .positive_int(sensor.sensitivity.sensitivity, Some(0), Some(sensor.sensitivity.sensitivity_max))
                    .build(),
            );
            properties.insert(sensitivity_property.name().to_owned(), sensitivity_property);

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
}
