use crate::execute_flows::execute_flows;
use crate::flow_engine::flow::Flow;
use crate::scheduler::SchedulerCommand;
use crate::store::StoreSnapshot;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver;
use tracing::instrument;

#[instrument(skip_all)]
pub async fn store_listener(mut rx: Receiver<StoreSnapshot>, flows: Vec<Flow>, scheduler_tx: Sender<SchedulerCommand>) {
    while rx.changed().await.is_ok() {
        let snapshot: StoreSnapshot = rx.borrow().clone();
        execute_flows(&flows, snapshot, scheduler_tx.clone()).await;
    }
}
