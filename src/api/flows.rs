use crate::api::ApiState;
use crate::flow_engine::SchedulerCommand;
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::put;
use axum::{Json, Router};
use serde::Serialize;
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

    let payload: SerializedFlow = match serde_json::from_str(body_str) {
        Ok(p) => p,
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
                FlowFactoryError::Deserialization(_) => unreachable!("payload is already deserialized by the json extractor"),
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

    if let Err(err) = state.flow_store.upsert(&id, body_str).await {
        error!("❌ Failed to persist flow to store: {err}");
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response();
    }

    let Some(updated_revision) = state.flow_registry.replace_existing(flow) else {
        return (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("flowNotFound"))).into_response();
    };

    // Always reconcile: the scheduler derives the desired schedule state itself
    // Send on a reserved permit is synchronous and infallible
    permit.send(SchedulerCommand::Reconcile { flow_id: id.clone(), revision: updated_revision });

    info!("Received request to update flow '{id}'... OK, revision {updated_revision}");
    (StatusCode::OK, Json(UpdateFlowResponse { id, revision: updated_revision })).into_response()
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
    use crate::flow_engine::VersionedFlow;
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

    #[tokio::test]
    async fn update_flow_replaces_the_registry_entry_and_sends_a_reconcile_command() {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        let flow = flow_loader::from_json(payload).unwrap();
        let versioned_flow = VersionedFlow { flow: Arc::new(flow), revision: 0 };
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![versioned_flow]));

        let response = call_update_flow(state, VALID_FLOW_ID, VALID_FLOW_JSON).await;

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
    async fn update_flow_returns_422_unprocessable_entity_when_the_path_id_does_not_match_the_body_id() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, "invalidId", VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "flowIdMismatch");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_422_for_a_semantically_invalid_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, INVALID_FLOW_ID, INVALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "missingEndNode");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_404_for_an_unknown_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_update_flow(state, VALID_FLOW_ID, VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(response).await["code"], "flowNotFound");
        assert!(scheduler_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn update_flow_returns_503_service_unavailable_and_leaves_the_registry_untouched_when_the_scheduler_is_unavailable() {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        let flow = flow_loader::from_json(payload).unwrap();
        let versioned_flow = VersionedFlow { flow: Arc::new(flow), revision: 0 };
        let (state, scheduler_rx) = create_state(FlowRegistry::new(vec![versioned_flow]));

        // Dropping the receiver simulates the scheduler being gone: `reserve_owned()`
        // then fails immediately instead of waiting for capacity, so the permit is
        // never granted.
        drop(scheduler_rx);

        let response = call_update_flow(state.clone(), VALID_FLOW_ID, VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "schedulerUnavailable");

        // No permit was granted, so the handler must have returned before
        // touching the registry at all - the flow's revision is still 0.
        assert_eq!(state.flow_registry.by_id(VALID_FLOW_ID).unwrap().revision, 0);
    }
}
