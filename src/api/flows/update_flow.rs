use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::flows::commit::{flow_factory_error_response, invalid_json, parse_request};
use crate::flow_service::UpdateFlowError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

pub(super) async fn update_flow(Path(id): Path<String>, State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    debug!("Received request to update flow '{id}'...");
    let request: UpdateFlowRequest = parse_request(&body)?;
    let revision = state.flow_service.update_flow(&id, request.base_revision, request.flow).await.map_err(update_error_response)?;
    info!("📥 Received request to update flow '{id}'... OK, revision {revision}");

    Ok((StatusCode::OK, Json(UpdateFlowResponse { id, revision })).into_response())
}

fn update_error_response(err: UpdateFlowError) -> Response {
    match err {
        UpdateFlowError::InvalidDocument(err) => invalid_json(err),
        UpdateFlowError::InvalidFlow(err) => flow_factory_error_response(err),
        UpdateFlowError::IdMismatch { id, flow_id } => {
            let response = ErrorResponse::new("flowIdMismatch", format!("body id '{flow_id}' does not match path id '{id}'"));
            (StatusCode::UNPROCESSABLE_ENTITY, Json(response)).into_response()
        }
        UpdateFlowError::SchedulerUnavailable => {
            error!("❌ Scheduler is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("schedulerUnavailable"))).into_response()
        }
        UpdateFlowError::NotFound => (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response(),
        UpdateFlowError::RevisionConflict { .. } => (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response(),
        UpdateFlowError::Internal(err) => {
            error!("❌ Failed to persist flow to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
        UpdateFlowError::CommitFailed(err) => {
            error!("❌ Flow commit task failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("internalError"))).into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateFlowRequest {
    base_revision: u64,
    // Kept as generic JSON so it can be parsed into a `SerializedFlow` and stored in the db as is
    flow: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct UpdateFlowResponse {
    pub id: String,
    pub revision: u64,
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::router;
    use crate::api::flows::test_support::{VALID_FLOW_ID, VALID_FLOW_JSON, Fixture, body_json, create_state, valid_flow_document};
    use crate::flow_engine::{SchedulerCommand, VersionedFlow};
    use crate::flow_loader;
    use crate::flow_loader::SerializedFlow;
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    const INVALID_FLOW_ID: &str = "01K7KKNRMMQZCBRKMM914VK75R";
    const INVALID_FLOW_JSON: &str = include_str!("../../../tests/resources/flows/invalid/missingEndNodeFlow.json");

    async fn call_update_flow(state: ApiState, id: &str, body: &str) -> Response {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/flows/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn update_request(base_revision: u64, flow_json: &str) -> String {
        format!(r#"{{"baseRevision":{base_revision},"flow":{flow_json}}}"#)
    }

    fn valid_versioned_flow(revision: u64) -> VersionedFlow {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        VersionedFlow { flow: Arc::new(flow_loader::from_json(payload).unwrap()), revision }
    }

    /// Seeds the store and registry with the valid flow at revision 0, like at boot
    async fn create_seeded_state() -> Fixture {
        let fixture = create_state(FlowRegistry::new(vec![valid_versioned_flow(0)]));
        fixture.flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");
        fixture
    }

    #[tokio::test]
    async fn update_flow_persists_the_flow_to_the_store() {
        let Fixture { state, flow_store, flow_registry, scheduler_rx: _scheduler_rx } = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored_flows = flow_store.list().await.expect("list succeeded");
        assert_eq!(stored_flows.len(), 1, "expected a single flow in the FlowStore");
        assert_eq!(stored_flows[0].id, VALID_FLOW_ID);
        assert_eq!(stored_flows[0].revision, 1);
        // Fails if the request envelope was stored instead of the flow
        assert_eq!(stored_flows[0].document, valid_flow_document(), "stored document must be the flow");
        assert_eq!(flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 1);
    }

    #[tokio::test]
    async fn update_flow_replaces_the_registry_entry_and_sends_a_reconcile_command() {
        let Fixture { state, flow_registry, mut scheduler_rx, .. } = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["id"], VALID_FLOW_ID);
        assert_eq!(body["revision"], 1);
        let versioned_flow = flow_registry.by_id(VALID_FLOW_ID).expect("expected a flow");
        assert_eq!(versioned_flow.revision, 1);

        match scheduler_rx.try_recv().expect("expected a Reconcile command") {
            SchedulerCommand::Reconcile { flow_id, revision } => {
                assert_eq!(flow_id, VALID_FLOW_ID);
                assert_eq!(revision, 1);
            }
            other => panic!("expected Reconcile command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn update_flow_leaves_the_registry_untouched_and_skips_reconcile_when_a_newer_revision_already_landed() {
        // Registry that is ahead of the store simulates a concurrent update that landed first
        let newer = valid_versioned_flow(5);
        let Fixture { state, flow_store, flow_registry, mut scheduler_rx } = create_state(FlowRegistry::new(vec![newer.clone()]));
        flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["revision"], 1);
        let entry = flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry");
        assert_eq!(entry.revision, 5);
        assert!(Arc::ptr_eq(&entry.flow, &newer.flow), "registry entry must not be replaced");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_409_conflict_and_changes_nothing_when_the_base_revision_is_stale() {
        let Fixture { state, flow_store, flow_registry, mut scheduler_rx } = create_seeded_state().await;
        // Someone else updated the flow first
        flow_store.update(VALID_FLOW_ID, 0, valid_flow_document()).await.expect("concurrent update");

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert_eq!(body["message"], "revision conflict: update is based on revision 0, but the current revision is 1");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 1);
        assert_eq!(flow_registry.by_id(VALID_FLOW_ID).unwrap().revision, 0);
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_body_is_not_utf8() {
        let Fixture { state, mut scheduler_rx, .. } = create_seeded_state().await;
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/flows/{VALID_FLOW_ID}"))
            .header("content-type", "application/json")
            .body(Body::from(vec![0xff, 0xfe]))
            .unwrap();

        let response = router().with_state(state).oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidUtf8");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_base_revision_is_missing() {
        let Fixture { state, mut scheduler_rx, .. } = create_seeded_state().await;
        let body = format!(r#"{{"flow":{VALID_FLOW_JSON}}}"#);

        let response = call_update_flow(state, VALID_FLOW_ID, &body).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_body_is_a_flow_without_envelope() {
        let Fixture { state, mut scheduler_rx, .. } = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_flow_is_not_a_valid_serialized_flow() {
        let Fixture { state, flow_store, mut scheduler_rx, .. } = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, r#"{"id":"x"}"#)).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 0);
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_422_unprocessable_entity_when_the_path_id_does_not_match_the_body_id() {
        let Fixture { state, mut scheduler_rx, .. } = create_seeded_state().await;

        let response = call_update_flow(state, "invalidId", &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "flowIdMismatch");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_422_unprocessable_entity_for_a_semantically_invalid_flow() {
        let Fixture { state, mut scheduler_rx, .. } = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, INVALID_FLOW_ID, &update_request(0, INVALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "missingEndNode");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_404_not_found_and_persists_nothing_for_an_unknown_flow() {
        let Fixture { state, flow_store, mut scheduler_rx, .. } = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "flowNotFound");
        assert!(flow_store.list().await.unwrap().is_empty(), "unknown flow must not be persisted");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_registers_and_reconciles_a_flow_that_is_stored_but_not_registered() {
        // E.g. a stored flow that failed to load at boot
        let Fixture { state, flow_store, flow_registry, mut scheduler_rx } = create_state(FlowRegistry::new(vec![]));
        flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["revision"], 1);
        assert_eq!(flow_registry.by_id(VALID_FLOW_ID).expect("flow registered").revision, 1);
        match scheduler_rx.try_recv().expect("expected a Reconcile command") {
            SchedulerCommand::Reconcile { flow_id, revision } => {
                assert_eq!(flow_id, VALID_FLOW_ID);
                assert_eq!(revision, 1);
            }
            other => panic!("expected Reconcile command, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn update_flow_returns_503_service_unavailable_and_changes_nothing_when_the_scheduler_is_unavailable() {
        let Fixture { state, flow_store, flow_registry, scheduler_rx } = create_seeded_state().await;

        // Dropping the receiver simulates the scheduler being gone: `reserve_owned()`
        // then fails immediately instead of waiting for capacity, so the permit is
        // never granted.
        drop(scheduler_rx);

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "schedulerUnavailable");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 0);
        assert_eq!(flow_registry.by_id(VALID_FLOW_ID).unwrap().revision, 0);
    }

    #[tokio::test]
    async fn update_flow_completes_the_registry_update_and_reconcile_when_the_request_is_cancelled() {
        let Fixture { state, flow_registry, mut scheduler_rx, .. } = create_seeded_state().await;

        // Poll the request once, up to the pending store write, then drop it like a client disconnect
        let body = update_request(0, VALID_FLOW_JSON);
        let mut request = Box::pin(call_update_flow(state, VALID_FLOW_ID, &body));
        let poll = request.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop()));
        assert!(poll.is_pending(), "request should still be in flight");
        drop(request);

        match tokio::time::timeout(Duration::from_secs(1), scheduler_rx.recv()).await {
            Ok(Some(SchedulerCommand::Reconcile { flow_id, revision })) => {
                assert_eq!(flow_id, VALID_FLOW_ID);
                assert_eq!(revision, 1);
            }
            other => panic!("expected Reconcile command, got {:?}", other),
        }
        assert_eq!(flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 1);
    }
}
