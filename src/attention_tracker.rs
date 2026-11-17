use crate::attention::{Attention, Issue, Subject};
use crate::domain::property::{EnumProperty, PropertyType};
use crate::domain::{BatteryState, Connectivity, Device};
use crate::store::StoreSnapshot;
use chrono::{DateTime, Utc};
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::sync::watch::Receiver as WatchReceiver;

pub struct AttentionTracker {
    store_rx: WatchReceiver<StoreSnapshot>,
    attention_tx: watch::Sender<Arc<[Attention]>>,
}

impl AttentionTracker {
    pub fn new(store_rx: WatchReceiver<StoreSnapshot>) -> (Self, watch::Receiver<Arc<[Attention]>>) {
        let (attention_tx, attention_rx) = watch::channel(Arc::from([]));

        (AttentionTracker { store_rx, attention_tx }, attention_rx)
    }

    pub async fn track(mut self) {
        self.store_rx.mark_changed();
        while self.store_rx.changed().await.is_ok() {
            let snapshot = self.store_rx.borrow_and_update().clone();
            // The published list is the previous state, so there is nothing else to keep track of
            self.attention_tx.send_if_modified(|current| {
                let attention = attention_for_devices(&snapshot, current, Utc::now());
                if attention == *current {
                    return false;
                }
                *current = attention;
                true
            });
        }
    }
}

fn attention_for_devices(snapshot: &StoreSnapshot, previous: &[Attention], now: DateTime<Utc>) -> Arc<[Attention]> {
    let mut attention: Vec<_> = snapshot
        .devices
        .values()
        .flat_map(|device| {
            device_issues(device).into_iter().map(|issue| {
                let subject = Subject::Device(device.id.clone());
                // An issue that was already there keeps its `since`, also when its level changes (Low -> Critical)
                let since = previous
                    .iter()
                    .find(|p| p.subject == subject && p.issue.is_same_kind(&issue))
                    .map_or(now, |p| p.since);
                Attention { subject, issue, since }
            })
        })
        .collect();
    // A HashMap has no order, without sorting every update looks like a change
    attention.sort_by(|a, b| a.subject.cmp(&b.subject));
    attention.into()
}

