use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::parse_request::parse_request;
use crate::api::zones::zone_view::ZoneView;
use crate::services::CreateZoneError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn create_zone(State(state): State<ApiState>, body: Bytes) -> Result<Response, Response> {
    let request: CreateZoneRequest = parse_request(&body)?;
    let stored_zone = state
        .zone_service
        .create_zone(&request.id, &request.name, request.group_ids, request.device_ids)
        .await
        .map_err(create_error_response)?;
    info!("📥 Received request to create zone '{}'... OK, revision {}", stored_zone.id, stored_zone.revision);

    let location = format!("/api/zones/{}", stored_zone.id);
    Ok((StatusCode::CREATED, [(header::LOCATION, location)], Json(ZoneView::from(stored_zone))).into_response())
}

fn create_error_response(err: CreateZoneError) -> Response {
    match err {
        CreateZoneError::IdAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("zoneIdAlreadyExists"))).into_response()
        }
        CreateZoneError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("zoneNameAlreadyExists"))).into_response()
        }
        CreateZoneError::ZoneEmpty => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("zoneEmpty", err.to_string()))).into_response()
        }
        CreateZoneError::UnknownGroups { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownGroups", err.to_string())))
                .into_response()
        }
        CreateZoneError::UnknownDevices { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownDevices", err.to_string())))
                .into_response()
        }
        CreateZoneError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        CreateZoneError::Internal(_) => {
            error!("❌ Failed to persist zone to the repository: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateZoneRequest {
    id: String,
    name: String,
    group_ids: Vec<String>,
    device_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{body_json, create_state_with_store_tx};
    use crate::api::zones::router;
    use crate::database::{Database, DatabaseError};
    use crate::domain::{Event, Group, Owner, Zone};
    use crate::repositories::ZoneRepository;
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
    fn create_state(zones: Vec<Zone>, group_ids: &[&str], device_ids: &[&str]) -> Fixture {
        let zones: HashMap<_, _> = zones.into_iter().map(|zone| (zone.id.clone(), Arc::new(zone))).collect();
        let groups: HashMap<_, _> = group_ids.iter().map(|id| (id.to_string(), Arc::new(group(id)))).collect();
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        let snapshot = StoreSnapshot { zones: Arc::new(zones), groups: Arc::new(groups), devices: Arc::new(devices) };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_store_tx(snapshot, store_tx);
        Fixture { state, database, store_rx }
    }

    fn group(id: &str) -> Group {
        Group {
            id: id.to_string(),
            name: id.to_string(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: Owner::Hearth,
            external_id: None,
        }
    }

    fn existing_zone(id: &str, name: &str) -> Zone {
        Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: BTreeSet::new(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: Owner::Integration,
            external_id: Some("external".to_string()),
        }
    }

    async fn call_create_zone(state: ApiState, body: &str) -> Response {
        let request = Request::builder()
            .method("POST")
            .uri("/api/zones")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    async fn stored_zone_ids(database: Database) -> Vec<String> {
        let stored = ZoneRepository::new(database).list().await.expect("list succeeded");
        stored.into_iter().map(|zone| zone.id).collect()
    }

    #[tokio::test]
    async fn create_zone_returns_the_created_zone_with_camel_case_fields() {
        let Fixture { state, store_rx: _store_rx, .. } = create_state(vec![], &["g1", "g2"], &["a", "b"]);

        let response = call_create_zone(
            state,
            r#"{"id":"downstairs","name":"Downstairs","groupIds":["g2","g1","g2"],"deviceIds":["b","a","b"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()[header::LOCATION], "/api/zones/downstairs");
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "downstairs",
                "name": "Downstairs",
                "groupIds": ["g1", "g2"],
                "deviceIds": ["a", "b"],
                "revision": 0,
                "owner": "hearth",
            })
        );
    }

    #[tokio::test]
    async fn create_zone_persists_the_zone_and_sends_it_to_the_store() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["g"], &["a"]);

        call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","groupIds":["g"],"deviceIds":["a"]}"#).await;

        assert_eq!(stored_zone_ids(database).await, vec!["downstairs"]);
        let Event::ZoneCreated(zone) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a ZoneCreated event");
        };
        assert_eq!((zone.id.as_str(), zone.owner), ("downstairs", Owner::Hearth));
    }

    #[tokio::test]
    async fn create_zone_returns_400_bad_request_for_an_invalid_body() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &[], &[]);

        let response = call_create_zone(state, r#"{"id":"downstairs"}"#).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_400_bad_request_when_group_ids_or_device_ids_are_missing() {
        let Fixture { state, database, store_rx: _store_rx } = create_state(vec![], &[], &["a"]);

        let response = call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
    }

    #[tokio::test]
    async fn create_zone_returns_409_conflict_for_an_existing_id() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![existing_zone("downstairs", "Other")], &[], &["a"]);

        let response =
            call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","groupIds":[],"deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::CONFLICT, "zoneIdAlreadyExists").await;
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_409_conflict_for_a_name_in_use_ignoring_case() {
        let Fixture { state, database, mut store_rx } =
            create_state(vec![existing_zone("other", "Downstairs")], &[], &["a"]);

        let response =
            call_create_zone(state, r#"{"id":"new","name":"downstairs","groupIds":[],"deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::CONFLICT, "zoneNameAlreadyExists").await;
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_422_unprocessable_entity_for_a_zone_without_groups_and_devices() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["g"], &["a"]);

        let response =
            call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","groupIds":[],"deviceIds":[]}"#).await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "zoneEmpty");
        assert_eq!(body["message"], "a zone must contain at least one group or device");
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_422_unprocessable_entity_for_unknown_groups() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &["known"], &[]);

        let response = call_create_zone(
            state,
            r#"{"id":"downstairs","name":"Downstairs","groupIds":["z","known","a"],"deviceIds":[]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownGroups");
        assert_eq!(body["message"], "unknown group ids: 'a, z'");
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_422_unprocessable_entity_for_unknown_devices() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &[], &["known"]);

        let response = call_create_zone(
            state,
            r#"{"id":"downstairs","name":"Downstairs","groupIds":[],"deviceIds":["z","known","a"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownDevices");
        assert_eq!(body["message"], "unknown device ids: 'a, z'");
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_500_internal_server_error_when_the_database_fails() {
        let Fixture { state, database, mut store_rx } = create_state(vec![], &[], &["a"]);
        database
            .call(|conn| conn.execute("DROP TABLE zones", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response =
            call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","groupIds":[],"deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::INTERNAL_SERVER_ERROR, "storageError").await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn create_zone_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } = create_state(vec![], &[], &["a"]);
        drop(store_rx);

        let response =
            call_create_zone(state, r#"{"id":"downstairs","name":"Downstairs","groupIds":[],"deviceIds":["a"]}"#).await;

        assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "storeUnavailable").await;
        assert!(stored_zone_ids(database).await.is_empty(), "nothing must be stored");
    }
}
