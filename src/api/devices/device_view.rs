use crate::api::devices::device_type_view::DeviceTypeView;
use crate::domain::Device;
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeviceView {
    pub id: String,
    pub r#type: DeviceTypeView,
    pub manufacturer: String,
    pub model_id: String,
    pub product_name: String,
    pub name: String,
    pub claimable: bool,
}

impl From<Arc<Device>> for DeviceView {
    fn from(device: Arc<Device>) -> Self {
        Self {
            id: device.id.clone(),
            r#type: (&device.r#type).into(),
            manufacturer: device.manufacturer.clone(),
            model_id: device.model_id.clone(),
            product_name: device.product_name.clone(),
            name: device.name.clone(),
            claimable: device.claimable,
        }
    }
}
