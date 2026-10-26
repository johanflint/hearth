use crate::domain::device::Device;
use crate::domain::property::{CartesianCoordinate, Gamut, PropertyLocator};
use crate::domain::{Group, Number};
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

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
    DiscoveredGroups(Vec<Group>),
    GroupChanged(GroupChanged),
    GroupRemoved {
        group_id: String,
    },
}

#[derive(Copy, Clone, PartialEq, Debug)]
pub enum InitiatedBy {
    Hearth,
    /// User initiated outside of Hearth
    External,
    /// The device corrected its own reported state, nobody changed it
    Device,
}

#[derive(PartialEq, Debug)]
pub struct GroupChanged {
    pub id: String,
    pub name: Option<String>,
    pub device_ids: Option<BTreeSet<String>>,
}
