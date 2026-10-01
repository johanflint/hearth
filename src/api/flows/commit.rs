use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::flow_engine::flow::Flow;
use crate::flow_engine::{SchedulerCommand, VersionedFlow};
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use crate::flow_registry::RegisterResult;
use axum::Json;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;
use tokio::sync::mpsc::OwnedPermit;
use tracing::{debug, error};

pub(super) fn parse_request<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    let body = str::from_utf8(body).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::new_code_only("invalidUtf8"))).into_response())?;
    serde_json::from_str(body).map_err(invalid_json)
}

pub(super) fn validate_flow_document(document: &serde_json::Value) -> Result<Flow, Response> {
    let serialized_flow = SerializedFlow::deserialize(document).map_err(invalid_json)?;
    flow_loader::from_json(serialized_flow).map_err(flow_factory_error_response)
}

/// Reserves channel capacity before mutating anything, so an unavailable scheduler is rejected
/// up front and the `Reconcile` send after the commit is infallible
pub(super) async fn reserve_scheduler_permit(state: &ApiState) -> Result<OwnedPermit<SchedulerCommand>, Response> {
    state.scheduler_tx.clone().reserve_owned().await.map_err(|err| {
        error!("❌ Scheduler is unavailable: {err}");
        (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("schedulerUnavailable"))).into_response()
    })
}

/// Spawned so a client disconnect can't cancel the request between the store write and the registry update,
/// the commit always runs to completion
pub(super) async fn run_to_completion<F>(commit: F) -> Result<Response, Response>
where
    F: Future<Output=Result<Response, Response>> + Send + 'static,
{
    tokio::spawn(commit).await.unwrap_or_else(|err| {
        error!("❌ Flow commit task failed: {err}");
        Err((StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("internalError"))).into_response())
    })
}

/// The flow store owns the revision; the registry mirrors it
pub(super) fn register_and_reconcile(state: &ApiState, flow: Flow, revision: u64, permit: OwnedPermit<SchedulerCommand>) {
    let id = flow.id().to_string();
    match state.flow_registry.register(VersionedFlow { flow: Arc::new(flow), revision }) {
        // Always reconcile: the scheduler derives the desired schedule state itself
        // Send on a reserved permit is synchronous and infallible
        RegisterResult::Added | RegisterResult::Replaced => {
            permit.send(SchedulerCommand::Reconcile { flow_id: id, revision });
        }
        // A concurrent write with a newer revision already landed and sent its own Reconcile
        RegisterResult::Stale { current_revision } => {
            debug!("Flow '{id}' revision {revision} is superseded by revision {current_revision}");
        }
    }
}

fn invalid_json(err: serde_json::Error) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidJson", err.to_string()))).into_response()
}

