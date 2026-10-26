use std::collections::BTreeSet;

/// Represents a group of devices, to be controlled together
#[derive(PartialEq, Debug)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: GroupOwner,
}

#[derive(PartialEq, Debug)]
pub enum GroupOwner {
    #[allow(dead_code)]
    Hearth,
    Integration,
}
