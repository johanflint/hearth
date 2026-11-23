use crate::domain::DeviceType;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum DeviceTypeView {
    Light,
    MotionSensor,
    Remote,
    ContactSensor,
    Switch,
    PresenceSensor,
}

impl From<&DeviceType> for DeviceTypeView {
    fn from(t: &DeviceType) -> Self {
        match t {
            DeviceType::Light => Self::Light,
            DeviceType::MotionSensor => Self::MotionSensor,
            DeviceType::Remote => Self::Remote,
            DeviceType::ContactSensor => Self::ContactSensor,
            DeviceType::Switch => Self::Switch,
            DeviceType::PresenceSensor => Self::PresenceSensor,
        }
    }
}
