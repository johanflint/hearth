use crate::claim_registry;
use crate::claim_registry::ClaimOrigin;
use crate::domain::GeoLocation;
use crate::domain::events::InitiatedBy;
use crate::domain::property::{BooleanProperty, PropertyType};
use crate::execute_flows::execute_flows;
use crate::flow_engine::VersionedFlow;
use crate::flow_registry::FlowRegistry;
use crate::scheduler::SchedulerCommand;
use crate::store::{PropertyChange, ReactiveUpdate, StoreSnapshot};
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::instrument;

#[instrument(skip_all)]
pub async fn store_listener(mut rx: Receiver<ReactiveUpdate>, flow_registry: Arc<FlowRegistry>, scheduler_tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    while let Some(update) = rx.recv().await {
        if let Some(change) = &update.changed {
            update_claim(&update.snapshot, change);
        }

        let flows = flow_registry.reactive_flows()
            .into_iter()
            .map(VersionedFlow::into_flow)
            .collect::<Vec<_>>();
        execute_flows(flows, update.snapshot, update.changed, scheduler_tx.clone(), geo_location.clone()).await;
    }
}

fn update_claim(snapshot: &StoreSnapshot, change: &PropertyChange) {
    let Some(device) = snapshot.devices.get(&change.device_id) else {
        return;
    };

    if !device.claimable {
        // Most devices never hold claims
        return;
    }

    match change.initiated_by {
        InitiatedBy::Hearth => return, // Hearth's writes carry their own claim, it's applied before they are sent
        InitiatedBy::Device => return, // Nobody changed the device
        InitiatedBy::External => {},
    }

    // The claim and release can't fail, claimability is checked above
    let on_value = device.get_property_of_type::<BooleanProperty>(PropertyType::On).map(|on| on.value());
    if on_value == Some(false) {
        let _ = claim_registry::release(device);
    } else {
        let _ = claim_registry::claim(device, ClaimOrigin::External);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claim_registry::AuthorizeError;
    use crate::domain::Device;
    use crate::test_support::DeviceBuilder;
    use std::collections::HashMap;

    // The claim registry is global, so every test uses its own device id
    fn snapshot(device: Device) -> StoreSnapshot {
        StoreSnapshot { devices: Arc::new(HashMap::from([(device.id.clone(), Arc::new(device))])) }
    }

    fn change(device_id: &str, initiated_by: InitiatedBy) -> PropertyChange {
        PropertyChange { device_id: device_id.to_string(), property_id: "on".to_string(), initiated_by }
    }

    fn claimed_by(device: &Device) -> Option<ClaimOrigin> {
        match claim_registry::authorize(device, None) {
            Err(AuthorizeError::Claimed(origin)) => Some(origin),
            _ => None,
        }
    }

    #[test]
    fn update_claim_claims_a_light_that_is_changed_externally() {
        let device = DeviceBuilder::new("claims_external").with_boolean_property("on", true).build();
        update_claim(&snapshot(device.clone()), &change(&device.id, InitiatedBy::External));

        assert_eq!(claimed_by(&device), Some(ClaimOrigin::External));
    }

    #[test]
    fn update_claim_ignores_a_change_made_by_hearth() {
        let device = DeviceBuilder::new("ignores_hearth").with_boolean_property("on", true).build();
        update_claim(&snapshot(device.clone()), &change(&device.id, InitiatedBy::Hearth));

        assert_eq!(claimed_by(&device), None);
    }

    #[test]
    fn update_claim_ignores_a_correction_by_the_device() {
        let device = DeviceBuilder::new("ignores_device").with_boolean_property("on", true).build();
        update_claim(&snapshot(device.clone()), &change(&device.id, InitiatedBy::Device));

        assert_eq!(claimed_by(&device), None);
    }

    #[test]
    fn update_claim_keeps_the_claim_when_hearth_turns_a_light_off() {
        let device = DeviceBuilder::new("keeps_hearth_off").with_boolean_property("on", true).build();
        claim_registry::claim(&device, ClaimOrigin::Flows(vec!["flow".to_string()])).unwrap();

        let off = DeviceBuilder::new("keeps_hearth_off").with_boolean_property("on", false).build();
        update_claim(&snapshot(off), &change("keeps_hearth_off", InitiatedBy::Hearth));

        assert_eq!(claimed_by(&device), Some(ClaimOrigin::Flows(vec!["flow".to_string()])));
    }

    #[test]
    fn update_claim_releases_a_light_that_is_turned_off_externally() {
        let device = DeviceBuilder::new("releases_off").with_boolean_property("on", true).build();
        update_claim(&snapshot(device.clone()), &change(&device.id, InitiatedBy::External));

        let off = DeviceBuilder::new("releases_off").with_boolean_property("on", false).build();
        update_claim(&snapshot(off), &change("releases_off", InitiatedBy::External));

        assert_eq!(claimed_by(&device), None);
    }

    #[test]
    fn update_claim_ignores_a_device_that_is_not_claimable() {
        let device = Device { claimable: false, ..DeviceBuilder::new("ignores_unclaimable").with_boolean_property("on", true).build() };
        update_claim(&snapshot(device.clone()), &change(&device.id, InitiatedBy::External));

        let claimable = Device { claimable: true, ..device };
        assert_eq!(claimed_by(&claimable), None);
    }
}
