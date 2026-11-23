use crate::attention::{Attention, AttentionList, FlowRun, FlowRunReporter, Issue, Subject};
use crate::domain::property::{EnumProperty, PropertyType};
use crate::domain::{BatteryState, Connectivity, Device};
use crate::integration::{Integration, IntegrationStatus};
use crate::store::StoreSnapshot;
use chrono::{DateTime, Utc};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::sync::{mpsc, watch};
use tokio::time::{MissedTickBehavior, interval};

const INTEGRATION_POLL_INTERVAL: Duration = Duration::from_secs(5);

pub struct AttentionTracker {
    store_rx: WatchReceiver<StoreSnapshot>,
    flow_run_rx: mpsc::Receiver<FlowRun>,
    integrations: Arc<[Arc<dyn Integration>]>,
    attention_tx: watch::Sender<AttentionList>,
}

impl AttentionTracker {
    pub fn new(
        store_rx: WatchReceiver<StoreSnapshot>,
        integrations: Arc<[Arc<dyn Integration>]>,
    ) -> (Self, watch::Receiver<AttentionList>, FlowRunReporter) {
        let (attention_tx, attention_rx) = watch::channel(AttentionList::default());
        let (flow_run_tx, flow_run_rx) = mpsc::channel::<FlowRun>(256);
        let flow_run_reporter = FlowRunReporter::new(flow_run_tx);

        (AttentionTracker { store_rx, flow_run_rx, integrations, attention_tx }, attention_rx, flow_run_reporter)
    }

    pub async fn track(mut self) {
        let mut poll = interval(INTEGRATION_POLL_INTERVAL);
        poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
        self.store_rx.mark_changed();

        loop {
            tokio::select! {
                changed = self.store_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    let snapshot = self.store_rx.borrow_and_update().clone();
                    // The published list is the previous state, so there is nothing else to keep track of
                    self.publish(|list| &mut list.devices, |current, now| attention_for_devices(&snapshot, current, now));
                }
                Some(run) = self.flow_run_rx.recv() => {
                    self.publish(
                        |list| &mut list.flows,
                        |current, now| attention_for_flow(run, current, now)
                    );
                }
                _ = poll.tick() => {
                    self.publish(
                        |list| &mut list.integrations,
                        |current, now| attention_for_integrations(self.integrations.as_ref(), current, now)
                    );
                }
            }
        }
    }

    /// Replaces one section of the list, the other sections are left alone
    fn publish(
        &self,
        section: impl FnOnce(&mut AttentionList) -> &mut Arc<[Attention]>,
        update: impl FnOnce(&[Attention], DateTime<Utc>) -> Vec<Attention>,
    ) {
        self.attention_tx.send_if_modified(|current| {
            let section = section(current);
            let next = update(section, Utc::now());

            if next == **section {
                return false;
            }
            *section = next.into();
            true
        });
    }
}

