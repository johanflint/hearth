use crate::domain::property::PropertyType;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum PropertyTypeView {
    BatteryLevel,
    BatteryState,
    Brightness,
    Button,
    ButtonLastChanged,
    Color,
    ColorTemperature,
    ContactSensor,
    ContactSensorLastChanged,
    Connectivity,
    Enabled,
    Illuminance,
    IlluminanceLastChanged,
    Motion,
    MotionLastChanged,
    MotionSensitivity,
    On,
    Presence,
    Tampered,
    TamperedLastChanged,
    Temperature,
    TemperatureLastChanged,
}

impl From<&PropertyType> for PropertyTypeView {
    fn from(value: &PropertyType) -> Self {
        match value {
            PropertyType::BatteryState => Self::BatteryState,
            PropertyType::BatteryLevel => Self::BatteryLevel,
            PropertyType::Brightness => Self::Brightness,
            PropertyType::Button => Self::Button,
            PropertyType::ButtonLastChanged => Self::ButtonLastChanged,
            PropertyType::Color => Self::Color,
            PropertyType::ColorTemperature => Self::ColorTemperature,
            PropertyType::ContactSensor => Self::ContactSensor,
            PropertyType::ContactSensorLastChanged => Self::ContactSensorLastChanged,
            PropertyType::Connectivity => Self::Connectivity,
            PropertyType::Enabled => Self::Enabled,
            PropertyType::Illuminance => Self::Illuminance,
            PropertyType::IlluminanceLastChanged => Self::IlluminanceLastChanged,
            PropertyType::Motion => Self::Motion,
            PropertyType::MotionLastChanged => Self::MotionLastChanged,
            PropertyType::MotionSensitivity => Self::MotionSensitivity,
            PropertyType::On => Self::On,
            PropertyType::Presence => Self::Presence,
            PropertyType::Tampered => Self::Tampered,
            PropertyType::TamperedLastChanged => Self::TamperedLastChanged,
            PropertyType::Temperature => Self::Temperature,
            PropertyType::TemperatureLastChanged => Self::TemperatureLastChanged,
        }
    }
}
