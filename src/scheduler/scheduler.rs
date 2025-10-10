use crate::execute_flows::execute_flows;
use crate::flow_engine::flow::Flow;
use crate::store::StoreSnapshot;
use chrono::Utc;
use cron::Schedule;
use std::str::FromStr;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::time::{Instant, sleep_until};
use tracing::{debug, error, info, instrument, warn};

#[derive(Debug)]
pub enum SchedulerCommand {
    Schedule(Flow),
}

#[instrument(skip_all)]
pub async fn scheduler(mut rx: Receiver<SchedulerCommand>, notifier_rx: WatchReceiver<StoreSnapshot>) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            SchedulerCommand::Schedule(flow) if flow.schedule().is_some() => {
                let flow_name = flow.name().to_string();
                debug!("🕗 Scheduling flow '{}'...", flow_name);

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
                        execute_flows(std::slice::from_ref(&flow), snapshot).await;
                    }
                });
                info!("🕗 Scheduling flow '{}'... OK", flow_name);
            }
            SchedulerCommand::Schedule(flow) => {
                error!("🕗 Scheduling flow '{}'... failed, not a scheduled flow", flow.name());
            }
        }
    }
}
