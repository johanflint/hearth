use crate::execute_flows::{FlowRunEnvironment, execute_flow, execute_flows};
use crate::flow_engine::VersionedFlow;
use crate::flow_engine::flow::{Flow, SleepMode};
use crate::flow_registry::FlowRegistry;
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub enum SchedulerCommand {
    // The `revision` must match the registry's current revision for this id,
    // otherwise the command is stale (a newer mutation already landed) and is ignored.
    Reconcile { flow_id: String, revision: u64 },
    // An `Arc<Flow>` is passed instead of a flow_id to ensure it resumes with
    // the same flow instance, even if a newer version is available
    ScheduleOnce { flow: Arc<Flow>, sleep_node_id: String, node_id: String, delay: Duration, mode: SleepMode },
}

// Internal way for a job's supervisor to report back on its own private channel
// once the task ends so the map entry can be cleaned up. Contains the generation
// it was spawned with so a retired job finishing late can't evict a newer
// replacment job (ABA problem).
struct JobFinished {
    flow_id: String,
    generation: u64,
}

struct ScheduledJob {
    generation: u64,
    cancellation: CancellationToken,
}

// A healthy schedule only moves forward, an occurrence that isn't later than the latest one means it's looping
const MAX_STALLED_OCCURRENCES: u32 = 100;

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
struct SleepKey {
    flow_id: String,
    sleep_node_id: String,
}
// Sent by a sleeping run once its wait ends (woken or cancelled), so it longer counts as sleeping
struct SleepFinished {
    key: SleepKey,
    generation: u64,
}

