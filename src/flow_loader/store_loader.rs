use crate::flow_engine::flow::Flow;
use crate::flow_store::{FlowStore, FlowStoreError};
use tracing::{info, instrument, warn};

#[instrument(skip_all)]
pub async fn load_flows_from_store(flow_store: &FlowStore) -> Result<Vec<Flow>, FlowStoreError> {
    info!("🗄️ Loading flows...");
    let stored_flows = flow_store.list().await?;
    let (flows, errors): (Vec<_>, Vec<_>) = stored_flows.into_iter().partition(|flow| flow.flow.is_ok());

    for error in errors.iter() {
        warn!("⚠️ Failed to load flow '{}' (revision {}): {}", error.id, error.revision, error.flow.as_ref().unwrap_err());
    }

    info!("🗄️ Loading flows... OK, {} loaded, {} failed", flows.len(), errors.len());
    let flows: Vec<Flow> = flows.into_iter().map(|flow| flow.flow.unwrap()).collect();
    Ok(flows)
}
