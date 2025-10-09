use crate::execute_flows::execute_flows;
use crate::flow_engine::flow::Flow;
use crate::store::StoreSnapshot;
use tokio::sync::watch::Receiver;
use tracing::instrument;

#[instrument(skip_all)]
pub async fn store_listener(mut rx: Receiver<StoreSnapshot>, flows: Vec<Flow>) {
    while rx.changed().await.is_ok() {
        let snapshot: StoreSnapshot = rx.borrow().clone();
        execute_flows(&flows, snapshot).await;
    }
}
