use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::flows::commit::{flow_factory_error_response, invalid_json, parse_request};
use crate::flow_service::{CreateFlowError, CreatedFlow};
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use reqwest::header;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

pub(super) async fn create_flow(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: CreateFlowRequest = parse_request(&body)?;
    let CreatedFlow { id, revision } = state.flow_service.create_flow(request.flow).await.map_err(create_error_response)?;
    info!("📥 Received request to create flow '{id}'... OK, revision {revision}");

    let location = format!("/api/flows/{id}");
    Ok((StatusCode::CREATED, [(header::LOCATION, location)], Json(CreateFlowResponse { id, revision })).into_response())
}

#[derive(Debug, Deserialize)]
struct CreateFlowRequest {
    // Kept as generic JSON so it can be parsed into a `SerializedFlow` and stored in the db as is
    flow: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CreateFlowResponse {
    id: String,
    revision: u64,
}

fn create_error_response(err: CreateFlowError) -> Response {
    match err {
        CreateFlowError::InvalidDocument(err) => invalid_json(err),
        CreateFlowError::InvalidFlow(err) => flow_factory_error_response(err),
        CreateFlowError::SchedulerUnavailable => {
            error!("❌ Scheduler is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("schedulerUnavailable"))).into_response()
        }
        CreateFlowError::AlreadyExists => (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("flowAlreadyExists"))).into_response(),
        CreateFlowError::Internal(err) => {
            error!("❌ Failed to persist flow to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
        CreateFlowError::CommitFailed(err) => {
            error!("❌ Flow commit task failed: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("internalError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::flows::router;
    use crate::api::flows::test_support::{INVALID_FLOW_JSON, VALID_FLOW_ID, VALID_FLOW_JSON, body_json, create_state, valid_flow_document};
    use crate::flow_engine::SchedulerCommand;
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_create_flow(state: ApiState, body: &str) -> Response {
        let request = Request::builder()
            .method("POST")
            .uri("/api/flows")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn create_request(flow_json: &str) -> String {
        format!(r#"{{"flow":{flow_json}}}"#)
    }

    #[tokio::test]
    async fn create_flow_stores_registers_and_reconciles_a_new_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let flow_store = state.flow_store.clone();
        let registry = state.flow_registry.clone();

        let response = call_create_flow(state, &create_request(VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()[header::LOCATION], format!("/api/flows/{VALID_FLOW_ID}"));
        assert_eq!(body_json(response).await, json!({"id": VALID_FLOW_ID, "revision": 0}));
        let stored_flow = flow_store.by_id(VALID_FLOW_ID).await.expect("by_id succeeded").expect("flow stored");
        assert_eq!(stored_flow.document, valid_flow_document(), "stored document must be the flow");
        assert_eq!(registry.by_id(VALID_FLOW_ID).expect("flow in registry").revision, 0);
        assert!(matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { revision: 0, .. })));
    }

    #[tokio::test]
    async fn create_flow_returns_409_conflict_for_an_existing_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        state.flow_store.insert(VALID_FLOW_ID, valid_flow_document()).await.expect("seed store");
        let registry = state.flow_registry.clone();

        let response = call_create_flow(state, &create_request(VALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(response).await["code"], "flowAlreadyExists");
        assert!(registry.by_id(VALID_FLOW_ID).is_none(), "registry must be unchanged");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn create_flow_returns_422_unprocessable_entity_for_a_semantically_invalid_flow() {
        let (state, mut scheduler_rx) = create_state(FlowRegistry::new(vec![]));
        let flow_store = state.flow_store.clone();

        let response = call_create_flow(state, &create_request(INVALID_FLOW_JSON)).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(response).await["code"], "missingEndNode");
        assert!(flow_store.list().await.unwrap().is_empty(), "nothing must be stored");
        assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
    }

    #[tokio::test]
    async fn create_flow_returns_400_bad_request_without_a_flow_envelope() {
        let (state, _scheduler_rx) = create_state(FlowRegistry::new(vec![]));

        let response = call_create_flow(state, VALID_FLOW_JSON).await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(body_json(response).await["code"], "invalidJson");
    }
}
