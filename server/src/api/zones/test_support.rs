use crate::api::ApiState;
use crate::api::test_support::create_state_with_snapshot;
use crate::domain::{Owner, Zone};
use crate::store::StoreSnapshot;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub(super) fn ids(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

pub(super) fn zone(id: &str, name: &str, revision: u64, owner: Owner) -> Zone {
    Zone {
        id: id.to_string(),
        name: name.to_string(),
        group_ids: ids(&["group"]),
        device_ids: ids(&["device"]),
        revision,
        owner,
        external_id: Some("external".to_string()),
    }
}

/// The API state with the zones in the store
pub(super) fn state_with_zones(zones: Vec<Zone>) -> ApiState {
    let zones: HashMap<_, _> = zones.into_iter().map(|zone| (zone.id.clone(), Arc::new(zone))).collect();
    create_state_with_snapshot(StoreSnapshot { zones: Arc::new(zones), ..Default::default() })
}
