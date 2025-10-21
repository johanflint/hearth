use crate::domain::GeoLocation;
use crate::execute_flows::execute_flows;
use crate::flow_registry::FlowRegistry;
use crate::scheduler::SchedulerCommand;
use crate::store::StoreSnapshot;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver;
use tracing::instrument;

#[instrument(skip_all)]
pub async fn store_listener(mut rx: Receiver<StoreSnapshot>, flow_registry: Arc<FlowRegistry>, scheduler_tx: Sender<SchedulerCommand>, geo_location: GeoLocation) {
    while rx.changed().await.is_ok() {
        let snapshot: StoreSnapshot = rx.borrow().clone();
        execute_flows(flow_registry.reactive_flows(), snapshot, scheduler_tx.clone(), geo_location.clone()).await;
    }
}
