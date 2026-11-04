use crate::domain::Device;
use crate::domain::owner::Owner;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// Represents a group of devices, to be controlled together
#[derive(Clone, PartialEq, Debug)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: Owner,
    pub external_id: Option<String>,
}

impl Group {
    pub fn unknown_device_ids<'a>(
        &'a self,
        devices: &'a HashMap<String, Arc<Device>>,
    ) -> impl Iterator<Item = &'a String> {
        self.device_ids.iter().filter(|id| !devices.contains_key(*id))
    }
}
