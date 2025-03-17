use crate::domain::device::Device;

#[derive(Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
    BooleanPropertyChanged { device_id: String, property_id: String, value: bool },
}
