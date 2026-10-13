use crate::domain::Number;
use crate::domain::device::Device;
use crate::domain::property::{CartesianCoordinate, Gamut, PropertyLocator};
use chrono::{DateTime, Utc};

#[derive(PartialEq, Debug)]
pub enum Event {
    DiscoveredDevices(Vec<Device>),
    BooleanPropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: bool,
        initiated_by: InitiatedBy,
    },
    ColorPropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        xy: CartesianCoordinate,
        gamut: Option<Gamut>,
        initiated_by: InitiatedBy,
    },
    DateTimePropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: Option<DateTime<Utc>>,
        initiated_by: InitiatedBy,
    },
    NumberPropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: Option<Number>,
        initiated_by: InitiatedBy,
    },
    EnumPropertyChanged {
        device_id: String,
        property_id: PropertyLocator,
        value: Option<String>,
        initiated_by: InitiatedBy,
    },
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum InitiatedBy {
    Hearth,
    External,
}