fn attention_for_devices(snapshot: &StoreSnapshot, previous: &[Attention], now: DateTime<Utc>) -> Vec<Attention> {
    let mut attention: Vec<_> = snapshot
        .devices
        .values()
        .flat_map(|device| {
            device_issues(device).into_iter().map(|issue| {
                let subject = Subject::Device(device.id.clone());
                let since = since_of(previous, &subject, &issue, now);
                Attention { subject, issue, since }
            })
        })
        .collect();
    // A HashMap has no order, without sorting every update looks like a change
    attention.sort_by(|a, b| a.subject.cmp(&b.subject));
    attention
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

fn attention_for_flow(run: FlowRun, previous: &[Attention], now: DateTime<Utc>) -> Vec<Attention> {
    let (flow_id, issue) = match run {
        FlowRun::Succeeded { flow_id } => (flow_id, None),
        FlowRun::Failed { flow_id, error } => (flow_id, Some(Issue::FlowFailed { error: error.to_string() })),
    };
    let subject = Subject::Flow(flow_id.clone());
    let since = issue.as_ref().map(|issue| since_of(previous, &subject, issue, now));
    let mut next: Vec<_> = previous.iter().filter(|a| a.subject != subject).cloned().collect();
    if let (Some(issue), Some(since)) = (issue, since) {
        next.push(Attention { subject, issue, since });
    }
    next.sort_by(|a, b| a.subject.cmp(&b.subject));
    next
}

fn attention_for_integrations(
    integrations: &[Arc<dyn Integration>],
    previous: &[Attention],
    now: DateTime<Utc>,
) -> Vec<Attention> {
    integrations
        .iter()
        .map(|integration| (integration.name(), integration.status()))
        .filter(|(_, status)| matches!(status, IntegrationStatus::Degraded | IntegrationStatus::Down))
        .map(|(integration_name, status)| {
            let subject = Subject::Integration(integration_name.to_string());
            let issue = Issue::Integration(status);
            let since = since_of(previous, &subject, &issue, now);
            Attention { subject, issue, since }
        })
        .collect()
}

// An issue that was already there keeps its `since`, also when its level changes (Low -> Critical)
fn since_of(previous: &[Attention], subject: &Subject, issue: &Issue, now: DateTime<Utc>) -> DateTime<Utc> {
    previous.iter().find(|p| &p.subject == subject && p.issue.is_same_kind(issue)).map_or(now, |p| p.since)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::property::Property;
    use crate::domain::{Command, Controller, Event};
    use crate::flow_engine::FlowEngineError;
    use crate::integration::{Discovery, IntegrationError, SharedStatus};
    use crate::test_support::DeviceBuilder;
    use async_trait::async_trait;
    use chrono::TimeZone;
    use rstest::rstest;
    use std::collections::HashMap;
    use std::slice::from_ref;
    use strum::IntoEnumIterator;
    use tokio::sync::mpsc::Sender;
    use tokio::time::timeout;

    #[derive(Debug)]
    struct StubController;

    #[async_trait]
    impl Controller for StubController {
        fn id(&self) -> &'static str {
            "stub"
        }

        async fn execute(&self, _command: Command) {}
    }

    #[derive(Debug)]
    struct StubIntegration {
        name: &'static str,
        status: SharedStatus,
    }

    #[async_trait]
    impl Integration for StubIntegration {
        fn name(&self) -> &'static str {
            self.name
        }

        fn enabled(&self) -> bool {
            true
        }

        async fn discover(&self) -> Result<Discovery, IntegrationError> {
            unimplemented!()
        }

        async fn observe(&self, _tx: Sender<Event>) -> Result<(), IntegrationError> {
            unimplemented!()
        }

        fn controller(&self) -> Arc<dyn Controller> {
            Arc::new(StubController)
        }

        fn status(&self) -> IntegrationStatus {
            self.status.get()
        }
    }

    /// The integration and the handle to change its status
    fn stub(name: &'static str, status: IntegrationStatus) -> (Arc<dyn Integration>, SharedStatus) {
        let status = SharedStatus::new(status);
        (Arc::new(StubIntegration { name, status: status.clone() }), status)
    }

    fn integration_subject(name: &str) -> Subject {
        Subject::Integration(name.into())
    }

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

    #[rstest]
    #[case::up(IntegrationStatus::Up, vec![])]
    #[case::degraded(IntegrationStatus::Degraded, vec![Issue::Integration(IntegrationStatus::Degraded)])]
    #[case::down(IntegrationStatus::Down, vec![Issue::Integration(IntegrationStatus::Down)])]
    fn attention_for_integrations_reports_integrations_that_are_not_up(
        #[case] status: IntegrationStatus,
        #[case] expected: Vec<Issue>,
    ) {
        let (hue, _) = stub("hue", status);

        let attention = attention_for_integrations(&[hue], &[], at(0));

        assert_eq!(attention.into_iter().map(|a| a.issue).collect::<Vec<_>>(), expected);
    }

    #[test]
    fn attention_for_integrations_starts_new_issues_now() {
        let (hue, _) = stub("hue", IntegrationStatus::Down);

        let attention = attention_for_integrations(&[hue], &[], at(5));

        assert_eq!(
            attention,
            [Attention {
                subject: integration_subject("hue"),
                issue: Issue::Integration(IntegrationStatus::Down),
                since: at(5)
            }]
        );
    }

    #[test]
    fn attention_for_integrations_keeps_since_when_the_status_changes() {
        let (hue, status) = stub("hue", IntegrationStatus::Down);
        let first = attention_for_integrations(from_ref(&hue), &[], at(0));

        status.set(IntegrationStatus::Degraded);
        let second = attention_for_integrations(&[hue], &first, at(5));

        assert_eq!(second[0].issue, Issue::Integration(IntegrationStatus::Degraded));
        assert_eq!(second[0].since, at(0));
    }

    #[test]
    fn attention_for_integrations_starts_over_when_an_issue_returns() {
        let (hue, status) = stub("hue", IntegrationStatus::Down);
        let first = attention_for_integrations(from_ref(&hue), &[], at(0));
        status.set(IntegrationStatus::Up);
        let resolved = attention_for_integrations(from_ref(&hue), &first, at(5));
        assert!(resolved.is_empty());

        status.set(IntegrationStatus::Down);
        let again = attention_for_integrations(&[hue], &resolved, at(10));

        assert_eq!(again[0].since, at(10));
    }

    #[test]
    fn attention_for_integrations_does_not_mix_up_integrations() {
        let (hue, _) = stub("hue", IntegrationStatus::Down);
        let first = attention_for_integrations(&[hue], &[], at(0));

        let (shelly, _) = stub("shelly", IntegrationStatus::Down);
        let second = attention_for_integrations(&[shelly], &first, at(5));

        assert_eq!(second[0].subject, integration_subject("shelly"));
        assert_eq!(second[0].since, at(5));
    }

    fn failed(flow_id: &str, error: &str) -> FlowRun {
        FlowRun::Failed { flow_id: flow_id.into(), error: FlowEngineError::MissingProvidedStartNode(error.into()) }
    }

    fn succeeded(flow_id: &str) -> FlowRun {
        FlowRun::Succeeded { flow_id: flow_id.into() }
    }

    fn flow_failure(flow_id: &str, error: &str, since: DateTime<Utc>) -> Attention {
        Attention {
            subject: Subject::Flow(flow_id.into()),
            issue: Issue::FlowFailed { error: format!("missing provided start node '{error}'") },
            since,
        }
    }

    #[test]
    fn attention_for_flow_starts_a_failure_now() {
        let attention = attention_for_flow(failed("a", "x"), &[], at(5));

        assert_eq!(attention, [flow_failure("a", "x", at(5))]);
    }

    #[test]
    fn attention_for_flow_keeps_since_and_takes_the_new_error_of_a_repeated_failure() {
        let first = attention_for_flow(failed("a", "x"), &[], at(0));

        let second = attention_for_flow(failed("a", "y"), &first, at(5));

        assert_eq!(second, [flow_failure("a", "y", at(0))]);
    }

    #[test]
    fn attention_for_flow_removes_the_failure_of_a_flow_that_succeeds() {
        let first = attention_for_flow(failed("a", "x"), &[], at(0));

        assert!(attention_for_flow(succeeded("a"), &first, at(5)).is_empty());
    }

    #[test]
    fn attention_for_flow_leaves_other_flows_alone_and_sorts_by_subject() {
        let previous = [flow_failure("a", "x", at(0)), flow_failure("c", "x", at(1))];

        let after_failure = attention_for_flow(failed("b", "x"), &previous, at(5));
        let after_success = attention_for_flow(succeeded("a"), &after_failure, at(6));

        assert_eq!(
            subjects(&after_failure),
            [&Subject::Flow("a".into()), &Subject::Flow("b".into()), &Subject::Flow("c".into())]
        );
        assert_eq!(after_success, [flow_failure("b", "x", at(5)), flow_failure("c", "x", at(1))]);
    }

    #[test]
    fn attention_for_flow_ignores_the_success_of_a_flow_that_did_not_fail() {
        let previous = [flow_failure("a", "x", at(0))];

        assert_eq!(attention_for_flow(succeeded("b"), &previous, at(5)), previous);
    }

    #[tokio::test]
    async fn track_publishes_failed_flows_and_clears_them_when_they_succeed() {
        let (store_tx, store_rx) = watch::channel(snapshot(vec![]));
        let (tracker, mut attention_rx, reporter) = AttentionTracker::new(store_rx, Arc::from([]));
        tokio::spawn(tracker.track());

        reporter.failed("a", FlowEngineError::MissingOutgoingNode("n".into()));
        attention_rx.wait_for(|list| !list.flows.is_empty()).await.unwrap();
        assert_eq!(
            attention_rx.borrow().flows[0].issue,
            Issue::FlowFailed { error: "missing outgoing node for node 'n'".into() }
        );

        reporter.succeeded("a");
        attention_rx.wait_for(|list| list.flows.is_empty()).await.unwrap();
        drop(store_tx);
    }

    fn new_tracker() -> (watch::Sender<StoreSnapshot>, AttentionTracker, watch::Receiver<AttentionList>) {
        let (store_tx, store_rx) = watch::channel(snapshot(vec![]));
        let (tracker, attention_rx, _) = AttentionTracker::new(store_rx, Arc::from([]));
        (store_tx, tracker, attention_rx)
    }

    fn attention_at_0(subject: Subject, issue: Issue) -> Attention {
        Attention { subject, issue, since: at(0) }
    }

    #[test]
    fn publish_replaces_only_its_own_section() {
        let (_store_tx, tracker, attention_rx) = new_tracker();
        tracker.publish(
            |list| &mut list.devices,
            |_, _| vec![attention_at_0(Subject::Device("1".into()), Issue::Battery(BatteryState::Low))],
        );
        tracker.publish(
            |list| &mut list.integrations,
            |_, _| vec![attention_at_0(integration_subject("hue"), Issue::Integration(IntegrationStatus::Down))],
        );

        tracker.publish(|list| &mut list.integrations, |_, _| vec![]);

        let list = attention_rx.borrow();
        assert_eq!(subjects(&list.devices), [&Subject::Device("1".into())]);
        assert!(list.integrations.is_empty());
    }

    #[test]
    fn publish_hands_the_current_section_to_the_update() {
        let (_store_tx, tracker, _attention_rx) = new_tracker();
        tracker.publish(
            |list| &mut list.devices,
            |_, _| vec![attention_at_0(Subject::Device("1".into()), Issue::Battery(BatteryState::Low))],
        );

        let mut previous_len = 0;
        tracker.publish(
            |list| &mut list.devices,
            |previous, _| {
                previous_len = previous.len();
                vec![]
            },
        );

        assert_eq!(previous_len, 1);
    }

    #[test]
    fn publish_does_not_notify_when_the_section_is_unchanged() {
        let (_store_tx, tracker, mut attention_rx) = new_tracker();
        let update =
            |_: &[Attention], _| vec![attention_at_0(Subject::Device("1".into()), Issue::Battery(BatteryState::Low))];
        tracker.publish(|list| &mut list.devices, update);
        assert!(attention_rx.has_changed().unwrap());
        attention_rx.borrow_and_update();

        tracker.publish(|list| &mut list.devices, update);

        assert!(!attention_rx.has_changed().unwrap());
    }

    #[test]
    fn is_same_kind_ignores_the_level() {
        let low = Issue::Battery(BatteryState::Low);

        assert!(low.is_same_kind(&Issue::Battery(BatteryState::Critical)));
        assert!(!low.is_same_kind(&Issue::Connectivity(Connectivity::Disconnected)));
    }

    fn start_tracker(
        devices: Vec<Device>,
        integrations: Vec<Arc<dyn Integration>>,
    ) -> (watch::Sender<StoreSnapshot>, watch::Receiver<AttentionList>) {
        let (store_tx, store_rx) = watch::channel(snapshot(devices));
        let (tracker, attention_rx, _) = AttentionTracker::new(store_rx, integrations.into());
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
        let (_store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)], vec![]);

        attention_rx.changed().await.unwrap();

        assert_eq!(subjects(&attention_rx.borrow().devices), [&Subject::Device("1".into())]);
    }

    #[tokio::test]
    async fn track_publishes_new_issues() {
        let (store_tx, mut attention_rx) = start_tracker(vec![], vec![]);
        let_tracker_process().await;

        store_tx.send(snapshot(vec![battery_device("1", BatteryState::Critical)])).unwrap();

        attention_rx.changed().await.unwrap();
        assert_eq!(attention_rx.borrow().devices[0].issue, Issue::Battery(BatteryState::Critical));
    }

    #[tokio::test]
    async fn track_publishes_when_an_issue_is_resolved() {
        let (store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)], vec![]);
        attention_rx.changed().await.unwrap();

        store_tx.send(snapshot(vec![battery_device("1", BatteryState::Normal)])).unwrap();

        attention_rx.changed().await.unwrap();
        assert!(attention_rx.borrow().devices.is_empty());
    }

    #[tokio::test]
    async fn track_does_not_publish_changes_that_need_no_attention() {
        let (store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)], vec![]);
        attention_rx.changed().await.unwrap();

        store_tx
            .send(snapshot(vec![battery_device("1", BatteryState::Low), battery_device("2", BatteryState::Normal)]))
            .unwrap();
        let_tracker_process().await;

        assert!(!attention_rx.has_changed().unwrap());
    }

    #[tokio::test]
    async fn track_publishes_integrations_that_are_down_at_startup() {
        let (hue, _) = stub("hue", IntegrationStatus::Down);
        let (_store_tx, mut attention_rx) = start_tracker(vec![], vec![hue]);

        attention_rx.changed().await.unwrap();

        assert_eq!(subjects(&attention_rx.borrow().integrations), [&integration_subject("hue")]);
    }

    #[tokio::test(start_paused = true)]
    async fn track_publishes_when_an_integration_recovers() {
        let (hue, status) = stub("hue", IntegrationStatus::Down);
        let (_store_tx, mut attention_rx) = start_tracker(vec![], vec![hue]);
        attention_rx.changed().await.unwrap();

        status.set(IntegrationStatus::Up);

        attention_rx.changed().await.unwrap();
        assert!(attention_rx.borrow().integrations.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn track_keeps_the_device_issues_when_an_integration_recovers() {
        let (hue, status) = stub("hue", IntegrationStatus::Down);
        let (_store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)], vec![hue]);
        attention_rx.wait_for(|list| list.iter().count() == 2).await.unwrap();

        status.set(IntegrationStatus::Up);

        attention_rx.wait_for(|list| list.integrations.is_empty()).await.unwrap();
        assert_eq!(subjects(&attention_rx.borrow().devices), [&Subject::Device("1".into())]);
    }

    #[tokio::test(start_paused = true)]
    async fn track_keeps_the_integration_issues_when_a_device_recovers() {
        let (hue, _) = stub("hue", IntegrationStatus::Down);
        let (store_tx, mut attention_rx) = start_tracker(vec![battery_device("1", BatteryState::Low)], vec![hue]);
        attention_rx.wait_for(|list| list.iter().count() == 2).await.unwrap();

        store_tx.send(snapshot(vec![battery_device("1", BatteryState::Normal)])).unwrap();

        attention_rx.wait_for(|list| list.devices.is_empty()).await.unwrap();
        assert_eq!(subjects(&attention_rx.borrow().integrations), [&integration_subject("hue")]);
    }

    #[tokio::test]
    async fn track_stops_when_the_store_stops() {
        let (store_tx, store_rx) = watch::channel(snapshot(vec![]));
        let (tracker, _attention_rx, _) = AttentionTracker::new(store_rx, Arc::from([]));
        let task = tokio::spawn(tracker.track());

        drop(store_tx);

        timeout(Duration::from_secs(1), task).await.expect("expected the tracker to stop").unwrap();
    }
}