#[instrument(skip_all)]
pub async fn scheduler(
    tx: Sender<SchedulerCommand>,
    mut rx: Receiver<SchedulerCommand>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
    flow_registry: Arc<FlowRegistry>,
    preset_registry: Arc<PresetRegistry>,
    environment: FlowRunEnvironment,
) {
    let mut scheduled_flows: HashMap<String, ScheduledJob> = HashMap::new();
    let mut sleeping: HashMap<SleepKey, Vec<ScheduledJob>> = HashMap::new();
    let mut next_generation: u64 = 0;
    // Unbounded: messages are tiny (flow id + generation) and always eventually
    // drained by the same loop, so there's no backpressure benefit to a bound
    // channel, only a risk of asupervisor blocking on `send()` under high reschedule churn.
    let (job_finished_tx, mut job_finished_rx) = mpsc::unbounded_channel::<JobFinished>();
    let (sleep_finished_tx, mut sleep_finished_rx) = mpsc::unbounded_channel::<SleepFinished>();

    loop {
        let cmd = tokio::select! {
            command = rx.recv() => match command {
                Some(cmd) => cmd,
                None => break, // Command channel closed -> shut down
            },
            Some(finished) = job_finished_rx.recv() => {
                evict_if_current_generation(&mut scheduled_flows, &finished.flow_id, finished.generation);
                continue;
            }
            Some(finished) = sleep_finished_rx.recv() => {
                remove_sleeping_job(&mut sleeping, &finished.key, finished.generation);
                continue;
            }
        };

        match cmd {
            SchedulerCommand::Reconcile { flow_id, revision } => {
                let entry = flow_registry.by_id(&flow_id);
                let entry = match reconcile_action(entry.clone(), revision) {
                    ReconcileAction::SkipStaleRevision => {
                        let current = entry.expect("stale revision implies an entry exists").revision;
                        debug!(
                            "🕗 Reconciling flow '{}'... skipped, stale revision {} (current {})",
                            flow_id, revision, current
                        );
                        continue;
                    }
                    ReconcileAction::CancelBecauseFlowRemoved => {
                        let cancelled_job = cancel_existing_job(&flow_id, &mut scheduled_flows);
                        let cancelled_sleeps = cancel_sleeping_jobs(&flow_id, &mut sleeping);
                        if cancelled_job || cancelled_sleeps > 0 {
                            info!(
                                "🕗 Reconciling flow '{}'... cancelled, flow no longer exists ({} sleeping run(s))",
                                flow_id, cancelled_sleeps
                            );
                        }
                        continue;
                    }
                    ReconcileAction::CancelBecauseNotScheduled => {
                        cancel_existing_job(&flow_id, &mut scheduled_flows);
                        info!("🕗 Reconciling flow '{}'... OK, not scheduled", flow_id);
                        continue;
                    }
                    ReconcileAction::CancelAndReschedule => {
                        cancel_existing_job(&flow_id, &mut scheduled_flows);
                        entry.expect("CancelAndReschedule implies an entry exists")
                    }
                };
                let schedule = entry.flow.schedule().expect("CancelAndReschedule implies an entry exists");

                let flow_name = entry.flow.name();
                debug!("🕗 Scheduling flow '{}'...", flow_name);
                let schedule_str = schedule.to_string();

                let generation = next_generation;
                next_generation = next_generation.wrapping_add(1);

                let cancellation = CancellationToken::new();
                let cancellation_clone = cancellation.clone();

                // Job loop
                let flow_id_clone = flow_id.clone();
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                let preset_registry_clone = preset_registry.clone();
                let environment_clone = environment.clone();
                let join_handle = tokio::spawn(async move {
                    let mut occurrences = schedule.after(
                        Utc::now().with_timezone(&environment_clone.timezone),
                        environment_clone.geo_location.clone(),
                    );
                    loop {
                        let now = Utc::now();
                        let Some(datetime) = next_upcoming_occurrence(&mut occurrences, now, &flow_id_clone) else {
                            return;
                        };
                        let duration = datetime.signed_duration_since(now);

                        let scheduled_instant =
                            Instant::now() + Duration::from_millis(duration.num_milliseconds() as u64);

                        if !scheduled_deadline_won(&cancellation, scheduled_instant).await {
                            info!("🕗 Scheduling flow '{}'... cancelled while waiting", flow_id_clone);
                            return;
                        }

                        debug!("🕗 Running scheduled flow '{}'...", flow_id_clone);
                        let snapshot = notifier_rx_clone.borrow().clone();

                        // During execution cancellation is deliberately ignored so a flow in progress always runs to completion
                        execute_flows(
                            vec![entry.flow.clone()],
                            snapshot,
                            preset_registry_clone.clone(),
                            None,
                            tx_clone.clone(),
                            &environment_clone,
                        )
                        .await;

                        if cancellation.is_cancelled() {
                            info!("🕗 Scheduling flow '{}'... cancelled after execution, stopping", flow_id_clone);
                            return;
                        }
                    }
                });

                // Supervise every job so a panic or unexpected completion is observed
                // and the map entry cleaned up
                let job_finished_tx = job_finished_tx.clone();
                let flow_id_for_supervisor = flow_id.clone();
                tokio::spawn(async move {
                    match join_handle.await {
                        Ok(()) => debug!("🕗 Scheduling flow '{}'... finished", flow_id_for_supervisor),
                        Err(e) if e.is_panic() => {
                            error!("🕗🕗 Scheduling flow '{}'... panicked", flow_id_for_supervisor)
                        }
                        Err(e) => warn!("🕗 Scheduling flow '{}'... failed: {}", flow_id_for_supervisor, e),
                    }
                    let _ =
                        job_finished_tx.send(JobFinished { flow_id: flow_id_for_supervisor.to_string(), generation });
                });

                scheduled_flows.insert(flow_id.clone(), ScheduledJob { generation, cancellation: cancellation_clone });
                info!(schedule = schedule_str, "🕗 Scheduling flow '{}'... OK", flow_id);
            }
            SchedulerCommand::ScheduleOnce { flow, sleep_node_id, node_id, delay, mode } => {
                let key = SleepKey { flow_id: flow.id().to_string(), sleep_node_id };
                if !admit_sleep(&mut sleeping, &key, mode) {
                    info!("🕗 Scheduling flow '{}'... ignored, already sleeping", flow.id());
                    continue;
                }

                let generation = next_generation;
                next_generation = next_generation.wrapping_add(1);

                let cancellation = CancellationToken::new();
                let cancellation_clone = cancellation.clone();

                debug!("🕗 Scheduling flow '{}' to run node '{}' after {:?}... OK", flow.id(), node_id, delay);
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                let preset_registry_clone = preset_registry.clone();
                let environment_clone = environment.clone();
                let sleep_finished_tx = sleep_finished_tx.clone();
                let key_clone = key.clone();
                tokio::spawn(async move {
                    let scheduled_instant = Instant::now() + Duration::from_millis(delay.as_millis() as u64);
                    let woken = scheduled_deadline_won(&cancellation, scheduled_instant).await;
                    // Report before executing, so `ignore` doesn't skip triggers while this run finishes
                    let _ = sleep_finished_tx.send(SleepFinished { key: key_clone, generation });
                    if !woken {
                        debug!("🕗 Scheduling flow '{}'... cancelled", flow.id());
                        return;
                    }

                    debug!("🕗 Waking up flow '{}'...", flow.name());
                    let snapshot = notifier_rx_clone.borrow().clone();
                    execute_flow(
                        flow,
                        Some(node_id),
                        snapshot,
                        preset_registry_clone,
                        tx_clone.clone(),
                        &environment_clone,
                    )
                    .await;
                });
                sleeping.entry(key).or_default().push(ScheduledJob { generation, cancellation: cancellation_clone });
            }
        }
    }
}

