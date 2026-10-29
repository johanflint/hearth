use crate::domain::GroupOwner;
use std::collections::BTreeSet;

/// Represents a zone that can contain groups and devices. Zones may overlap.
#[derive(Clone, PartialEq, Debug)]
pub struct Zone {
    pub id: String,
    pub name: String,
    pub group_ids: BTreeSet<String>,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: GroupOwner,
    pub external_id: Option<String>,
}