fn flow_factory_error_response(err: FlowFactoryError) -> Response {
    let message = err.to_string();
    let error_response = match err {
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

    (StatusCode::UNPROCESSABLE_ENTITY, Json(error_response)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::test_support::{INVALID_FLOW_JSON, VALID_FLOW_ID, VALID_FLOW_JSON, body_json, create_state, valid_flow_document};
    use crate::flow_registry::FlowRegistry;
    use serde_json::json;
    use tokio::sync::oneshot;

    fn valid_flow() -> Flow {
        flow_loader::from_json(serde_json::from_str(VALID_FLOW_JSON).unwrap()).unwrap()
    }

    fn panicking_commit() -> Result<Response, Response> {
        panic!("commit failed")
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    #[test]
    fn parse_request_parses_a_json_body() {
        let body = Bytes::from(r#"{"flow":{"id":"flow"}}"#);

        let request: serde_json::Value = parse_request(&body).expect("parsed");

        assert_eq!(request, json!({"flow": {"id": "flow"}}));
    }

    #[tokio::test]
    async fn parse_request_returns_400_invalid_utf8_for_a_body_that_is_not_utf8() {
        let body = Bytes::from_static(&[0xff, 0xfe]);

        let response = parse_request::<serde_json::Value>(&body).expect_err("must fail");

        assert_error(response, StatusCode::BAD_REQUEST, "invalidUtf8").await;
    }

    #[tokio::test]
    async fn parse_request_returns_400_invalid_json_for_a_body_that_is_not_json() {
        let body = Bytes::from("not json");

        let response = parse_request::<serde_json::Value>(&body).expect_err("must fail");

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
    }

    #[test]
    fn validate_flow_document_returns_the_flow_for_a_valid_document() {
        let flow = validate_flow_document(&valid_flow_document()).expect("valid flow");

        assert_eq!(flow.id(), VALID_FLOW_ID);
    }

    #[tokio::test]
    async fn validate_flow_document_returns_400_invalid_json_for_a_document_that_is_not_a_flow() {
        let response = validate_flow_document(&json!({"id": "flow"})).expect_err("must fail");

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
    }

    #[tokio::test]
    async fn validate_flow_document_returns_422_with_the_validation_code_for_a_semantically_invalid_flow() {
        let document = serde_json::from_str(INVALID_FLOW_JSON).unwrap();

        let response = validate_flow_document(&document).expect_err("must fail");

        assert_error(response, StatusCode::UNPROCESSABLE_ENTITY, "missingEndNode").await;
    }

    #[tokio::test]
    async fn reserve_scheduler_permit_reserves_capacity() {
        let (state, _scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let capacity = state.scheduler_tx.capacity();

        let _permit = reserve_scheduler_permit(&state).await.expect("permit reserved");

        assert_eq!(state.scheduler_tx.capacity(), capacity - 1);
    }

    #[tokio::test]
    async fn reserve_scheduler_permit_returns_503_scheduler_unavailable_when_the_scheduler_is_gone() {
        let (state, scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        drop(scheduler_rx);

        let response = reserve_scheduler_permit(&state).await.expect_err("must fail");

        assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "schedulerUnavailable").await;
    }

    #[tokio::test]
    async fn run_to_completion_returns_the_result_of_the_commit() {
        let ok = run_to_completion(async { Ok(StatusCode::OK.into_response()) }).await;
        let err = run_to_completion(async { Err(StatusCode::CONFLICT.into_response()) }).await;

        assert_eq!(ok.expect("ok").status(), StatusCode::OK);
        assert_eq!(err.expect_err("err").status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn run_to_completion_returns_500_internal_error_when_the_commit_panics() {
        let response = run_to_completion(async { panicking_commit() }).await.expect_err("must fail");

        assert_error(response, StatusCode::INTERNAL_SERVER_ERROR, "internalError").await;
    }

    #[tokio::test]
    async fn run_to_completion_finishes_the_commit_when_the_request_is_cancelled() {
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let (done_tx, done_rx) = oneshot::channel();
        let request = tokio::spawn(run_to_completion(async move {
            started_tx.send(()).unwrap();
            release_rx.await.unwrap();
            done_tx.send(()).unwrap();
            Ok(StatusCode::OK.into_response())
        }));
        started_rx.await.expect("commit started");

        // Simulates a client disconnect halfway through the commit
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        release_tx.send(()).unwrap();

        done_rx.await.expect("commit ran to completion");
    }

    #[tokio::test]
    async fn register_and_reconcile_registers_an_unknown_flow_and_reconciles_it() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let permit = reserve_scheduler_permit(&state).await.unwrap();

        register_and_reconcile(&state, valid_flow(), 0, permit);

        assert_eq!(state.flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 0);
        assert!(matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { flow_id, revision: 0 }) if flow_id == VALID_FLOW_ID));
    }

    #[tokio::test]
    async fn register_and_reconcile_replaces_an_older_revision_and_reconciles_it() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(valid_flow()), revision: 0 }]));
        let permit = reserve_scheduler_permit(&state).await.unwrap();

        register_and_reconcile(&state, valid_flow(), 1, permit);

        assert_eq!(state.flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 1);
        assert!(matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { revision: 1, .. })));
    }

    #[tokio::test]
    async fn register_and_reconcile_ignores_a_stale_revision_and_releases_the_permit() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(valid_flow()), revision: 5 }]));
        let capacity = state.scheduler_tx.capacity();
        let permit = reserve_scheduler_permit(&state).await.unwrap();

        register_and_reconcile(&state, valid_flow(), 3, permit);

        assert_eq!(state.flow_registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 5, "registry must be unchanged");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        assert_eq!(state.scheduler_tx.capacity(), capacity, "permit must be released");
    }
}
