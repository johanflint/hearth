use crate::domain::property::{
    BooleanProperty, ColorProperty, DateTimeProperty, EnumProperty, NumberProperty, Property, PropertyLocator,
};
use crate::domain::{Device, Event, Group, GroupChanged, GroupOwner, InitiatedBy};
use crate::metrics::{Metric, ResultOutcomeLabel};
use crate::property_changed_reducer::reduce_property_changed_event;
use metrics::{counter, gauge};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch::{Receiver as WatchReceiver, Sender as WatchSender};
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, instrument, warn};

pub type DeviceMap = HashMap<String, Arc<Device>>;
pub type GroupMap = HashMap<String, Arc<Group>>;

#[derive(Default, Clone, Debug)]
pub struct StoreSnapshot {
    pub devices: Arc<DeviceMap>,
    pub groups: Arc<GroupMap>,
}

// Delivered in event order to the reactive flow listener. Unlike the watch
// channel, this queue preserves intermediate snapshots while its receiver
// remains active. Only reactive flows need this, scheduled flows just read
// the current state from the StoreSnapshot.
#[derive(Clone, Debug)]
pub struct ReactiveUpdate {
    pub snapshot: StoreSnapshot,
    pub changed: Option<PropertyChange>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropertyChange {
    pub device_id: String,
    pub property_id: String,
    pub initiated_by: InitiatedBy,
}

#[derive(Debug)]
pub struct Store {
    devices: DeviceMap,
    groups: GroupMap,
    rx: Receiver<Event>,
    notifier_tx: WatchSender<StoreSnapshot>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
    reactive_tx: Sender<ReactiveUpdate>,
}

/// Capacity of the reactive update channel, see `ReactiveUpdate`.
///
/// Because `Store::listen()` awaits the `reactive_tx.send()`, filling this
/// channel deliberately blocks the store from processing further events
/// (and transitively, any observer from delivering them) rather than dropping
/// reactive updates. If reactive flows are consistently slower than incoming
/// events, raise this value or investigate the slow flow(s).
const REACTIVE_CHANNEL_CAPACITY: usize = 32;

impl Store {
    pub fn new(rx: Receiver<Event>) -> (Self, Receiver<ReactiveUpdate>) {
        let devices = HashMap::new();
        let groups = HashMap::new();
        let snapshot = StoreSnapshot { devices: Arc::new(devices.clone()), groups: Arc::new(groups.clone()) };
        let (notifier_tx, notifier_rx) = watch::channel::<StoreSnapshot>(snapshot);
        let (reactive_tx, reactive_rx) = mpsc::channel::<ReactiveUpdate>(REACTIVE_CHANNEL_CAPACITY);

        let store = Store { devices, groups, rx, notifier_tx, notifier_rx, reactive_tx };
        // Returning the receiver makes is impossible to forget to wire it
        (store, reactive_rx)
    }

    pub fn notifier(&self) -> WatchReceiver<StoreSnapshot> {
        self.notifier_rx.clone()
    }

    #[instrument(skip(self))]
    pub async fn listen(&mut self) {
        while let Some(event) = self.rx.recv().await {
            let changed = self.apply_event(event);
            let snapshot =
                StoreSnapshot { devices: Arc::new(self.devices.clone()), groups: Arc::new(self.groups.clone()) };
            self.notifier_tx.send(snapshot.clone()).unwrap_or_default();
            if let Err(e) = self.reactive_tx.send(ReactiveUpdate { snapshot, changed }).await {
                error!("⚠️ Unable to deliver reactive update, reactive flows will not run for this change: {}", e);
            }
            info!("🔄 Updated store");
        }
    }

    fn apply_event(&mut self, event: Event) -> Option<PropertyChange> {
        debug!("🔵 Received event: {:?}", event);

        match event {
            Event::DiscoveredDevices(discovered_devices) => {
                let num_devices = discovered_devices.len();
                debug!("🔵 Registering {} new device(s)...", num_devices);
                self.devices.extend(discovered_devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))));
                info!("🔵 Registering {} new device(s)... OK", num_devices);

