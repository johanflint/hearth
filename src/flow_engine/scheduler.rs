use crate::domain::GeoLocation;
use crate::execute_flows::{execute_flow, execute_flows};
use crate::flow_engine::flow::Flow;
use crate::flow_registry::{FlowRegistry, RegistryEntry};
use crate::store::StoreSnapshot;
use chrono::Local;
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
    ScheduleOnce { flow: Arc<Flow>, node_id: String, delay: Duration },
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

#[instrument(skip_all)]
pub async fn scheduler(
    tx: Sender<SchedulerCommand>,
    mut rx: Receiver<SchedulerCommand>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
    flow_registry: Arc<FlowRegistry>,
    geo_location: GeoLocation,
) {
    let mut scheduled_flows: HashMap<String, ScheduledJob> = HashMap::new();
    let mut next_generation: u64 = 0;
    // Unbounded: messages are tiny (flow id + generation) and always eventually
    // drained by the same loop, so there's no backpressure benefit to a bound
    // channel, only a risk of asupervisor blocking on `send()` under high reschedule churn.
    let (job_finished_tx, mut job_finished_rx) = mpsc::unbounded_channel::<JobFinished>();

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
        };

        match cmd {
            SchedulerCommand::Reconcile { flow_id, revision } => {
                let entry = flow_registry.by_id(&flow_id);
                let entry = match reconcile_action(entry.as_ref(), revision) {
                    ReconcileAction::SkipStaleRevision => {
                        let current = entry.expect("stale revision implies an entry exists").revision;
                        debug!("🕗 Reconciling flow '{}'... skipped, stale revision {} (current {})", flow_id, revision, current);
                        continue;
                    }
                    ReconcileAction::CancelBecauseFlowRemoved => {
                        if cancel_existing_job(&flow_id, &mut scheduled_flows) {
                            info!("🕗 Reconciling flow '{}'... cancelled, flow no longer exists", flow_id);
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
                let geo_location_clone = geo_location.clone();
                let join_handle = tokio::spawn(async move {
                    for datetime in schedule.upcoming(Local, geo_location_clone.clone()) {
                        let duration = datetime.signed_duration_since(Local::now());
                        if duration.num_milliseconds() < 0 {
                            continue; // Already passed
                        }

                        let scheduled_instant = Instant::now() + Duration::from_millis(duration.num_milliseconds() as u64);

                        if !scheduled_deadline_won(&cancellation, scheduled_instant).await {
                            info!("🕗 Scheduling flow '{}'... cancelled while waiting", flow_id_clone);
                            return;
                        }

                        debug!("🕗 Running scheduled flow '{}'...", flow_id_clone);
                        let snapshot = notifier_rx_clone.borrow().clone();

                        // During execution cancellation is deliberately ignored so a flow in progress always runs to completion
                        execute_flows(vec![entry.flow.clone()], snapshot, None, tx_clone.clone(), geo_location_clone.clone()).await;

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
                        Err(e) if e.is_panic() => error!("🕗🕗 Scheduling flow '{}'... panicked", flow_id_for_supervisor),
                        Err(e) => warn!("🕗 Scheduling flow '{}'... failed: {}", flow_id_for_supervisor, e),
                    }
                    let _ = job_finished_tx.send(JobFinished { flow_id: flow_id_for_supervisor.to_string(), generation });
                });

                scheduled_flows.insert(flow_id.clone(), ScheduledJob { generation, cancellation: cancellation_clone });
                info!(schedule = schedule_str, "🕗 Scheduling flow '{}'... OK", flow_id);
            }
            SchedulerCommand::ScheduleOnce { flow, node_id, delay } => {
                debug!("🕗 Scheduling flow '{}' to run node '{}' after {:?}... OK", flow.id(), node_id, delay);
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                let geo_location_clone = geo_location.clone();
                tokio::spawn(async move {
                    let scheduled_instant = Instant::now() + Duration::from_millis(delay.as_millis() as u64);
                    sleep_until(scheduled_instant).await;

                    debug!("🕗 Waking up flow '{}'...", flow.name());
                    let snapshot = notifier_rx_clone.borrow().clone();
                    execute_flow(flow, Some(node_id), snapshot, tx_clone.clone(), geo_location_clone.clone()).await;
                });
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

// Waits until either `scheduled_instant` or cancellation, whichever comes first.
// Returns `true` only when the deadline wins; a `true` result coommits the
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

#[derive(Debug, PartialEq, Eq)]
enum ReconcileAction {
    SkipStaleRevision,
    CancelBecauseFlowRemoved,
    CancelBecauseNotScheduled,
    CancelAndReschedule,
}

fn reconcile_action(entry: Option<&RegistryEntry>, revision: u64) -> ReconcileAction {
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
    use crate::flow_engine::Schedule;
    use crate::flow_engine::flow::{FlowNode, FlowNodeKind};

    fn registry_entry(schedule: Option<Schedule>, revision: u64) -> RegistryEntry {
        let start_node = FlowNode::new("start".to_string(), vec![], FlowNodeKind::Start);
        let flow = Flow::new("flow".to_string(), "flow".to_string(), schedule, None, Arc::new(start_node), HashMap::new()).unwrap();
        RegistryEntry { flow: Arc::new(flow), revision }
    }

    #[test]
    fn reconcile_action_cancels_when_the_flow_was_removed() {
        assert_eq!(reconcile_action(None, 0), ReconcileAction::CancelBecauseFlowRemoved);
    }

    #[test]
    fn reconcile_action_skips_a_stale_revision() {
        // The core race-fix guard: a command carrying an outdated revision must
        // never act, even though an entry still exists for the flow.
        let entry = registry_entry(None, 2);
        assert_eq!(reconcile_action(Some(&entry), 1), ReconcileAction::SkipStaleRevision);
    }

    #[test]
    fn reconcile_action_cancels_when_the_flow_has_no_schedule() {
        let entry = registry_entry(None, 0);
        assert_eq!(reconcile_action(Some(&entry), 0), ReconcileAction::CancelBecauseNotScheduled);
    }

    #[test]
    fn reconcile_action_cancels_and_reschedules_when_matching_and_scheduled() {
        let entry = registry_entry(Some(Schedule::Cron("* * * * * *".to_string())), 0);
        assert_eq!(reconcile_action(Some(&entry), 0), ReconcileAction::CancelAndReschedule);
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
}
