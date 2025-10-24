use crate::domain::GeoLocation;
use crate::execute_flows::{execute_flow, execute_flows};
use crate::flow_registry::FlowRegistry;
use crate::store::StoreSnapshot;
use chrono::Utc;
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

// Pass config
#[instrument(skip_all)]
pub async fn scheduler(
    tx: Sender<SchedulerCommand>,
    mut rx: Receiver<SchedulerCommand>,
    notifier_rx: WatchReceiver<StoreSnapshot>,
    flow_registry: Arc<FlowRegistry>,
    geo_location: GeoLocation,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            SchedulerCommand::Schedule { flow_id } => {
                let Some(flow) = flow_registry.by_id(&flow_id) else {
                    warn!("🕗 Scheduling flow '{}'... failed, flow not found", flow_id);
                    continue;
                };

                let flow_name = flow.name().to_string();
                debug!("🕗 Scheduling flow '{}'...", flow_name);

                let Some(schedule) = flow.schedule() else {
                    error!("🕗 Scheduling flow '{}'... failed, not a scheduled flow", flow.name());
                    continue;
                };

                let schedule_str = schedule.to_string();

                // Job loop
                let notifier_rx_clone = notifier_rx.clone();
                let tx_clone = tx.clone();
                let geo_location_clone = geo_location.clone();
                tokio::spawn(async move {
                    for datetime in schedule.upcoming(Utc, geo_location_clone.clone()) {
                        let duration = datetime.signed_duration_since(Utc::now());
                        if duration.num_milliseconds() < 0 {
                            continue; // Already passed
                        }

                        let scheduled_instant = Instant::now() + Duration::from_millis(duration.num_milliseconds() as u64);
                        sleep_until(scheduled_instant).await;

                        debug!("🕗 Running scheduled flow '{}'...", flow.name());
                        let snapshot = notifier_rx_clone.borrow().clone();
                        execute_flows(vec![flow.clone()], snapshot, tx_clone.clone(), geo_location_clone.clone()).await;
                    }
                });
                info!(schedule = schedule_str, "🕗 Scheduling flow '{}'... OK", flow_name);
            }
            SchedulerCommand::ScheduleOnce { flow_id, node_id, delay } => {
                let Some(flow) = flow_registry.by_id(&flow_id) else {
                    warn!("🕗 Scheduling flow '{}'... failed, flow not found", flow_id);
                    return;
                };

                debug!("🕗 Scheduling flow '{}' to run node '{}' after {:?}... OK", flow_id, node_id, delay);
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
