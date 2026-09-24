use crate::domain::Number;
use crate::domain::device::Device;
use crate::domain::property::{CartesianCoordinate, Gamut, PropertyLocator};
use chrono::{DateTime, Utc};

#[derive(PartialEq, Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
    BooleanPropertyChanged {
        device_id: String,
        property_id: String,
        value: bool,
    },
    ColorPropertyChanged {
        device_id: String,
        property_id: String,
        xy: CartesianCoordinate,
        gamut: Option<Gamut>,
    },
    DateTimePropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: Option<DateTime<Utc>>,
    },
    NumberPropertyChanged {
        device_id: String,
        property_id: String,
        value: Option<Number>,
    },
    EnumPropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: Option<String>,
    },
}
