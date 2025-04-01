use crate::flow_engine::property_value::PropertyValue;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub enum Command {
    ControlDevice {
        device_id: String,
        property: Arc<HashMap<String, PropertyValue>>,
    },
}