// Cancels and removes any job for `flow_id`. Returns `true` if one was found.
// Note that this only signals cancellation, it never aborts, so a job currently
// being executed will run to completion.
fn cancel_existing_job(flow_id: &str, jobs: &mut HashMap<String, ScheduledJob>) -> bool {
    let Some(previous_job) = jobs.remove(flow_id) else {
        return false;
    };
    previous_job.cancellation.cancel();
    true
}

// Cancels and removes the sleeping runs of every Sleep node of `flow_id`. Returns how many were cancelled.
// Like `cancel_existing_job`, this only interrupts the wait, a run that has already woken finishes.
fn cancel_sleeping_jobs(flow_id: &str, sleeping: &mut HashMap<SleepKey, Vec<ScheduledJob>>) -> usize {
    let mut cancelled = 0;
    sleeping.retain(|key, jobs| {
        if key.flow_id != flow_id {
            return true;
        }
        for job in jobs.drain(..) {
            job.cancellation.cancel();
            cancelled += 1;
        }
        false
    });
    cancelled
}

// Skips the occurrences that have already passed. Returns None when the schedule has ended or stopped advancing.
fn next_upcoming_occurrence(
    occurrences: &mut impl Iterator<Item = DateTime<Tz>>,
    now: DateTime<Utc>,
    flow_id: &str,
) -> Option<DateTime<Tz>> {
    let mut latest_skipped: Option<DateTime<Tz>> = None;
    let mut stalled_occurrences = 0;
    for datetime in occurrences {
        if datetime >= now {
            return Some(datetime);
        }

        if latest_skipped.is_some_and(|latest| datetime <= latest) {
            stalled_occurrences += 1;
            if stalled_occurrences > MAX_STALLED_OCCURRENCES {
                error!("🕗 Scheduling flow '{}'... schedule stopped advancing, stopping", flow_id);
                return None;
            }
        } else {
            latest_skipped = Some(datetime);
        }
    }
    None
}

// Waits until either `scheduled_instant` or cancellation, whichever comes first.
// Returns `true` only when the deadline wins; a `true` result commits the
// caller to running. A cancellation after this returns does not interrupt an
// already committed execution.
async fn scheduled_deadline_won(cancellation: &CancellationToken, scheduled_instant: Instant) -> bool {
    tokio::select! {
        biased;
        _ = cancellation.cancelled() => false,
        _ = sleep_until(scheduled_instant) => !cancellation.is_cancelled(),
    }
}

fn evict_if_current_generation(scheduled_jobs: &mut HashMap<String, ScheduledJob>, flow_id: &str, generation: u64) {
    if scheduled_jobs.get(flow_id).is_some_and(|job| job.generation == generation) {
        scheduled_jobs.remove(flow_id);
    }
}

