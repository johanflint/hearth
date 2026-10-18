use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::parse_request::{invalid_json, parse_request};
use crate::api::presets::preset_response::PresetResponse;
use crate::domain::Value;
use crate::preset_service::CreatePresetError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn create_preset(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: CreatePresetRequest = parse_request(&body)?;
    let stored_preset =
        state.preset_service.create_preset(&request.name, request.value).await.map_err(create_error_response)?;
    info!("📥 Received request to create preset '{}'... OK, revision {}", stored_preset.id, stored_preset.revision);

    let location = format!("/api/presets/{}", stored_preset.id);
    Ok((StatusCode::CREATED, [(header::LOCATION, location)], Json(PresetResponse::from(stored_preset))).into_response())
}

#[derive(Debug, Deserialize)]
struct CreatePresetRequest {
    name: String,
    value: Value, // Rejects unknown types at parse time
}

fn create_error_response(err: CreatePresetError) -> Response {
    match err {
        CreatePresetError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("presetNameAlreadyExists"))).into_response()
        }
        CreatePresetError::UnsupportedValue(err) => invalid_json(err),
        CreatePresetError::Internal(err) => {
            error!("❌ Failed to persist preset to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::presets::router;
    use crate::api::test_support::{Fixture, body_json, create_state_with_database, insert_preset_row};
    use crate::database::DatabaseError;
    use crate::flow_registry::FlowRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use rstest::rstest;
    use serde_json::json;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn call_create_preset(state: ApiState, body: &str) -> Response {
        let request = Request::builder()
            .method("POST")
            .uri("/api/presets")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    #[tokio::test]
    async fn create_preset_returns_the_created_preset_with_camel_case_fields() {
        let (Fixture { state, .. }, _) = create_state_with_database(FlowRegistry::new(vec![]));

        let response =
            call_create_preset(state, r##"{"name":"Red","value":{"type":"color","value":"#ff0000"}}"##).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let location = response.headers()[header::LOCATION].to_str().expect("location is a string").to_string();
        let body = body_json(response).await;
        let id = body["id"].as_str().expect("id is a string");
        assert_eq!(Uuid::parse_str(id).expect("id is a UUID").get_version_num(), 7);
        assert_eq!(location, format!("/api/presets/{id}"));
        assert_eq!(body["revision"], 0);
        assert_eq!(body["name"], "Red");
        assert_eq!(body["value"], json!({ "type": "color", "value": "#ff0000" }));
        assert!(body["createdAt"].is_string(), "got {body}");
        assert!(body["updatedAt"].is_string(), "got {body}");
    }

    #[tokio::test]
    async fn create_preset_persists_the_preset_and_leaves_others_untouched() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "other", "Other", "10").await;
        let preset_service = state.preset_service.clone();

        let response = call_create_preset(state, r#"{"name":"Warm","value":true}"#).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = body_json(response).await;
        let created = preset_service.retrieve_preset(body["id"].as_str().unwrap()).await.expect("retrieve succeeded");
        assert_eq!(created.name, "Warm");
        assert_eq!(created.value, json!(true));
        assert_eq!(created.revision, 0);
        let other = preset_service.retrieve_preset("other").await.expect("retrieve succeeded");
        assert_eq!(other.name, "Other");
        assert_eq!(other.value, json!(10));
        assert_eq!(preset_service.list().await.expect("list succeeded").len(), 2);
    }

    #[rstest]
    #[case::same_case("Warm")]
    #[case::different_case("wARM")]
    #[tokio::test]
    async fn create_preset_returns_a_conflict_for_a_name_that_is_already_in_use(#[case] name: &str) {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        insert_preset_row(&database, "warm", "Warm", "true").await;
        let preset_service = state.preset_service.clone();
        let body = format!(r#"{{"name":"{name}","value":false}}"#);

        let response = call_create_preset(state, &body).await;

        assert_error(response, StatusCode::CONFLICT, "presetNameAlreadyExists").await;
        let presets = preset_service.list().await.expect("list succeeded");
        assert_eq!(presets.len(), 1, "nothing must be stored");
        assert_eq!(presets[0].value, json!(true));
    }

    #[rstest]
    #[case::not_json("not json")]
    #[case::missing_name(r#"{"value":true}"#)]
    #[case::missing_value(r#"{"name":"Warm"}"#)]
    #[case::name_is_not_a_string(r#"{"name":1,"value":true}"#)]
    #[case::value_is_null(r#"{"name":"Warm","value":null}"#)]
    #[case::value_is_an_array(r#"{"name":"Warm","value":[1]}"#)]
    #[case::unknown_value_type(r#"{"name":"Warm","value":{"type":"hologram","value":1}}"#)]
    #[tokio::test]
    async fn create_preset_returns_invalid_json_for_an_invalid_body_and_stores_nothing(#[case] body: &'static str) {
        let (Fixture { state, .. }, _) = create_state_with_database(FlowRegistry::new(vec![]));
        let preset_service = state.preset_service.clone();

        let response = call_create_preset(state, body).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert!(preset_service.list().await.expect("list succeeded").is_empty(), "nothing must be stored");
    }

    #[tokio::test]
    async fn create_preset_returns_a_storage_error_when_the_store_fails() {
        let (Fixture { state, .. }, database) = create_state_with_database(FlowRegistry::new(vec![]));
        database
            .call(|conn| conn.execute("DROP TABLE presets", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response = call_create_preset(state, r#"{"name":"Warm","value":true}"#).await;

        assert_error(response, StatusCode::INTERNAL_SERVER_ERROR, "storageError").await;
    }

    #[test]
    fn create_error_response_maps_an_unsupported_value_to_invalid_json() {
        let err = serde_json::to_value(Value::None).unwrap_err();

        let response = create_error_response(CreatePresetError::UnsupportedValue(err));

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
