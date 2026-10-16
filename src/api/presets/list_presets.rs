use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::preset_service::ListPresetError;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tracing::error;

pub(super) async fn list_presets(State(state): State<ApiState>) -> Result<Response, Response> {
    let stored_presets = state.preset_service.list().await.map_err(list_error_response)?;

    let presets = stored_presets
        .into_iter()
        .map(|preset| PresetResponse {
            id: preset.id,
            name: preset.name,
            value: preset.value,
            created_at: preset.created_at,
            updated_at: preset.updated_at,
        })
        .collect();

    Ok((StatusCode::OK, Json(ListPresetsResponse { presets })).into_response())
}

#[derive(Debug, Serialize)]
struct ListPresetsResponse {
    presets: Vec<PresetResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PresetResponse {
    id: String,
    name: String,
    value: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: Option<DateTime<Utc>>,
}

fn list_error_response(err: ListPresetError) -> Response {
    match err {
        ListPresetError::Internal(err) => {
            error!("❌ Failed to list presets from store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::presets::router;
    use crate::api::test_support::{Fixture, body_json, create_state, create_state_with_database, insert_preset_row};
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_list_presets(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/presets").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn list_presets_returns_an_empty_list_for_an_empty_store() {
        let state = create_state(FlowRegistry::new(vec![])).state;

        let response = call_list_presets(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "presets": [] }));
    }

    #[tokio::test]
    async fn list_presets_returns_the_stored_presets_with_camel_case_fields() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", r##"{"type":"color","value":"#ff0000"}"##).await;
        insert_preset_row(&database, "dim", "Dim", "10").await;

        let response = call_list_presets(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        let presets = body["presets"].as_array().expect("presets array");
        assert_eq!(presets.len(), 2);
        assert_eq!(presets[0]["id"], "dim");
        assert_eq!(presets[0]["name"], "Dim");
        assert_eq!(presets[0]["value"], 10);
        assert!(presets[0]["createdAt"].is_string(), "got {body}");
        assert!(presets[0]["updatedAt"].is_null(), "got {body}");
        assert_eq!(presets[1]["id"], "warm");
        assert_eq!(presets[1]["value"], json!({ "type": "color", "value": "#ff0000" }));
    }

    #[tokio::test]
    async fn list_presets_returns_a_storage_error_when_the_store_fails() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "corrupt", "Corrupt", "not valid json").await;

        let response = call_list_presets(state).await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(response).await["code"], "storageError");
    }
}
