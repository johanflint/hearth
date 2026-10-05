use crate::domain::device::Device;
use crate::flow_engine::property_value::PropertyCommand;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub enum Command {
    ControlDevice {
        device: Arc<Device>,
        property: Arc<HashMap<String, PropertyCommand>>,
    },
}
