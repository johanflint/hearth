use crate::flow_engine::VersionedFlow;
use crate::flow_loader::factory::from_json_value;
use crate::flow_store::{FlowStore, FlowStoreError};
use std::sync::Arc;
use tracing::{info, instrument, warn};

#[instrument(skip_all)]
pub async fn load_flows_from_store(flow_store: &FlowStore) -> Result<Vec<VersionedFlow>, FlowStoreError> {
    info!("🗄️ Loading flows...");
    let stored_flows = flow_store.list().await?;
    let mut flows = Vec::with_capacity(stored_flows.len());
    let mut failed = 0;

    for stored_flow in stored_flows {
        match from_json_value(stored_flow.document) {
            Ok(flow) => flows.push(VersionedFlow { flow: Arc::new(flow), revision: stored_flow.revision }),
            Err(err) => {
                failed += 1;
                warn!("⚠️ Failed to load flow '{}' (revision {}): {}", stored_flow.id, stored_flow.revision, err);
            }
        }
    }

    info!("🗄️ Loading flows... OK, {} loaded, {failed} failed", flows.len());
    Ok(flows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    #[tokio::test]
    async fn load_flows_from_store_skips_invalid_flows_and_preserves_revisions() {
        let flow_store = FlowStore::open(Path::new(":memory:")).expect("failed to open in-memory flow store");
        let valid_flow = json!({"id": "validFlow", "name": "Valid", "nodes": [{"id": "startNode", "type": "startNode", "outgoingNode": "endNode"}, {"id": "endNode", "type": "endNode"}]});
        flow_store.insert("validFlow", valid_flow.clone()).await.expect("insert valid flow");
        flow_store.update("validFlow", 0, valid_flow).await.expect("update valid flow");
        // Valid JSON, but not a valid flow
        flow_store.insert("invalidFlow", json!({"id": "invalidFlow", "name": "Invalid", "nodes": []})).await.expect("insert invalid flow");

        let flows = load_flows_from_store(&flow_store).await.expect("load succeeded");

        assert_eq!(flows.len(), 1);
        assert_eq!(flows[0].id(), "validFlow");
        assert_eq!(flows[0].revision, 1);
    }
}
