use crate::execute_flows::{execute_flow, execute_flows};
use crate::flow_registry::FlowRegistry;
use crate::store::StoreSnapshot;
use chrono::Utc;
use cron::Schedule;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub enum SchedulerCommand {
    Schedule { flow_id: String },
    ScheduleOnce { flow_id: String, node_id: String, delay: Duration },
}

#[instrument(skip_all)]
pub async fn scheduler(tx: Sender<SchedulerCommand>, mut rx: Receiver<SchedulerCommand>, notifier_rx: WatchReceiver<StoreSnapshot>, flow_registry: Arc<FlowRegistry>) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            SchedulerCommand::Schedule { flow_id } => {
                let Some(flow) = flow_registry.by_id(&flow_id) else {
                    warn!("🕗 Scheduling flow '{}'... failed, flow not found", flow_id);
                    continue;
                };

                let flow_name = flow.name().to_string();
                debug!("🕗 Scheduling flow '{}'...", flow_name);

                if flow.schedule().is_none() {
                    error!("🕗 Scheduling flow '{}'... failed, not a scheduled flow", flow.name());
                    continue;
                }

                let cron = flow.schedule().unwrap().to_string(); // Safe because of the match guard
                let schedule = match Schedule::from_str(&cron) {
                    Ok(schedule) => schedule,
                    Err(_e) => {
                        warn!("🕗 Scheduling flow '{}'... failed, invalid cron expression '{}'", flow.name(), cron);
                        continue;
                    }
                };

                // Job loop
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    for datetime in schedule.upcoming(Utc) {
                        let duration = datetime.signed_duration_since(Utc::now());
                        if duration.num_milliseconds() < 0 {
                            continue; // Already passed
                        }

                        let scheduled_instant = Instant::now() + Duration::from_millis(duration.num_milliseconds() as u64);
                        sleep_until(scheduled_instant).await;

                        debug!(cron, "🕗 Running scheduled flow '{}'...", flow.name());
                        let snapshot = notifier_rx_clone.borrow().clone();
                        execute_flows(vec![flow.clone()], snapshot, tx_clone.clone()).await;
                    }
                });
                info!("🕗 Scheduling flow '{}'... OK", flow_name);
            }
            SchedulerCommand::ScheduleOnce { flow_id, node_id, delay } => {
                let Some(flow) = flow_registry.by_id(&flow_id) else {
                    warn!("🕗 Scheduling flow '{}'... failed, flow not found", flow_id);
                    return;
                };

                debug!("🕗 Scheduling flow '{}' to run node '{}' after {:?}... OK", flow_id, node_id, delay);
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let scheduled_instant = Instant::now() + Duration::from_millis(delay.as_millis() as u64);
                    sleep_until(scheduled_instant).await;

                    debug!("🕗 Waking up flow '{}'...", flow.name());
                    let snapshot = notifier_rx_clone.borrow().clone();
                    execute_flow(flow, Some(node_id), snapshot, tx_clone.clone()).await;
                });
            }
        }
    }
}
