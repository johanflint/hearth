use std::collections::BTreeSet;

/// Represents a group of devices, to be controlled together
#[derive(Clone, PartialEq, Debug)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: GroupOwner,
    pub external_id: Option<String>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum GroupOwner {
    #[allow(dead_code)]
    Hearth,
    Integration,
}
