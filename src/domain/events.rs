use crate::domain::device::Device;

#[derive(Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
}