                counter!(Metric::StoreDeviceDiscoveries.name()).increment(num_devices as u64);
                gauge!(Metric::StoreDeviceCount.name()).set(self.devices.len() as f64);
                None
            }
            Event::BooleanPropertyChanged { device_id, property_id, value, initiated_by } => {
                let Some(resolved_property_id) =
                    resolve_property_id::<BooleanProperty>(&self.devices, &device_id, &property_id)
                else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve boolean property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "boolean", "result" => "failure")
                        .increment(1);
                    return None;
                };
                let result = reduce_property_changed_event(
                    &mut self.devices,
                    &device_id,
                    &resolved_property_id,
                    initiated_by,
                    |property: &mut BooleanProperty| property.set_value(value),
                );
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "boolean", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange {
                    device_id,
                    property_id: resolved_property_id,
                    initiated_by,
                })
            }
            Event::DateTimePropertyChanged { device_id, property_id, value, initiated_by } => {
                let Some(resolved_property_id) =
                    resolve_property_id::<DateTimeProperty>(&self.devices, &device_id, &property_id)
                else {
                    error!(
                        device_id,
                        ?property_id,
                        "⚠️ Could not resolve date time property for device '{}'",
                        device_id
                    );
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "date_time", "result" => "failure").increment(1);
                    return None;
                };

                let result = reduce_property_changed_event(
                    &mut self.devices,
                    &device_id,
                    &resolved_property_id,
                    initiated_by,
                    |property: &mut DateTimeProperty| property.set_value(value),
                );
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "date_time", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange {
                    device_id,
                    property_id: resolved_property_id,
                    initiated_by,
                })
            }
            Event::ColorPropertyChanged { device_id, property_id, xy, gamut, initiated_by } => {
                let Some(resolved_property_id) =
                    resolve_property_id::<ColorProperty>(&self.devices, &device_id, &property_id)
                else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve color property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "color", "result" => "failure")
                        .increment(1);
                    return None;
                };
                let result = reduce_property_changed_event(
                    &mut self.devices,
                    &device_id,
                    &resolved_property_id,
                    initiated_by,
                    |property: &mut ColorProperty| property.set_value(xy, gamut),
                );
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "color", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange {
                    device_id,
                    property_id: resolved_property_id,
                    initiated_by,
                })
            }
            Event::NumberPropertyChanged { device_id, property_id, value, initiated_by } => {
                let Some(resolved_property_id) =
                    resolve_property_id::<NumberProperty>(&self.devices, &device_id, &property_id)
                else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve number property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "number", "result" => "failure")
                        .increment(1);
                    return None;
                };
                let result = reduce_property_changed_event(
                    &mut self.devices,
                    &device_id.clone(),
                    &resolved_property_id,
                    initiated_by,
                    move |property: &mut NumberProperty| property.set_value(value),
                );
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "number", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange {
                    device_id,
                    property_id: resolved_property_id,
                    initiated_by,
                })
            }
            Event::EnumPropertyChanged { device_id, property_id, value, initiated_by } => {
                let Some(resolved_property_id) =
                    resolve_property_id::<EnumProperty>(&self.devices, &device_id, &property_id)
                else {
                    error!(device_id, ?property_id, "⚠️ Could not resolve enum property for device '{}'", device_id);
                    counter!(Metric::StorePropertyChanges.name(), "property_type" => "enum", "result" => "failure")
                        .increment(1);
                    return None;
                };
                let result = reduce_property_changed_event(
                    &mut self.devices,
                    &device_id.clone(),
                    &resolved_property_id,
                    initiated_by,
                    move |property: &mut EnumProperty| property.set_value(value),
                );
                counter!(Metric::StorePropertyChanges.name(), "property_type" => "enum", "result" => result.metric_label()).increment(1);
                result.unwrap_or(false).then(|| PropertyChange {
                    device_id,
                    property_id: resolved_property_id,
                    initiated_by,
                })
            }
            Event::DiscoveredGroups(discovered_groups) => {
                let num_groups = discovered_groups.len();
                debug!("🔵 Registering {} new group(s)...", num_groups);
                self.groups.extend(discovered_groups.into_iter().map(|group| (group.id.clone(), Arc::new(group))));
                info!("🔵 Registering {} new group(s)... OK", num_groups);

                counter!(Metric::StoreGroupDiscoveries.name()).increment(num_groups as u64);
                gauge!(Metric::StoreGroupCount.name()).set(self.groups.len() as f64);
                None
            }
            Event::GroupChanged(GroupChanged { id, name, device_ids, external_id }) => {
                let group = match self.groups.get(&id) {
                    Some(existing) => {
                        let existing = Group::clone(existing);
                        Group {
                            name: name.unwrap_or(existing.name),
                            device_ids: device_ids.unwrap_or(existing.device_ids),
                            external_id: external_id.or(existing.external_id),
                            ..existing
                        }
                    }
                    None => match (name, device_ids) {
                        (Some(name), Some(device_ids)) => Group {
                            id: id.clone(),
                            name,
                            device_ids,
                            revision: 0,
                            owner: GroupOwner::Integration,
                            external_id,
                        },
                        _ => {
                            warn!("⚠️ Ignoring partial update for an unknown group '{id}'");
                            return None;
                        }
                    },
                };

                // Discovery skips groups without lights, so a group that lost its last light is removed
                if group.device_ids.is_empty() {
                    self.groups.remove(&id);
                } else {
                    self.groups.insert(id, Arc::new(group));
                }

                gauge!(Metric::StoreGroupCount.name()).set(self.groups.len() as f64);
                None
            }
            Event::GroupRemoved { group_id } => {
                self.groups.remove(&group_id);
                gauge!(Metric::StoreGroupCount.name()).set(self.groups.len() as f64);
                None
            }
        }
    }
}

