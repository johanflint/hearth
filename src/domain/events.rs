use crate::domain::Number;
use crate::domain::device::Device;
use crate::domain::property::{CartesianCoordinate, Gamut};

#[derive(PartialEq, Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
    BooleanPropertyChanged {
        device_id: String,
        property_id: String,
        value: bool,
    },
    NumberPropertyChanged {
        device_id: String,
        property_id: String,
        value: Number,
    },
    ColorPropertyChanged {
        device_id: String,
        property_id: String,
        xy: CartesianCoordinate,
        gamut: Option<Gamut>,
    },
}
