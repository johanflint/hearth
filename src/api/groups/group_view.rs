use crate::domain::{Group, GroupOwner};
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GroupView {
    pub id: String,
    pub name: String,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: GroupOwnerView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum GroupOwnerView {
    Hearth,
    Integration,
}

impl From<Arc<Group>> for GroupView {
    fn from(group: Arc<Group>) -> Self {
        Self {
            id: group.id.clone(),
            name: group.name.clone(),
            device_ids: group.device_ids.clone(),
            revision: group.revision,
            owner: (&group.owner).into(),
        }
    }
}

impl From<&GroupOwner> for GroupOwnerView {
    fn from(owner: &GroupOwner) -> Self {
        match owner {
            GroupOwner::Hearth => GroupOwnerView::Hearth,
            GroupOwner::Integration => GroupOwnerView::Integration,
        }
    }
}
