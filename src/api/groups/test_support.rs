use crate::api::ApiState;
use crate::api::test_support::create_state_with_snapshot;
use crate::domain::{Group, Owner};
use crate::store::StoreSnapshot;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

pub(super) fn group(id: &str, name: &str, revision: u64, owner: Owner) -> Group {
    Group {
        id: id.to_string(),
        name: name.to_string(),
        device_ids: BTreeSet::from(["device".to_string()]),
        revision,
        owner,
        external_id: Some("external".to_string()),
    }
}

/// The API state with the groups in the store
pub(super) fn state_with_groups(groups: Vec<Group>) -> ApiState {
    let groups: HashMap<_, _> = groups.into_iter().map(|group| (group.id.clone(), Arc::new(group))).collect();
    create_state_with_snapshot(StoreSnapshot { groups: Arc::new(groups), ..Default::default() })
}
