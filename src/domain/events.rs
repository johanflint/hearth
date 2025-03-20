use crate::domain::device::Device;
use crate::domain::property::Number;

#[derive(Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
    BooleanPropertyChanged { device_id: String, property_id: String, value: bool },
    NumberPropertyChanged { device_id: String, property_id: String, value: Number },
}
