use crate::domain::device::Device;
use crate::flow_engine::property_value::PropertyValue;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug)]
pub enum Command {
    ControlDevice {
        device: Arc<RwLock<Device>>,
        property: Arc<HashMap<String, PropertyValue>>,
    },
}