fn remove_sleeping_job(sleeping: &mut HashMap<SleepKey, Vec<ScheduledJob>>, key: &SleepKey, generation: u64) {
    let Some(jobs) = sleeping.get_mut(key) else {
        return;
    };
    jobs.retain(|job| job.generation != generation);
    if jobs.is_empty() {
        sleeping.remove(key);
    }
}

// Applies `mode` to the sleeps already pending for `key`. Returns `false` if the new sleep must be dropped.
fn admit_sleep(sleeping: &mut HashMap<SleepKey, Vec<ScheduledJob>>, key: &SleepKey, mode: SleepMode) -> bool {
    match mode {
        SleepMode::Parallel => true,
        SleepMode::Ignore => !sleeping.contains_key(key),
        SleepMode::Restart => {
            for job in sleeping.remove(key).unwrap_or_default() {
                job.cancellation.cancel();
            }
            true
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ReconcileAction {
    SkipStaleRevision,
    CancelBecauseFlowRemoved,
    CancelBecauseNotScheduled,
    CancelAndReschedule,
}

fn reconcile_action(entry: Option<VersionedFlow>, revision: u64) -> ReconcileAction {
    let Some(entry) = entry else {
        return ReconcileAction::CancelBecauseFlowRemoved;
    };
    if entry.revision != revision {
        return ReconcileAction::SkipStaleRevision;
    }
    if entry.flow.schedule().is_none() {
        return ReconcileAction::CancelBecauseNotScheduled;
    }
    ReconcileAction::CancelAndReschedule
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GeoLocation;
    use crate::flow_engine::Schedule;
    use crate::flow_engine::flow::{FlowNode, FlowNodeKind};
    use chrono::TimeZone;
    use rstest::rstest;

    fn versioned_flow(schedule: Option<Schedule>, revision: u64) -> VersionedFlow {
        let start_node = FlowNode::new("start".to_string(), vec![], FlowNodeKind::Start);
        let flow =
            Flow::new("flow".to_string(), "flow".to_string(), schedule, None, Arc::new(start_node), HashMap::new())
                .unwrap();
        VersionedFlow { flow: Arc::new(flow), revision }
    }

    #[test]
    fn next_upcoming_occurrence_skips_passed_occurrences() {
        let at = |hour| Tz::UTC.with_ymd_and_hms(2026, 1, 1, hour, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 11, 30, 0).unwrap();

        let next = next_upcoming_occurrence(&mut [at(10), at(11), at(12), at(13)].into_iter(), now, "flow");

        assert_eq!(next, Some(at(12)));
    }

    #[test]
    fn next_upcoming_occurrence_returns_none_when_the_schedule_has_ended() {
        let now = Utc.with_ymd_and_hms(2026, 1, 1, 11, 30, 0).unwrap();

        assert_eq!(next_upcoming_occurrence(&mut std::iter::empty(), now, "flow"), None);
    }

    #[test]
    fn next_upcoming_occurrence_stops_a_schedule_that_goes_back_in_time() {
        let at = |hour| Tz::UTC.with_ymd_and_hms(2026, 1, 1, hour, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();

        // Never returns without the stall detection, the iterator is endless
        let next = next_upcoming_occurrence(&mut [at(2), at(1)].into_iter().cycle(), now, "flow");

        assert_eq!(next, None);
    }

    // Europe/London leaves BST on 2026-10-25, everything before `now` has passed
    #[rstest]
    #[case("0 0 2 * * *", 2)]
    #[case("0 0 * * * *", 0)]
    fn next_upcoming_occurrence_passes_the_end_of_daylight_saving_time(#[case] expression: &str, #[case] hour: u32) {
        let london = Tz::Europe__London;
        let from = london.with_ymd_and_hms(2026, 10, 24, 12, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 10, 27, 0, 0, 0).unwrap();
        let mut occurrences = Schedule::Cron(expression.to_string()).after(from, GeoLocation::default());

        let next = next_upcoming_occurrence(&mut occurrences, now, "flow");

        assert_eq!(next, Some(london.with_ymd_and_hms(2026, 10, 27, hour, 0, 0).unwrap()));
    }

    #[test]
    fn reconcile_action_cancels_when_the_flow_was_removed() {
        assert_eq!(reconcile_action(None, 0), ReconcileAction::CancelBecauseFlowRemoved);
    }

    #[test]
    fn reconcile_action_skips_a_stale_revision() {
        // The core race-fix guard: a command carrying an outdated revision must
        // never act, even though an entry still exists for the flow.
        let entry = versioned_flow(None, 2);
        assert_eq!(reconcile_action(Some(entry), 1), ReconcileAction::SkipStaleRevision);
    }

    #[test]
    fn reconcile_action_cancels_when_the_flow_has_no_schedule() {
        let entry = versioned_flow(None, 0);
        assert_eq!(reconcile_action(Some(entry), 0), ReconcileAction::CancelBecauseNotScheduled);
    }

    #[test]
    fn reconcile_action_cancels_and_reschedules_when_matching_and_scheduled() {
        let entry = versioned_flow(Some(Schedule::Cron("* * * * * *".to_string())), 0);
        assert_eq!(reconcile_action(Some(entry), 0), ReconcileAction::CancelAndReschedule);
    }

    #[test]
    fn cancel_existing_job_removes_and_cancels_when_present() {
        let mut scheduled_jobs = HashMap::new();
        let cancellation = CancellationToken::new();
        scheduled_jobs.insert("flow".to_string(), ScheduledJob { generation: 0, cancellation: cancellation.clone() });

        assert!(cancel_existing_job("flow", &mut scheduled_jobs));
        assert!(cancellation.is_cancelled());
        assert!(!scheduled_jobs.contains_key("flow"));
    }

    #[test]
    fn cancel_existing_jobs_does_nothing_if_flow_is_absent() {
        let mut scheduled_jobs = HashMap::new();
        assert!(!cancel_existing_job("flow", &mut scheduled_jobs));
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_deadline_won_returns_false_when_cancelled_before_wait() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let deadline = Instant::now() + Duration::from_secs(60);

        assert!(!scheduled_deadline_won(&cancellation, deadline).await);
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_deadline_won_returns_false_when_cancelled_while_waiting() {
        let cancellation = CancellationToken::new();
        let waiting = cancellation.clone();
        let deadline = Instant::now() + Duration::from_secs(60);

        let waiter = tokio::spawn(async move { scheduled_deadline_won(&waiting, deadline).await });
        tokio::task::yield_now().await;
        cancellation.cancel();

        assert!(!waiter.await.expect("wait task should not panic"));
    }

    #[tokio::test(start_paused = true)]
    async fn scheduled_deadline_won_returns_true_when_deadline_is_reached() {
        let cancellation = CancellationToken::new();
        let deadline = Instant::now() + Duration::from_secs(60);

        let waiter = tokio::spawn({
            let cancellation = cancellation.clone();
            async move { scheduled_deadline_won(&cancellation, deadline).await }
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(60)).await;

        assert!(waiter.await.expect("wait task should not panic"));
    }

    #[test]
    fn evict_if_current_generation_removes_if_the_generation_matches() {
        let mut scheduled_jobs = HashMap::new();
        let cancellation = CancellationToken::new();
        scheduled_jobs.insert("flow".to_string(), ScheduledJob { generation: 2, cancellation });

        evict_if_current_generation(&mut scheduled_jobs, "flow", 2);

        assert!(!scheduled_jobs.contains_key("flow"));
    }

    #[test]
    fn evict_if_current_generation_ignores_stale_generation() {
        // A retired job (generation 1) finishing late must not evict its
        // replacement (generation 2), which is a classic ABA hazard (https://en.wikipedia.org/wiki/ABA_problem).
        let mut scheduled_jobs = HashMap::new();
        let cancellation = CancellationToken::new();
        scheduled_jobs.insert("flow".to_string(), ScheduledJob { generation: 2, cancellation });

        evict_if_current_generation(&mut scheduled_jobs, "flow", 1);

        assert!(scheduled_jobs.contains_key("flow"));
    }

    fn sleep_key(sleep_node_id: &str) -> SleepKey {
        SleepKey { flow_id: "flow".to_string(), sleep_node_id: sleep_node_id.to_string() }
    }

    fn sleeping_job(generation: u64) -> ScheduledJob {
        ScheduledJob { generation, cancellation: CancellationToken::new() }
    }

    fn generations(sleeping: &HashMap<SleepKey, Vec<ScheduledJob>>, key: &SleepKey) -> Vec<u64> {
        sleeping.get(key).map(|jobs| jobs.iter().map(|job| job.generation).collect()).unwrap_or_default()
    }

    #[test]
    fn remove_sleeping_job_removes_only_the_matching_generation() {
        let key = sleep_key("sleepNode");
        let mut sleeping = HashMap::from([(key.clone(), vec![sleeping_job(1), sleeping_job(2), sleeping_job(3)])]);

        remove_sleeping_job(&mut sleeping, &key, 2);

        assert_eq!(generations(&sleeping, &key), vec![1, 3]);
    }

    #[test]
    fn remove_sleeping_job_removes_the_key_once_nothing_sleeps() {
        // `ignore` checks for the key, so an empty list must not linger
        let key = sleep_key("sleepNode");
        let mut sleeping = HashMap::from([(key.clone(), vec![sleeping_job(1)])]);

        remove_sleeping_job(&mut sleeping, &key, 1);

        assert!(!sleeping.contains_key(&key));
    }

    #[test]
    fn remove_sleeping_job_ignores_a_stale_generation() {
        // A cancelled sleep (generation 1) reporting late must not remove the sleep that replaced it (generation 2)
        let key = sleep_key("sleepNode");
        let mut sleeping = HashMap::from([(key.clone(), vec![sleeping_job(2)])]);

        remove_sleeping_job(&mut sleeping, &key, 1);

        assert_eq!(generations(&sleeping, &key), vec![2]);
    }

    #[test]
    fn remove_sleeping_job_only_affects_its_own_sleep_node() {
        let key = sleep_key("sleepNode");
        let other = sleep_key("otherSleepNode");
        let mut sleeping =
            HashMap::from([(key.clone(), vec![sleeping_job(1)]), (other.clone(), vec![sleeping_job(1)])]);

        remove_sleeping_job(&mut sleeping, &key, 1);

        assert!(!sleeping.contains_key(&key));
        assert_eq!(generations(&sleeping, &other), vec![1]);
    }

    #[test]
    fn remove_sleeping_job_does_nothing_for_an_unknown_key() {
        let mut sleeping = HashMap::from([(sleep_key("sleepNode"), vec![sleeping_job(1)])]);

        remove_sleeping_job(&mut sleeping, &sleep_key("unknown"), 1);

        assert_eq!(generations(&sleeping, &sleep_key("sleepNode")), vec![1]);
    }

    #[rstest]
    #[case::parallel(SleepMode::Parallel)]
    #[case::ignore(SleepMode::Ignore)]
    #[case::restart(SleepMode::Restart)]
    fn admit_sleep_admits_when_nothing_sleeps(#[case] mode: SleepMode) {
        let mut sleeping = HashMap::new();

        assert!(admit_sleep(&mut sleeping, &sleep_key("sleepNode"), mode));
    }

    #[test]
    fn admit_sleep_parallel_keeps_existing_sleeps() {
        let key = sleep_key("sleepNode");
        let existing = sleeping_job(1);
        let cancellation = existing.cancellation.clone();
        let mut sleeping = HashMap::from([(key.clone(), vec![existing])]);

        assert!(admit_sleep(&mut sleeping, &key, SleepMode::Parallel));

        assert_eq!(generations(&sleeping, &key), vec![1]);
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn admit_sleep_ignore_rejects_while_sleeping() {
        let key = sleep_key("sleepNode");
        let existing = sleeping_job(1);
        let cancellation = existing.cancellation.clone();
        let mut sleeping = HashMap::from([(key.clone(), vec![existing])]);

        assert!(!admit_sleep(&mut sleeping, &key, SleepMode::Ignore));

        assert_eq!(generations(&sleeping, &key), vec![1]);
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn admit_sleep_ignore_admits_when_another_sleep_node_sleeps() {
        let mut sleeping = HashMap::from([(sleep_key("otherSleepNode"), vec![sleeping_job(1)])]);

        assert!(admit_sleep(&mut sleeping, &sleep_key("sleepNode"), SleepMode::Ignore));
    }

    #[test]
    fn admit_sleep_restart_cancels_and_removes_all_existing_sleeps() {
        let key = sleep_key("sleepNode");
        let jobs = vec![sleeping_job(1), sleeping_job(2)];
        let cancellations: Vec<_> = jobs.iter().map(|job| job.cancellation.clone()).collect();
        let mut sleeping = HashMap::from([(key.clone(), jobs)]);

        assert!(admit_sleep(&mut sleeping, &key, SleepMode::Restart));

        assert!(!sleeping.contains_key(&key));
        assert!(cancellations.iter().all(CancellationToken::is_cancelled));
    }

    #[test]
    fn admit_sleep_restart_leaves_other_sleep_nodes_alone() {
        let key = sleep_key("sleepNode");
        let other = sleep_key("otherSleepNode");
        let other_job = sleeping_job(1);
        let other_cancellation = other_job.cancellation.clone();
        let mut sleeping = HashMap::from([(key.clone(), vec![sleeping_job(2)]), (other.clone(), vec![other_job])]);

        assert!(admit_sleep(&mut sleeping, &key, SleepMode::Restart));

        assert_eq!(generations(&sleeping, &other), vec![1]);
        assert!(!other_cancellation.is_cancelled());
    }

    #[test]
    fn cancel_sleeping_jobs_cancels_and_removes_the_sleeps_of_every_sleep_node() {
        let first = sleep_key("sleepNode");
        let second = sleep_key("otherSleepNode");
        let jobs = vec![sleeping_job(1), sleeping_job(2)];
        let other_jobs = vec![sleeping_job(3)];
        let cancellations: Vec<_> = jobs.iter().chain(&other_jobs).map(|job| job.cancellation.clone()).collect();
        let mut sleeping = HashMap::from([(first, jobs), (second, other_jobs)]);

        assert_eq!(cancel_sleeping_jobs("flow", &mut sleeping), 3);

        assert!(sleeping.is_empty());
        assert!(cancellations.iter().all(CancellationToken::is_cancelled));
    }

    #[test]
    fn cancel_sleeping_jobs_leaves_other_flows_alone() {
        // Another flow can use the same sleep node id
        let key = sleep_key("sleepNode");
        let other_flow = SleepKey { flow_id: "otherFlow".to_string(), sleep_node_id: "sleepNode".to_string() };
        let other_job = sleeping_job(2);
        let other_cancellation = other_job.cancellation.clone();
        let mut sleeping = HashMap::from([(key, vec![sleeping_job(1)]), (other_flow.clone(), vec![other_job])]);

        assert_eq!(cancel_sleeping_jobs("flow", &mut sleeping), 1);

        assert_eq!(sleeping.len(), 1);
        assert_eq!(generations(&sleeping, &other_flow), vec![2]);
        assert!(!other_cancellation.is_cancelled());
    }

    #[test]
    fn cancel_sleeping_jobs_does_nothing_for_an_unknown_flow() {
        let key = sleep_key("sleepNode");
        let job = sleeping_job(1);
        let cancellation = job.cancellation.clone();
        let mut sleeping = HashMap::from([(key.clone(), vec![job])]);

        assert_eq!(cancel_sleeping_jobs("unknown", &mut sleeping), 0);

        assert_eq!(generations(&sleeping, &key), vec![1]);
        assert!(!cancellation.is_cancelled());
    }

    #[test]
    fn remove_sleeping_job_ignores_a_sleep_cancelled_with_its_flow() {
        // A cancelled run still reports back after its flow was removed, which must not fail or resurrect the key
        let key = sleep_key("sleepNode");
        let mut sleeping = HashMap::from([(key.clone(), vec![sleeping_job(1)])]);
        cancel_sleeping_jobs("flow", &mut sleeping);

        remove_sleeping_job(&mut sleeping, &key, 1);

        assert!(sleeping.is_empty());
    }
}
