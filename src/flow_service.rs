use crate::flow_store::{FlowStore, FlowStoreError, StoredFlow};
use std::sync::Arc;
use thiserror::Error;
use tracing::error;

#[derive(Debug)]
pub struct FlowService {
    pub(super) flow_store: Arc<FlowStore>,
}

impl FlowService {
    pub fn new(flow_store: Arc<FlowStore>) -> Self {
        FlowService { flow_store }
    }

    pub async fn list(&self) -> Result<Vec<StoredFlow>, ListFlowsError> {
        Ok(self.flow_store.list().await?)
    }

    pub async fn retrieve_flow(&self, id: &str) -> Result<StoredFlow, RetrieveFlowError> {
        self.flow_store.by_id(id).await?.ok_or(RetrieveFlowError::NotFound)
    }
}

#[derive(Debug, Error)]
pub enum ListFlowsError {
    #[error(transparent)]
    Internal(#[from] FlowStoreError),
}

#[derive(Debug, Error)]
pub enum RetrieveFlowError {
    #[error("flow not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] FlowStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn flow_document() -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]})
    }

    fn create_service() -> (FlowService, Arc<FlowStore>) {
        let flow_store = Arc::new(FlowStore::open(Path::new(":memory:")).expect("failed to open in-memory flow store"));
        (FlowService::new(Arc::clone(&flow_store)), flow_store)
    }

    mod list {
        use super::*;

        #[tokio::test]
        async fn returns_an_empty_list_for_an_empty_store() {
            let (service, _flow_store) = create_service();

            let flows = service.list().await.expect("list succeeded");

            assert!(flows.is_empty(), "got {flows:?}");
        }

        #[tokio::test]
        async fn returns_the_stored_flows() {
            let (service, flow_store) = create_service();
            flow_store.insert("flow", flow_document()).await.expect("seed store");
            flow_store.update("flow", 0, flow_document()).await.expect("update store");
            flow_store.insert("other", flow_document()).await.expect("seed store");

            let mut flows = service.list().await.expect("list succeeded");

            flows.sort_by(|a, b| a.id.cmp(&b.id));
            let ids_and_revisions: Vec<(&str, u64)> = flows.iter().map(|flow| (flow.id.as_str(), flow.revision)).collect();
            assert_eq!(ids_and_revisions, vec![("flow", 1), ("other", 0)]);
        }
    }

    mod retrieve_flow {
        use super::*;

        #[tokio::test]
        async fn returns_the_stored_flow() {
            let (service, flow_store) = create_service();
            flow_store.insert("flow", flow_document()).await.expect("seed store");
            flow_store.update("flow", 0, flow_document()).await.expect("update store");

            let stored_flow = service.retrieve_flow("flow").await.expect("by_id succeeded");

            assert_eq!(stored_flow.id, "flow");
            assert_eq!(stored_flow.revision, 1);
            assert_eq!(stored_flow.document, flow_document());
        }

        #[tokio::test]
        async fn returns_not_found_for_an_unknown_flow() {
            let (service, flow_store) = create_service();
            flow_store.insert("other", flow_document()).await.expect("seed store");

            let result = service.retrieve_flow("flow").await;

            assert!(matches!(result, Err(RetrieveFlowError::NotFound)), "got {result:?}");
        }
    }
}
