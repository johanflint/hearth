use crate::domain::{Group, Owner};
use crate::group_store::StoredGroup;
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
    pub owner: OwnerView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum OwnerView {
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

impl From<StoredGroup> for GroupView {
    fn from(group: StoredGroup) -> Self {
        Self {
            id: group.id,
            name: group.name,
            device_ids: group.device_ids,
            revision: group.revision,
            owner: OwnerView::Hearth,
        }
    }
}

impl From<&Owner> for OwnerView {
    fn from(owner: &Owner) -> Self {
        match owner {
            Owner::Hearth => OwnerView::Hearth,
            Owner::Integration => OwnerView::Integration,
        }
    }
}