fn device_issues(device: &Device) -> Vec<Issue> {
    let value = device
        .get_property_of_type::<EnumProperty>(PropertyType::Connectivity)
        .and_then(|p| p.value())
        .and_then(|v| Connectivity::from_str(v).ok());

    let mut issues = vec![];
    if let Some(connectivity) = value
        && matches!(connectivity, Connectivity::Issues | Connectivity::Disconnected)
    {
        issues.push(Issue::Connectivity(connectivity))
    }

    let value = device
        .get_property_of_type::<EnumProperty>(PropertyType::BatteryState)
        .and_then(|p| p.value())
        .and_then(|v| BatteryState::from_str(v).ok());

    if let Some(battery_state) = value
        && matches!(battery_state, BatteryState::Low | BatteryState::Critical)
    {
        issues.push(Issue::Battery(battery_state))
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::Property;
    use crate::test_support::DeviceBuilder;
    use chrono::TimeZone;
    use rstest::rstest;
    use std::collections::HashMap;
    use std::time::Duration;
    use strum::IntoEnumIterator;
    use tokio::time::timeout;

    fn at(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, minute, 0).unwrap()
    }

    fn enum_property<T: IntoEnumIterator + ToString>(property_type: PropertyType, value: T) -> Box<dyn Property> {
        let allowed_values = T::iter().map(|v| v.to_string()).collect();
        Box::new(
            EnumProperty::new(
                format!("{property_type:?}"),
                property_type,
                true,
                None,
                Some(value.to_string()),
                allowed_values,
            )
            .unwrap(),
        )
    }

    fn device(id: &str, connectivity: Option<Connectivity>, battery: Option<BatteryState>) -> Device {
        let properties = [
            connectivity.map(|c| enum_property(PropertyType::Connectivity, c)),
            battery.map(|b| enum_property(PropertyType::BatteryState, b)),
        ];
        DeviceBuilder::new(id).with_properties(properties.into_iter().flatten().collect()).build()
    }

    fn battery_device(id: &str, battery: BatteryState) -> Device {
        device(id, None, Some(battery))
    }

    fn snapshot(devices: Vec<Device>) -> StoreSnapshot {
        let devices: HashMap<_, _> = devices.into_iter().map(|d| (d.id.clone(), Arc::new(d))).collect();
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn subjects(attention: &[Attention]) -> Vec<&Subject> {
        attention.iter().map(|a| &a.subject).collect()
    }

    #[rstest]
    #[case::connected(Connectivity::Connected, vec![])]
    #[case::unknown(Connectivity::Unknown, vec![])]
    #[case::issues(Connectivity::Issues, vec![Issue::Connectivity(Connectivity::Issues)])]
    #[case::disconnected(Connectivity::Disconnected, vec![Issue::Connectivity(Connectivity::Disconnected)])]
    fn device_issues_reports_connectivity_problems(#[case] connectivity: Connectivity, #[case] expected: Vec<Issue>) {
        assert_eq!(device_issues(&device("1", Some(connectivity), None)), expected);
    }

    #[rstest]
    #[case::normal(BatteryState::Normal, vec![])]
    #[case::low(BatteryState::Low, vec![Issue::Battery(BatteryState::Low)])]
    #[case::critical(BatteryState::Critical, vec![Issue::Battery(BatteryState::Critical)])]
    fn device_issues_reports_battery_problems(#[case] battery: BatteryState, #[case] expected: Vec<Issue>) {
        assert_eq!(device_issues(&battery_device("1", battery)), expected);
    }

    #[test]
    fn device_issues_reports_every_problem_of_a_device() {
        let device = device("1", Some(Connectivity::Disconnected), Some(BatteryState::Critical));

        assert_eq!(
            device_issues(&device),
            vec![Issue::Connectivity(Connectivity::Disconnected), Issue::Battery(BatteryState::Critical)]
        );
    }

    #[test]
    fn device_issues_ignores_a_device_without_the_properties() {
        assert_eq!(device_issues(&DeviceBuilder::new("1").build()), vec![]);
    }

    #[test]
    fn device_issues_ignores_a_property_without_a_value() {
        let allowed_values = BatteryState::iter().map(|s| s.to_string()).collect();
        let property =
            EnumProperty::new("battery".into(), PropertyType::BatteryState, true, None, None, allowed_values);
        let device = DeviceBuilder::new("1").with_properties(vec![Box::new(property.unwrap())]).build();

        assert_eq!(device_issues(&device), vec![]);
    }

    #[test]
    fn attention_for_devices_is_empty_without_issues() {
        let devices = snapshot(vec![battery_device("1", BatteryState::Normal)]);

        assert!(attention_for_devices(&devices, &[], at(0)).is_empty());
    }

    #[test]
    fn attention_for_devices_starts_new_issues_now() {
        let attention = attention_for_devices(&snapshot(vec![battery_device("1", BatteryState::Low)]), &[], at(5));

        assert_eq!(
            *attention,
            [Attention {
                subject: Subject::Device("1".into()),
                issue: Issue::Battery(BatteryState::Low),
                since: at(5)
            }]
        );
    }

    #[test]
    fn attention_for_devices_keeps_since_of_an_ongoing_issue() {
        let devices = snapshot(vec![battery_device("1", BatteryState::Low)]);
        let first = attention_for_devices(&devices, &[], at(0));

        let second = attention_for_devices(&devices, &first, at(5));

        assert_eq!(first, second);
    }

    #[test]
    fn attention_for_devices_keeps_since_when_the_level_changes() {
        let first = attention_for_devices(&snapshot(vec![battery_device("1", BatteryState::Low)]), &[], at(0));

        let second = attention_for_devices(&snapshot(vec![battery_device("1", BatteryState::Critical)]), &first, at(5));

        assert_eq!(second[0].issue, Issue::Battery(BatteryState::Critical));
        assert_eq!(second[0].since, at(0));
    }

    #[test]
    fn attention_for_devices_starts_over_when_an_issue_returns() {
        let low = snapshot(vec![battery_device("1", BatteryState::Low)]);
        let first = attention_for_devices(&low, &[], at(0));
        let resolved = attention_for_devices(&snapshot(vec![battery_device("1", BatteryState::Normal)]), &first, at(5));
        assert!(resolved.is_empty());

        let again = attention_for_devices(&low, &resolved, at(10));

        assert_eq!(again[0].since, at(10));
    }

    #[test]
    fn attention_for_devices_keeps_since_per_kind_of_issue() {
        let before = snapshot(vec![device("1", Some(Connectivity::Disconnected), None)]);
        let first = attention_for_devices(&before, &[], at(0));

        let after = snapshot(vec![device("1", Some(Connectivity::Disconnected), Some(BatteryState::Low))]);
        let second = attention_for_devices(&after, &first, at(5));

        assert_eq!(second[0].since, at(0), "the connectivity issue is ongoing");
        assert_eq!(second[1].since, at(5), "the battery issue is new");
    }

    #[test]
    fn attention_for_devices_does_not_mix_up_devices() {
        let first = attention_for_devices(&snapshot(vec![battery_device("1", BatteryState::Low)]), &[], at(0));

        let second = attention_for_devices(&snapshot(vec![battery_device("2", BatteryState::Low)]), &first, at(5));

        assert_eq!(second[0].subject, Subject::Device("2".into()));
        assert_eq!(second[0].since, at(5));
    }

    #[test]
    fn attention_for_devices_is_sorted_by_device() {
        let devices = ["c", "a", "d", "b"].map(|id| battery_device(id, BatteryState::Low)).into();

        let attention = attention_for_devices(&snapshot(devices), &[], at(0));

        let expected = ["a", "b", "c", "d"].map(|id| Subject::Device(id.into()));
        assert_eq!(subjects(&attention), expected.iter().collect::<Vec<_>>());
    }

    #[test]
    fn is_same_kind_ignores_the_level() {
        let low = Issue::Battery(BatteryState::Low);

        assert!(low.is_same_kind(&Issue::Battery(BatteryState::Critical)));
        assert!(!low.is_same_kind(&Issue::Connectivity(Connectivity::Disconnected)));
    }

    fn start_tracker(devices: Vec<Device>) -> (watch::Sender<StoreSnapshot>, watch::Receiver<Arc<[Attention]>>) {
        let (store_tx, store_rx) = watch::channel(snapshot(devices));
        let (tracker, attention_rx) = AttentionTracker::new(store_rx);
        tokio::spawn(tracker.track());
        (store_tx, attention_rx)
    }

    // The tracker is idle once it has processed every update, a single-threaded runtime gets there on yielding
    async fn let_tracker_process() {
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn track_publishes_the_issues_that_exist_at_startup() {
        let (_store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)]);

        attention_rx.changed().await.unwrap();

        assert_eq!(subjects(&attention_rx.borrow()), [&Subject::Device("1".into())]);
    }

    #[tokio::test]
    async fn track_publishes_new_issues() {
        let (store_tx, mut attention_rx) = start_tracker(vec![]);
        let_tracker_process().await;

        store_tx.send(snapshot(vec![battery_device("1", BatteryState::Critical)])).unwrap();

        attention_rx.changed().await.unwrap();
        assert_eq!(attention_rx.borrow()[0].issue, Issue::Battery(BatteryState::Critical));
    }

    #[tokio::test]
    async fn track_publishes_when_an_issue_is_resolved() {
        let (store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)]);
        attention_rx.changed().await.unwrap();

        store_tx.send(snapshot(vec![battery_device("1", BatteryState::Normal)])).unwrap();

        attention_rx.changed().await.unwrap();
        assert!(attention_rx.borrow().is_empty());
    }

    #[tokio::test]
    async fn track_does_not_publish_changes_that_need_no_attention() {
        let (store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)]);
        attention_rx.changed().await.unwrap();

        store_tx
            .send(snapshot(vec![battery_device("1", BatteryState::Low), battery_device("2", BatteryState::Normal)]))
            .unwrap();
        let_tracker_process().await;

        assert!(!attention_rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn track_stops_when_the_store_stops() {
        let (store_tx, store_rx) = watch::channel(snapshot(vec![]));
        let (tracker, _attention_rx) = AttentionTracker::new(store_rx);
        let task = tokio::spawn(tracker.track());

        drop(store_tx);

        timeout(Duration::from_secs(1), task).await.expect("expected the tracker to stop").unwrap();
    }
}
