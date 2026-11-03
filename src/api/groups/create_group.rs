use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::groups::group_view::GroupView;
use crate::api::parse_request::parse_request;
use crate::group_service::CreateGroupError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn create_group(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: CreateGroupRequest = parse_request(&body)?;
    let stored_group = state
        .group_service
        .create_group(&request.id, &request.name, request.device_ids)
        .await
        .map_err(create_error_response)?;
    info!("📥 Received request to create group '{}'... OK, revision {}", stored_group.id, stored_group.revision);

    let location = format!("/api/groups/{}", stored_group.id);
    Ok((StatusCode::CREATED, [(header::LOCATION, location)], Json(GroupView::from(stored_group))).into_response())
}

fn create_error_response(err: CreateGroupError) -> Response {
    match err {
        CreateGroupError::IdAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("groupIdAlreadyExists"))).into_response()
        }
        CreateGroupError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("groupNameAlreadyExists"))).into_response()
        }
        CreateGroupError::GroupEmpty => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("groupEmpty", err.to_string()))).into_response()
        }
        CreateGroupError::UnknownDevices { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownDevices", err.to_string())))
                .into_response()
        }
        CreateGroupError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        CreateGroupError::Internal(_) => {
            error!("❌ Failed to persist group to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateGroupRequest {
    id: String,
    name: String,
    device_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::groups::router;
    use crate::api::test_support::{body_json, create_state_with_store_tx};
    use crate::database::Database;
    use crate::domain::{Event, Group, GroupOwner};
    use crate::group_store::GroupStore;
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use tower::ServiceExt;

    struct Fixture {
        state: ApiState,
        database: Database,
        store_rx: mpsc::Receiver<Event>,
    }

    /// Bind `store_rx`: dropping it, e.g. by leaving it out with `..`, makes the store unavailable.
    fn create_state(groups: Vec<Group>, device_ids: &[&str]) -> Fixture {
        let groups: HashMap<_, _> = groups.into_iter().map(|group| (group.id.clone(), Arc::new(group))).collect();
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        let snapshot = StoreSnapshot { devices: Arc::new(devices), groups: Arc::new(groups), ..Default::default() };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_store_tx(snapshot, store_tx);
        Fixture { state, database, store_rx }
    }

    fn existing_group(id: &str, name: &str) -> Group {
        Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: GroupOwner::Integration,
            external_id: Some("external".to_string()),
        }
    }

    async fn call_create_group(state: ApiState, body: &str) -> Response {
        let request = Request::builder()
            .method("POST")
            .uri("/api/groups")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    async fn stored_group_ids(database: Database) -> Vec<String> {
        let stored = GroupStore::new(database).list().await.expect("list succeeded");
        stored.into_iter().map(|group| group.id).collect()
    }

    #[tokio::test]
    async fn create_group_returns_the_created_group_with_camel_case_fields() {
        let Fixture { state, store_rx: _store_rx, .. } = create_state(vec![], &["a", "b"]);

        let response = call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":["b","a","b"]}"#).await;

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()[header::LOCATION], "/api/groups/kitchen");
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "kitchen",
                "name": "Kitchen",
                "deviceIds": ["a", "b"],
                "revision": 0,
                "owner": "hearth",
            })
        );
    }

    #[tokio::test]
    async fn create_group_persists_the_group_and_sends_it_to_the_store() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["a"]);

        call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":["a"]}"#).await;

        assert_eq!(stored_group_ids(database).await, vec!["kitchen"]);
        let Event::GroupCreated(group) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a GroupCreated event");
        };
        assert_eq!((group.id.as_str(), group.owner), ("kitchen", GroupOwner::Hearth));
    }

    #[tokio::test]
    async fn create_group_returns_400_bad_request_for_an_invalid_body() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &[]);

        let response = call_create_group(state, r#"{"id":"kitchen"}"#).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_returns_409_conflict_for_an_existing_id() {
        let Fixture { state, database, mut store_rx } = create_state(vec![existing_group("kitchen", "Other")], &["a"]);

        let response = call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::CONFLICT, "groupIdAlreadyExists").await;
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_returns_409_conflict_for_a_name_in_use_ignoring_case() {
        let Fixture { state, database, mut store_rx } = create_state(vec![existing_group("other", "Kitchen")], &["a"]);

        let response = call_create_group(state, r#"{"id":"new","name":"kitchen","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::CONFLICT, "groupNameAlreadyExists").await;
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_returns_422_unprocessable_entity_for_a_group_without_devices() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["a"]);

        let response = call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":[]}"#).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "groupEmpty");
        assert_eq!(body["message"], "a group must contain at least one device");
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_returns_422_unprocessable_entity_for_unknown_devices() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["known"]);

        let response =
            call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":["z","known","a"]}"#).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownDevices");
        assert_eq!(body["message"], "unknown device ids: 'a, z'");
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_group_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } = create_state(vec![], &["a"]);
        drop(store_rx);

        let response = call_create_group(state, r#"{"id":"kitchen","name":"Kitchen","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "storeUnavailable").await;
        assert!(stored_group_ids(database).await.is_empty(), "nothing must be stored");
    }
}
