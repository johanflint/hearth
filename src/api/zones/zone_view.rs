use crate::api::owner_view::OwnerView;
use crate::domain::Zone;
use serde::Serialize;
use std::collections::BTreeSet;
use std::sync::Arc;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ZoneView {
    pub id: String,
    pub name: String,
    pub group_ids: BTreeSet<String>,
    pub device_ids: BTreeSet<String>,
    pub revision: u64,
    pub owner: OwnerView,
}

impl From<Arc<Zone>> for ZoneView {
    fn from(zone: Arc<Zone>) -> Self {
        Self {
            id: zone.id.clone(),
            name: zone.name.clone(),
            group_ids: zone.group_ids.clone(),
            device_ids: zone.device_ids.clone(),
            revision: zone.revision,
            owner: (&zone.owner).into(),
        }
    }
}