// Resolves a PropertyLocator (by name, or by the controller-owned external_id of the resource it
// represents) to the property's well-known name, as expected by `reduce_property_changed_event`.
fn resolve_property_id<T: 'static + Property>(
    devices: &DeviceMap,
    device_id: &str,
    locator: &PropertyLocator,
) -> Option<String> {
    devices.get(device_id)?.resolve_property::<T>(locator).map(|property| property.name().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Group, GroupOwner, InitiatedBy};
    use crate::test_support::DeviceBuilder;
    use std::collections::BTreeSet;
    use tokio::sync::mpsc::channel;

    fn group(id: &str, name: &str, device_ids: &[&str]) -> Group {
        Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: device_ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: None,
        }
    }

    fn start_store() -> (Sender<Event>, Receiver<ReactiveUpdate>) {
        let (tx, rx) = channel::<Event>(8);
        let (mut store, reactive_rx) = Store::new(rx);
        tokio::spawn(async move {
            store.listen().await;
        });
        (tx, reactive_rx)
    }

    #[tokio::test]
    async fn discovered_groups_are_stored_without_changing_the_devices() {
        let (tx, mut reactive_rx) = start_store();

        let device = DeviceBuilder::new("device").with_boolean_property("on", false).build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();
        reactive_rx.recv().await.expect("devices should produce a reactive update");

        tx.send(Event::DiscoveredGroups(vec![group("group", "Group", &["device"])])).await.unwrap();
        let update = reactive_rx.recv().await.expect("groups should produce a reactive update");

        assert_eq!(update.changed, None);
        assert_eq!(update.snapshot.groups.len(), 1);
        assert_eq!(*update.snapshot.groups["group"], group("group", "Group", &["device"]));
        assert_eq!(update.snapshot.devices.len(), 1);
        assert!(update.snapshot.devices.contains_key("device"));
    }

    #[tokio::test]
    async fn discovered_groups_replace_a_group_with_the_same_id() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(Event::DiscoveredGroups(vec![group("group", "Old name", &["a"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(Event::DiscoveredGroups(vec![group("group", "New name", &["a", "b"])])).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.snapshot.groups.len(), 1);
        assert_eq!(*update.snapshot.groups["group"], group("group", "New name", &["a", "b"]));
    }

    #[tokio::test]
    async fn discovered_groups_are_added_to_existing_groups() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(Event::DiscoveredGroups(vec![group("first", "First", &["a"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(Event::DiscoveredGroups(vec![group("second", "Second", &["b"])])).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        let mut ids: Vec<_> = update.snapshot.groups.keys().cloned().collect();
        ids.sort();
        assert_eq!(ids, vec!["first", "second"]);
    }

    #[tokio::test]
    async fn discovered_devices_leave_the_groups_unchanged() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(Event::DiscoveredGroups(vec![group("group", "Group", &["device"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        let device = DeviceBuilder::new("device").with_boolean_property("on", false).build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.snapshot.groups.len(), 1);
        assert!(update.snapshot.groups.contains_key("group"));
        assert_eq!(update.snapshot.devices.len(), 1);
    }

    fn group_changed(id: &str, name: Option<&str>, device_ids: Option<&[&str]>) -> Event {
        Event::GroupChanged(GroupChanged {
            id: id.to_string(),
            name: name.map(str::to_string),
            device_ids: device_ids.map(|ids| ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>()),
            external_id: None,
        })
    }

    fn group_changed_with_external_id(id: &str, external_id: &str) -> Event {
        Event::GroupChanged(GroupChanged {
            id: id.to_string(),
            name: None,
            device_ids: None,
            external_id: Some(external_id.to_string()),
        })
    }

    #[tokio::test]
    async fn group_changed_with_name_and_devices_creates_an_unknown_group() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(group_changed("group", Some("Group"), Some(&["a", "b"]))).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.changed, None);
        assert_eq!(update.snapshot.groups.len(), 1);
        assert_eq!(*update.snapshot.groups["group"], group("group", "Group", &["a", "b"]));
    }

    #[tokio::test]
    async fn group_changed_creates_a_group_with_the_external_id() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(Event::GroupChanged(GroupChanged {
            id: "group".to_string(),
            name: Some("Group".to_string()),
            device_ids: Some(BTreeSet::from(["a".to_string(), "b".to_string()])),
            external_id: Some("grouped-light".to_string()),
        }))
        .await
        .unwrap();
        let update = reactive_rx.recv().await.unwrap();

        let expected = Group { external_id: Some("grouped-light".to_string()), ..group("group", "Group", &["a", "b"]) };
        assert_eq!(*update.snapshot.groups["group"], expected);
    }

    #[tokio::test]
    async fn group_changed_without_an_external_id_keeps_the_existing_one() {
        let (tx, mut reactive_rx) = start_store();
        let existing = Group { external_id: Some("grouped-light".to_string()), ..group("group", "Group", &["a"]) };
        tx.send(Event::DiscoveredGroups(vec![existing])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("group", Some("Renamed"), None)).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.snapshot.groups["group"].external_id.as_deref(), Some("grouped-light"));
    }

    #[tokio::test]
    async fn group_changed_with_an_external_id_replaces_the_existing_one() {
        let (tx, mut reactive_rx) = start_store();
        let existing = Group { external_id: Some("old".to_string()), ..group("group", "Group", &["a"]) };
        tx.send(Event::DiscoveredGroups(vec![existing])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed_with_external_id("group", "new")).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.snapshot.groups["group"].external_id.as_deref(), Some("new"));
    }

    #[tokio::test]
    async fn group_changed_with_only_a_name_keeps_the_devices() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("group", "Old name", &["a", "b"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("group", Some("New name"), None)).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(*update.snapshot.groups["group"], group("group", "New name", &["a", "b"]));
    }

    #[tokio::test]
    async fn group_changed_with_only_devices_keeps_the_name() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("group", "Group", &["a"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("group", None, Some(&["a", "b"]))).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(*update.snapshot.groups["group"], group("group", "Group", &["a", "b"]));
    }

    #[tokio::test]
    async fn group_changed_keeps_the_revision_and_owner() {
        let (tx, mut reactive_rx) = start_store();
        let existing = Group { revision: 3, owner: GroupOwner::Hearth, ..group("group", "Group", &["a"]) };
        tx.send(Event::DiscoveredGroups(vec![existing])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("group", Some("Renamed"), None)).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        let stored = &update.snapshot.groups["group"];
        assert_eq!(stored.revision, 3);
        assert_eq!(stored.owner, GroupOwner::Hearth);
    }

    #[tokio::test]
    async fn group_changed_leaves_other_groups_unchanged() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("first", "First", &["a"]), group("second", "Second", &["b"])]))
            .await
            .unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("first", Some("Renamed"), None)).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(*update.snapshot.groups["second"], group("second", "Second", &["b"]));
    }

    #[tokio::test]
    async fn group_changed_without_devices_removes_the_group() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("group", "Group", &["a"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(group_changed("group", None, Some(&[]))).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert!(update.snapshot.groups.is_empty());
    }

    #[tokio::test]
    async fn group_changed_for_a_new_group_without_devices_creates_nothing() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(group_changed("group", Some("Group"), Some(&[]))).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert!(update.snapshot.groups.is_empty());
    }

    #[tokio::test]
    async fn partial_group_changed_for_an_unknown_group_is_ignored() {
        let (tx, mut reactive_rx) = start_store();

        tx.send(group_changed("group", Some("Group"), None)).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();
        assert!(update.snapshot.groups.is_empty());

        tx.send(group_changed("group", None, Some(&["a"]))).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();
        assert!(update.snapshot.groups.is_empty());
    }

    #[tokio::test]
    async fn group_removed_removes_only_that_group() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("first", "First", &["a"]), group("second", "Second", &["b"])]))
            .await
            .unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(Event::GroupRemoved { group_id: "first".to_string() }).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.changed, None);
        assert_eq!(update.snapshot.groups.len(), 1);
        assert!(update.snapshot.groups.contains_key("second"));
    }

    #[tokio::test]
    async fn group_removed_for_an_unknown_group_changes_nothing() {
        let (tx, mut reactive_rx) = start_store();
        tx.send(Event::DiscoveredGroups(vec![group("group", "Group", &["a"])])).await.unwrap();
        reactive_rx.recv().await.unwrap();

        tx.send(Event::GroupRemoved { group_id: "unknown".to_string() }).await.unwrap();
        let update = reactive_rx.recv().await.unwrap();

        assert_eq!(update.snapshot.groups.len(), 1);
    }

    #[tokio::test]
    async fn property_change_is_persisted_in_the_store() {
        let (tx, rx) = channel::<Event>(8);
        let (mut store, _reactive_rx) = Store::new(rx);
        let mut notifier = store.notifier();

        tokio::spawn(async move {
            store.listen().await;
        });

        // Discover the device whose "on" property is false
        let device = DeviceBuilder::new("device").with_boolean_property("on", false).build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();
        notifier.changed().await.unwrap();

        // Flip the property to true
        tx.send(Event::BooleanPropertyChanged {
            device_id: "device".to_string(),
            property_id: PropertyLocator::Name("on".to_string()),
            value: true,
            initiated_by: InitiatedBy::Hearth,
        })
        .await
        .unwrap();
        notifier.changed().await.unwrap();

        let snapshot = notifier.borrow().clone();
        let device = snapshot.devices.get("device").expect("device should exist in snapshot");
        let on_property = device.get_property::<BooleanProperty>("on").expect("property should exist");

        assert_eq!(on_property.value(), true, "expected the store snapshot to reflect the property change");
    }

    #[tokio::test]
    async fn reactive_updates_are_never_merged_even_under_back_to_back_events() {
        // Regression test: every event must produce its own reactive update, in
        // order, with no merging/dropping - even when several events are sent
        // before the store has a chance to process the first one.
        let (tx, rx) = channel::<Event>(8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        let device = DeviceBuilder::new("device")
            .with_boolean_property("on", false)
            .with_boolean_property("motion", false)
            .build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();

        // Sent back-to-back, without awaiting a reactive update in between
        tx.send(Event::BooleanPropertyChanged {
            device_id: "device".to_string(),
            property_id: PropertyLocator::Name("on".to_string()),
            value: true,
            initiated_by: InitiatedBy::Hearth,
        })
        .await
        .unwrap();
        tx.send(Event::BooleanPropertyChanged {
            device_id: "device".to_string(),
            property_id: PropertyLocator::Name("motion".to_string()),
            value: true,
            initiated_by: InitiatedBy::Hearth,
        })
        .await
        .unwrap();

        let discovered = reactive_rx.recv().await.expect("expected a reactive update for device discovery");
        assert_eq!(discovered.changed, None);

        let first = reactive_rx.recv().await.expect("expected a reactive update for the first property change");
        assert_eq!(
            first.changed,
            Some(PropertyChange {
                device_id: "device".to_string(),
                property_id: "on".to_string(),
                initiated_by: InitiatedBy::Hearth
            })
        );

        let second = reactive_rx.recv().await.expect("expected a reactive update for the second property change");
        assert_eq!(
            second.changed,
            Some(PropertyChange {
                device_id: "device".to_string(),
                property_id: "motion".to_string(),
                initiated_by: InitiatedBy::Hearth
            })
        );
    }

    #[tokio::test]
    async fn reactive_update_reports_no_change_when_the_event_is_rejected() {
        let (tx, rx) = channel::<Event>(8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        // No device named "unknown" exists, so the reducer rejects this event
        tx.send(Event::BooleanPropertyChanged {
            device_id: "unknown".to_string(),
            property_id: PropertyLocator::Name("on".to_string()),
            value: true,
            initiated_by: InitiatedBy::Hearth,
        })
        .await
        .unwrap();

        let update = reactive_rx.recv().await.expect("expected a reactive update even for a rejected event");
        assert_eq!(update.changed, None, "a rejected event must not be reported as a property change");
    }

    #[tokio::test]
    async fn reactive_updates_survive_filling_the_channel_beyond_capacity() {
        // Regression test for the channel's core promise: even once the reactive
        // channel (REACTIVE_CHANNEL_CAPACITY slots) fills up completely because the
        // consumer is falling behind, Store::listen just blocks until there's room -
        // no update is ever dropped or merged, and order is preserved.
        let num_events = REACTIVE_CHANNEL_CAPACITY * 2;
        let (tx, rx) = channel::<Event>(num_events + 8);
        let (mut store, mut reactive_rx) = Store::new(rx);

        tokio::spawn(async move {
            store.listen().await;
        });

        let device = DeviceBuilder::new("device").with_boolean_property("on", false).build();
        tx.send(Event::DiscoveredDevices(vec![device])).await.unwrap();

        // Queue well past the reactive channel's capacity before consuming anything
        for i in 0..num_events {
            tx.send(Event::BooleanPropertyChanged {
                device_id: "device".to_string(),
                property_id: PropertyLocator::Name("on".to_string()),
                value: i % 2 == 0,
                initiated_by: InitiatedBy::Hearth,
            })
            .await
            .unwrap();
        }

        let discovered = reactive_rx.recv().await.expect("expected the discovery update");
        assert_eq!(discovered.changed, None);

        for i in 0..num_events {
            let update =
                reactive_rx.recv().await.expect("expected every queued event to eventually arrive, none dropped");
            let value =
                update.snapshot.devices.get("device").unwrap().get_property::<BooleanProperty>("on").unwrap().value();
            assert_eq!(
                value,
                i % 2 == 0,
                "update #{i} must reflect its own event's value, not a later one merged into it"
            );
        }
    }
}
