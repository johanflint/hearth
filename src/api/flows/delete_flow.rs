use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::flows::commit::{reserve_scheduler_permit, run_to_completion};
use crate::flow_engine::SchedulerCommand;
use crate::flow_registry::UnregisterResult;
use crate::flow_store::DeleteError;
use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{debug, error, info};

pub(super) async fn delete_flow(Path(id): Path<String>, query: Result<Query<DeleteFlowQuery>, QueryRejection>, State(state): State<ApiState>) -> Result<Response, Response> {
    let Query(DeleteFlowQuery { base_revision }) = query.map_err(invalid_query)?;
    let permit = reserve_scheduler_permit(&state).await?;

    // Known race, accepted: an update that wrote this revision to the store, but hasn't registered it yet,
    // re-adds the flow to the registry after this delete removes it. The flow then keeps running until the
    // next boot, which loads from the store. Rare (needs a concurrent update and delete of the same flow),
    // so not worth a per-flow lock.
    run_to_completion(async move {
        state.flow_store.delete(&id, base_revision).await.map_err(delete_error_response)?;

        // The store is authoritative, so the flow is deleted no matter the registry
        match state.flow_registry.unregister(&id, base_revision) {
            UnregisterResult::Deleted => {
                // The reconcile will detect that the flow is gone from the registry and cancel any schedules
                permit.send(SchedulerCommand::Reconcile { flow_id: id.clone(), revision: base_revision });
            }
            UnregisterResult::NotFound => debug!("Flow '{id}' was not registered, e.g. because it failed to load at boot"),
            UnregisterResult::Stale { current_revision } => debug!("Flow '{id}' revision {base_revision} is superseded by revision {current_revision}"),
        };

        info!("📥 Received request to delete flow '{id}'... OK, revision {base_revision}");
        Ok(StatusCode::NO_CONTENT.into_response())
    }).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFlowQuery {
    base_revision: u64,
}

fn invalid_query(err: QueryRejection) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidQuery", err.body_text()))).into_response()
}

fn delete_error_response(err: DeleteError) -> Response {
    match err {
        DeleteError::NotFound => (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response(),
        DeleteError::RevisionConflict { .. } => (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response(),
        DeleteError::Store(err) => {
            error!("❌ Failed to delete flow from store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::router;
    use crate::api::flows::test_support::{VALID_FLOW_ID, VALID_FLOW_JSON, body_json, create_state, valid_flow_document};
    use crate::flow_engine::{SchedulerCommand, VersionedFlow};
    use crate::flow_loader;
    use crate::flow_loader::SerializedFlow;
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    async fn call_delete_flow(state: ApiState, uri: &str) -> Response {
        let request = Request::builder().method("DELETE").uri(uri).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn delete_uri(id: &str, base_revision: u64) -> String {
        format!("/api/flows/{id}?baseRevision={base_revision}")
    }

    fn valid_versioned_flow(revision: u64) -> VersionedFlow {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        VersionedFlow { flow: Arc::new(flow_loader::from_json(payload).unwrap()), revision }
    }

    // Stores a validflow at the given revision
    async fn seed_store(state: &ApiState, revision: u64) {
        state.flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");
        for base_revision in 0..revision {
            state.flow_store.update(VALID_FLOW_ID, base_revision, valid_flow_document()).await.expect("seed store revision");
        }
    }

    /// Seeds the store and registry with the valid flow at the given revision, like at boot
    async fn create_seeded_state(revision: u64) -> (ApiState, mpsc::Receiver<SchedulerCommand>) {
        let (state, scheduler_rx) = create_state(FlowRegistry::new(vec![valid_versioned_flow(revision)]));
        seed_store(&state, revision).await;
        (state, scheduler_rx)
    }

    #[tokio::test]
    async fn delete_flow_returns_204_no_content_when_the_base_revision_matches() {
        let (state, _scheduler_rx) = create_seeded_state(0).await;
        let flow_store = state.flow_store.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let stored_flow = flow_store.by_id(VALID_FLOW_ID).await.expect("by_id succeeded");
        assert!(stored_flow.is_none(), "flow must be deleted, got {stored_flow:?}");
    }

    #[tokio::test]
    async fn delete_flow_deletes_the_registry_entry_and_sends_a_reconcile_command() {
        let (state, mut scheduler_rx) = create_seeded_state(0).await;
        let registry = state.flow_registry.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let versioned_flow = registry.by_id(VALID_FLOW_ID);
        assert!(versioned_flow.is_none(), "expected the flow to be deleted from the registry");

        match scheduler_rx.try_recv().expect("expected a Reconcile command") {
            SchedulerCommand::Reconcile { flow_id, revision } => {
                assert_eq!(flow_id, VALID_FLOW_ID);
                assert_eq!(revision, 0);
            }
            other => panic!("expected Reconcile command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn delete_flow_deletes_a_registry_entry_that_lags_behind_the_store() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![valid_versioned_flow(0)]));
        seed_store(&state, 1).await;
        let registry = state.flow_registry.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 1)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(registry.by_id(VALID_FLOW_ID).is_none(), "expected the flow to be deleted from the registry");
        assert!(matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { revision: 1, .. })));
    }

    #[tokio::test]
    async fn delete_flow_deletes_a_stored_flow_that_is_not_registered() {
        // E.g. a flow that failed to load at boot
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        seed_store(&state, 0).await;
        let flow_store = state.flow_store.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 0)).await;

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert!(flow_store.by_id(VALID_FLOW_ID).await.unwrap().is_none(), "flow must be deleted");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn delete_flow_returns_409_conflict_for_a_stale_base_revision() {
        let (state, mut scheduler_rx) = create_seeded_state(1).await;
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 0)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(response).await["code"], "revisionConflict");
        assert!(flow_store.by_id(VALID_FLOW_ID).await.unwrap().is_some(), "flow must not be deleted from the store");
        assert!(registry.by_id(VALID_FLOW_ID).is_some(), "flow must not be deleted from the registry");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn delete_flow_returns_409_conflict_for_a_future_base_revision() {
        let (state, mut scheduler_rx) = create_seeded_state(0).await;
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();

        let response = call_delete_flow(state, &delete_uri(VALID_FLOW_ID, 1)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(response).await["code"], "revisionConflict");
        assert!(flow_store.by_id(VALID_FLOW_ID).await.unwrap().is_some(), "flow must not be deleted from the store");
        assert!(registry.by_id(VALID_FLOW_ID).is_some(), "flow must not be deleted from the registry");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn delete_flow_returns_404_not_found_for_an_unknown_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let response = call_delete_flow(state, &delete_uri("unknownFlow", 0)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "flowNotFound");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn delete_flow_returns_400_bad_request_without_a_base_revision() {
        let (state, _scheduler_rx) = create_seeded_state(0).await;
        let flow_store = state.flow_store.clone();

        let response = call_delete_flow(state, &format!("/api/flows/{VALID_FLOW_ID}")).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidQuery");
        assert!(flow_store.by_id(VALID_FLOW_ID).await.unwrap().is_some(), "flow must not be deleted");
    }

    #[tokio::test]
    async fn delete_flow_returns_400_bad_request_for_a_base_revision_that_is_not_a_number() {
        let (state, _scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_delete_flow(state, &format!("/api/flows/{VALID_FLOW_ID}?baseRevision=latest")).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidQuery");
    }
}
