use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::flow_engine::SchedulerCommand;
use crate::flow_loader::FlowFactoryError;
use axum::Json;
use axum::body::Bytes;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc::OwnedPermit;
use tracing::error;

pub(super) fn parse_request<T: DeserializeOwned>(body: &Bytes) -> Result<T, Response> {
    let body = str::from_utf8(body).map_err(|_| (StatusCode::BAD_REQUEST, Json(ErrorResponse::new_code_only("invalidUtf8"))).into_response())?;
    serde_json::from_str(body).map_err(invalid_json)
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

pub(super) fn invalid_json(err: serde_json::Error) -> Response {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse::new("invalidJson", err.to_string()))).into_response()
}

pub(super) fn flow_factory_error_response(err: FlowFactoryError) -> Response {
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
    use crate::api::flows::test_support::{body_json, create_state};
    use crate::flow_registry::FlowRegistry;
    use serde_json::json;
    use tokio::sync::oneshot;

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
}
