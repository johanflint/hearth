use crate::flow_engine;
use crate::flow_engine::flow::Flow;
use crate::flow_engine::{Context, FlowEngineError};
use crate::store::DeviceMap;
use futures::stream::FuturesUnordered;
use tokio::sync::watch::Receiver;
use tracing::{info, instrument};

#[instrument(skip_all)]
pub async fn store_listener(mut rx: Receiver<DeviceMap>, flows: Vec<Flow>) {
    while rx.changed().await.is_ok() {
        let store: DeviceMap = rx.borrow().clone();
        // Note that the read_guard locks until it is dropped, can be avoided to clone the read_guard which is expensive
        let read_guard = store.read().await;
        info!("Updated store: {:?}", read_guard);

        let context = Context::new(store.clone());
        execute_flows(&flows, &context).await;
    }
}

async fn execute_flows(flows: &[Flow], context: &Context) -> Vec<Result<(), FlowEngineError>> {
    use futures::stream::StreamExt;
    FuturesUnordered::from_iter(flows.iter().map(|flow| async { flow_engine::execute(flow, context).await }))
        .collect::<Vec<_>>()
        .await
}
