use crate::api::ApiState;
use crate::flow_engine::{SchedulerCommand, VersionedFlow};
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use crate::flow_registry::ReplaceResult;
use crate::flow_store::FlowStoreError;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, error, info};

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/flow/{id}", put(update_flow))
}

async fn update_flow(Path(id): Path<String>, State(state): State<ApiState>, body: Bytes) -> Response {
    debug!("Received request to update flow '{id}'...");
    let body_str = match str::from_utf8(&body) {
        Ok(s) => s,
        Err(_) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse::new_code_only("invalidUtf8"))).into_response(),
    };

    let request: UpdateFlowRequest = match serde_json::from_str(body_str) {
        Ok(request) => request,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidJson", e.to_string()))).into_response(),
    };

    // Store the flow itself, not the request envelope
    let flow_json = request.flow.to_string();
    let payload: SerializedFlow = match serde_json::from_value(request.flow) {
        Ok(payload) => payload,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidJson", e.to_string()))).into_response(),
    };

    if id != payload.id {
        return (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("flowIdMismatch", format!("body id '{}' does not match path id '{}'", payload.id, id)))).into_response();
    }

    let flow = match flow_loader::from_json(payload) {
        Ok(flow) => flow,
        Err(flow_factory_error) => {
            let message = flow_factory_error.to_string();
            let error_response = match flow_factory_error {
                FlowFactoryError::Deserialization(_) => unreachable!("payload is already deserialized into a SerializedFlow"),
                FlowFactoryError::MissingStartNode => ErrorResponse::new("missingStartNode", message),
                FlowFactoryError::TooManyStartNodes(_) => ErrorResponse::new("tooManyStartNodes", message),
                FlowFactoryError::MissingEndNode => ErrorResponse::new("missingEndNode", message),
                FlowFactoryError::NoConnectingNode { .. } => ErrorResponse::new("noConnectingNode", message),
                FlowFactoryError::MissingNode { .. } => ErrorResponse::new("missingNode", message),
                FlowFactoryError::TooManyParentNodes { .. } => ErrorResponse::new("tooManyParentNodes", message),
                FlowFactoryError::UnusedNodes { .. } => ErrorResponse::new("unusedNodes", message),
                FlowFactoryError::DuplicateLinkValues { .. } => ErrorResponse::new("duplicateLinkValues", message),
                FlowFactoryError::PropertyChangedInScheduledFlow => ErrorResponse::new("propertyChangedInScheduledFlow", message),
            };

            return (StatusCode::UNPROCESSABLE_ENTITY, Json(error_response)).into_response();
        }
    };

    // Reserve channel capacity before mutating the registry. This way, a cancelled
    // request or a full channel can never leave the registry updated without a
    // matching `Reconcile` guaranteed to follow.
    let permit = match state.scheduler_tx.clone().reserve_owned().await {
        Ok(permit) => permit,
        Err(err) => {
            error!("❌ Scheduler is unavailable: {err}");
            return (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("schedulerUnavailable"))).into_response();
        }
    };

    // The flow store owns the revision; the registry mirrors it
    let updated_revision = match state.flow_store.update(&id, request.base_revision, &flow_json).await {
        Ok(revision) => revision,
        Err(FlowStoreError::NotFound) => return (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response(),
        Err(err @ FlowStoreError::RevisionConflict { .. }) => return (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response(),
        Err(err) => {
            error!("❌ Failed to persist flow to store: {err}");
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response();
        }
    };

    match state.flow_registry.replace_existing(VersionedFlow { flow: Arc::new(flow), revision: updated_revision }) {
        // Always reconcile: the scheduler derives the desired schedule state itself
        // Send on a reserved permit is synchronous and infallible
        ReplaceResult::Replaced => {
            permit.send(SchedulerCommand::Reconcile { flow_id: id.clone(), revision: updated_revision });
            info!("Received request to update flow '{id}'... OK, revision {updated_revision}");
        }
        // A concurrent update with a newer revision already landed and sent its own Reconcile
        ReplaceResult::Stale { current_revision } => {
            debug!("Received request to update flow '{id}'... superseded, revision {updated_revision} is older than revision {current_revision}");
        },
        ReplaceResult::NotFound => return (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response(),
    }

    (StatusCode::OK, Json(UpdateFlowResponse { id, revision: updated_revision })).into_response()
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

#[derive(Debug, Serialize)]
struct ErrorResponse {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ErrorResponse {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        ErrorResponse { code: code.to_string(), message: Some(message.into()) }
    }

    pub fn new_code_only(code: &'static str) -> Self {
        ErrorResponse { code: code.to_string(), message: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_registry::FlowRegistry;
    use crate::flow_store::FlowStore;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use metrics_exporter_prometheus::PrometheusBuilder;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    const VALID_FLOW_ID: &str = "01K7KK6H5R7Y72QJEJSJQCKMRQ";
    const VALID_FLOW_JSON: &str = include_str!("../../tests/resources/flows/logFlow.json");
    const INVALID_FLOW_ID: &str = "01K7KKNRMMQZCBRKMM914VK75R";
    const INVALID_FLOW_JSON: &str = include_str!("../../tests/resources/flows/invalid/missingEndNodeFlow.json");

    fn create_state(flow_registry: FlowRegistry) -> (ApiState, mpsc::Receiver<SchedulerCommand>) {
        let (scheduler_tx, scheduler_rx) = mpsc::channel(8);
        let handle = PrometheusBuilder::new().build_recorder().handle();
        let flow_store = FlowStore::open(std::path::Path::new(":memory:")).expect("failed to open in-memory flow store");
        (ApiState::new(handle, Arc::new(flow_registry), Arc::new(flow_store), scheduler_tx), scheduler_rx)
    }

    async fn call_update_flow(state: ApiState, id: &str, body: &str) -> Response {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/flow/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn update_request(base_revision: u64, flow_json: &str) -> String {
        format!(r#"{{"baseRevision":{base_revision},"flow":{flow_json}}}"#)
    }

    fn valid_versioned_flow(revision: u64) -> VersionedFlow {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        VersionedFlow { flow: Arc::new(flow_loader::from_json(payload).unwrap()), revision }
    }

    /// Seeds the store and registry with the valid flow at revision 0, like at boot
    async fn create_seeded_state() -> (ApiState, mpsc::Receiver<SchedulerCommand>) {
        let (state, scheduler_rx) = create_state(FlowRegistry::new(vec![valid_versioned_flow(0)]));
        state.flow_store.insert(VALID_FLOW_ID, VALID_FLOW_JSON).await.expect("seed store");
        (state, scheduler_rx)
    }

    #[tokio::test]
    async fn update_flow_persists_the_flow_to_the_store() {
        let (state, _scheduler_rx) = create_seeded_state().await;
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        let stored_flows = flow_store.list().await.expect("list succeeded");
        assert_eq!(stored_flows.len(), 1, "expected a single flow in the FlowStore");
        assert_eq!(stored_flows[0].id, VALID_FLOW_ID);
        assert_eq!(stored_flows[0].revision, 1);
        // Fails if the request envelope was stored instead of the flow
        assert!(stored_flows[0].flow.is_ok(), "stored document must be the flow: {:?}", stored_flows[0].flow);
        assert_eq!(registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 1);
    }

    #[tokio::test]
    async fn update_flow_replaces_the_registry_entry_and_sends_a_reconcile_command() {
        let (state, mut scheduler_rx) = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["id"], VALID_FLOW_ID);
        assert_eq!(body["revision"], 1);

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
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![newer.clone()]));
        state.flow_store.insert(VALID_FLOW_ID, VALID_FLOW_JSON).await.expect("seed store");
        let registry = state.flow_registry.clone();

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["revision"], 1);
        let entry = registry.by_id(VALID_FLOW_ID).expect("flow in registry");
        assert_eq!(entry.revision, 5);
        assert!(Arc::ptr_eq(&entry.flow, &newer.flow), "registry entry must not be replaced");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_409_conflict_and_changes_nothing_when_the_base_revision_is_stale() {
        let (state, mut scheduler_rx) = create_seeded_state().await;
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();
        // Someone else updated the flow first
        flow_store.update(VALID_FLOW_ID, 0, VALID_FLOW_JSON).await.expect("concurrent update");

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert_eq!(body["message"], "revision conflict: update is based on revision 0, but the current revision is 1");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 1);
        assert_eq!(registry.by_id(VALID_FLOW_ID).unwrap().revision, 0);
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_body_is_not_utf8() {
        let (state, mut scheduler_rx) = create_seeded_state().await;
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/flow/{VALID_FLOW_ID}"))
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
        let (state, mut scheduler_rx) = create_seeded_state().await;
        let body = format!(r#"{{"flow":{VALID_FLOW_JSON}}}"#);

        let response = call_update_flow(state, VALID_FLOW_ID, &body).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_body_is_a_flow_without_envelope() {
        let (state, mut scheduler_rx) = create_seeded_state().await;

        let response = call_update_flow(state, VALID_FLOW_ID, VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_400_bad_request_when_the_flow_is_not_a_valid_serialized_flow() {
        let (state, mut scheduler_rx) = create_seeded_state().await;
        let flow_store = state.flow_store.clone();

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, r#"{"id":"x"}"#)).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 0);
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_422_unprocessable_entity_when_the_path_id_does_not_match_the_body_id() {
        let (state, mut scheduler_rx) = create_seeded_state().await;

        let response = call_update_flow(state, "invalidId", &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "flowIdMismatch");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_422_unprocessable_entity_for_a_semantically_invalid_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, INVALID_FLOW_ID, &update_request(0, INVALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "missingEndNode");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_404_not_found_and_persists_nothing_for_an_unknown_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let flow_store = state.flow_store.clone();

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "flowNotFound");
        assert!(flow_store.list().await.unwrap().is_empty(), "unknown flow must not be persisted");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_404_not_found_when_the_flow_is_stored_but_not_registered() {
        // E.g. a stored flow that failed to load at boot
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        state.flow_store.insert(VALID_FLOW_ID, VALID_FLOW_JSON).await.expect("seed store");

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "flowNotFound");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_503_service_unavailable_and_changes_nothing_when_the_scheduler_is_unavailable() {
        let (state, scheduler_rx) = create_seeded_state().await;
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();

        // Dropping the receiver simulates the scheduler being gone: `reserve_owned()`
        // then fails immediately instead of waiting for capacity, so the permit is
        // never granted.
        drop(scheduler_rx);

        let response = call_update_flow(state, VALID_FLOW_ID, &update_request(0, VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "schedulerUnavailable");
        assert_eq!(flow_store.list().await.unwrap()[0].revision, 0);
        assert_eq!(registry.by_id(VALID_FLOW_ID).unwrap().revision, 0);
    }
}
