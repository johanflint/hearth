use crate::api::ApiState;
use crate::api::error::ErrorResponse;
use crate::api::parse_request::parse_request;
use crate::api::zones::zone_view::ZoneView;
use crate::zone_service::UpdateZoneError;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use tracing::{error, info};

pub(super) async fn update_zone(
    Path(id): Path<String>,
    State(state): State<ApiState>,
    body: Bytes,
) -> Result<Response, Response> {
    let request: UpdateZoneRequest = parse_request(&body)?;

    let stored_zone = state
        .zone_service
        .update_zone(&id, request.base_revision, &request.name, request.group_ids, request.device_ids)
        .await
        .map_err(update_error_response)?;
    info!("📥 Received request to update zone '{}'... OK, revision {}", stored_zone.id, stored_zone.revision);

    Ok((StatusCode::OK, Json(ZoneView::from(stored_zone))).into_response())
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct UpdateZoneRequest {
    base_revision: u64,
    name: String,
    group_ids: Vec<String>,
    device_ids: Vec<String>,
}

fn update_error_response(err: UpdateZoneError) -> Response {
    match err {
        UpdateZoneError::ZoneEmpty => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("zoneEmpty", err.to_string()))).into_response()
        }
        UpdateZoneError::NotOwned => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new_code_only("zoneNotOwned"))).into_response()
        }
        UpdateZoneError::UnknownGroups { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownGroups", err.to_string())))
                .into_response()
        }
        UpdateZoneError::UnknownDevices { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, Json(ErrorResponse::new("unknownDevices", err.to_string())))
                .into_response()
        }
        UpdateZoneError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorResponse::new_code_only("zoneNotFound"))).into_response()
        }
        UpdateZoneError::RevisionConflict { .. } => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new("revisionConflict", err.to_string()))).into_response()
        }
        UpdateZoneError::NameAlreadyExists => {
            (StatusCode::CONFLICT, Json(ErrorResponse::new_code_only("zoneNameAlreadyExists"))).into_response()
        }
        UpdateZoneError::StoreUnavailable => {
            error!("❌ Store is unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(ErrorResponse::new_code_only("storeUnavailable"))).into_response()
        }
        UpdateZoneError::Internal(_) => {
            error!("❌ Failed to persist zone to store: {err}");
            (StatusCode::INTERNAL_SERVER_ERROR, Json(ErrorResponse::new_code_only("storageError"))).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{body_json, create_state_with_store_tx};
    use crate::api::zones::router;
    use crate::database::{Database, DatabaseError};
    use crate::domain::{Event, Group, Owner, Zone};
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use crate::zone_store::{StoredZone, ZoneStore};
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

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn integration_zone() -> Zone {
        Zone {
            id: "hue".to_string(),
            name: "Hue zone".to_string(),
            group_ids: BTreeSet::new(),
            device_ids: set(&["a"]),
            revision: 0,
            owner: Owner::Integration,
            external_id: Some("external".to_string()),
        }
    }

    /// Creates a state with a stored zone "downstairs" named "Downstairs" with group "g" and device "a".
    /// The snapshot knows groups "g" and "h" and the given devices.
    /// Bind `store_rx`: dropping it, e.g. by leaving it out with `..`, makes the store unavailable.
    async fn create_state(device_ids: &[&str]) -> Fixture {
        create_state_with_zones(device_ids, vec![]).await
    }

    /// Like `create_state`, with the stored zone also in the snapshot next to the other zones
    async fn create_state_with_zones(device_ids: &[&str], other_zones: Vec<Zone>) -> Fixture {
        let devices: HashMap<_, _> =
            device_ids.iter().map(|id| (id.to_string(), Arc::new(DeviceBuilder::new(id).build()))).collect();
        let groups: HashMap<_, _> = ["g", "h"]
            .into_iter()
            .map(|id| {
                let group = Group {
                    id: id.to_string(),
                    name: id.to_string(),
                    device_ids: BTreeSet::new(),
                    revision: 0,
                    owner: Owner::Hearth,
                    external_id: None,
                };
                (id.to_string(), Arc::new(group))
            })
            .collect();
        let downstairs = Zone {
            id: "downstairs".to_string(),
            name: "Downstairs".to_string(),
            group_ids: set(&["g"]),
            device_ids: set(&["a"]),
            revision: 0,
            owner: Owner::Hearth,
            external_id: None,
        };
        let zones: HashMap<_, _> =
            other_zones.into_iter().chain([downstairs]).map(|zone| (zone.id.clone(), Arc::new(zone))).collect();
        let snapshot = StoreSnapshot { devices: Arc::new(devices), groups: Arc::new(groups), zones: Arc::new(zones) };
        let (store_tx, store_rx) = mpsc::channel(8);
        let (state, database) = create_state_with_store_tx(snapshot, store_tx);
        ZoneStore::new(database.clone())
            .insert("downstairs", "Downstairs", &set(&["g"]), &set(&["a"]))
            .await
            .expect("insert succeeded");
        Fixture { state, database, store_rx }
    }

    async fn call_update_zone(state: ApiState, id: &str, body: &str) -> Response {
        let request = Request::builder()
            .method("PUT")
            .uri(format!("/api/zones/{id}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn assert_error(response: Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        assert_eq!(body_json(response).await["code"], code);
    }

    async fn stored_zones(database: Database) -> Vec<StoredZone> {
        ZoneStore::new(database).list().await.expect("list succeeded")
    }

    async fn assert_unchanged(database: Database) {
        let stored = stored_zones(database).await;
        let zone = stored.iter().find(|zone| zone.id == "downstairs").expect("the zone is stored");
        assert_eq!((zone.name.as_str(), zone.revision), ("Downstairs", 0));
        assert_eq!((&zone.group_ids, &zone.device_ids), (&set(&["g"]), &set(&["a"])));
    }

    #[tokio::test]
    async fn update_zone_returns_the_updated_zone_with_camel_case_fields() {
        let Fixture { state, store_rx: _store_rx, .. } = create_state(&["a", "b"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["h","g","h"],"deviceIds":["b","a","b"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "downstairs",
                "name": "Upstairs",
                "groupIds": ["g", "h"],
                "deviceIds": ["a", "b"],
                "revision": 1,
                "owner": "hearth",
            })
        );
    }

    #[tokio::test]
    async fn update_zone_persists_the_zone_and_sends_the_change_to_the_store() {
        let Fixture { state, database, mut store_rx } = create_state(&["a", "b"]).await;

        call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["h"],"deviceIds":["b"]}"#,
        )
        .await;

        let stored = stored_zones(database).await;
        assert_eq!((stored[0].name.as_str(), stored[0].revision), ("Upstairs", 1));
        assert_eq!((&stored[0].group_ids, &stored[0].device_ids), (&set(&["h"]), &set(&["b"])));
        let Event::ZoneChanged(changed) = store_rx.try_recv().expect("an event was sent") else {
            panic!("expected a ZoneChanged event");
        };
        assert_eq!(changed.id, "downstairs");
        assert_eq!(changed.name.as_deref(), Some("Upstairs"));
        assert_eq!(changed.group_ids, Some(set(&["h"])));
        assert_eq!(changed.device_ids, Some(set(&["b"])));
        assert_eq!(changed.revision, Some(1));
    }

    #[rstest::rstest]
    #[case::missing_device_ids(r#"{"baseRevision":0,"name":"Upstairs","groupIds":["g"]}"#)]
    #[case::missing_group_ids(r#"{"baseRevision":0,"name":"Upstairs","deviceIds":["a"]}"#)]
    #[case::missing_name(r#"{"baseRevision":0,"groupIds":["g"],"deviceIds":["a"]}"#)]
    #[case::missing_revision(r#"{"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#)]
    #[case::negative_revision(r#"{"baseRevision":-1,"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#)]
    #[case::not_json("nope")]
    #[tokio::test]
    async fn update_zone_returns_400_bad_request_for_an_invalid_body(#[case] body: &str) {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(state, "downstairs", body).await;

        assert_error(response, StatusCode::BAD_REQUEST, "invalidJson").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_404_not_found_for_an_unknown_zone() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "unknown",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_error(response, StatusCode::NOT_FOUND, "zoneNotFound").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_409_conflict_for_a_stale_revision() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":3,"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(body["code"], "revisionConflict");
        assert!(body["message"].as_str().is_some_and(|message| !message.is_empty()));
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_409_conflict_for_a_name_in_use_ignoring_case() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;
        ZoneStore::new(database.clone()).insert("upstairs", "Upstairs", &set(&["g"]), &set(&[])).await.unwrap();

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"uPSTAIRS","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_error(response, StatusCode::CONFLICT, "zoneNameAlreadyExists").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_422_unprocessable_entity_for_a_zone_owned_by_an_integration() {
        let Fixture { state, database, mut store_rx } = create_state_with_zones(&["a"], vec![integration_zone()]).await;

        let response =
            call_update_zone(state, "hue", r#"{"baseRevision":0,"name":"Renamed","groupIds":["g"],"deviceIds":["a"]}"#)
                .await;

        assert_error(response, StatusCode::UNPROCESSABLE_ENTITY, "zoneNotOwned").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_409_conflict_for_a_name_in_use_by_an_integration_zone() {
        let Fixture { state, database, mut store_rx } = create_state_with_zones(&["a"], vec![integration_zone()]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"hue ZONE","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_error(response, StatusCode::CONFLICT, "zoneNameAlreadyExists").await;
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_allows_the_zone_to_keep_its_own_name_when_it_is_in_the_store() {
        let Fixture { state, store_rx: _store_rx, .. } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"DOWNSTAIRS","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["name"], "DOWNSTAIRS");
    }

    #[tokio::test]
    async fn update_zone_returns_500_internal_server_error_when_the_database_fails() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;
        database
            .call(|conn| conn.execute("DROP TABLE zones", []).map_err(DatabaseError::from))
            .await
            .expect("drop the table");

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_error(response, StatusCode::INTERNAL_SERVER_ERROR, "storageError").await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_422_unprocessable_entity_for_a_zone_without_groups_and_devices() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":[],"deviceIds":[]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "zoneEmpty");
        assert_eq!(body["message"], "a zone must contain at least one group or device");
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_422_unprocessable_entity_for_unknown_groups() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["z","g","b"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownGroups");
        assert_eq!(body["message"], "unknown group ids: 'b, z'");
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_422_unprocessable_entity_for_unknown_devices() {
        let Fixture { state, database, mut store_rx } = create_state(&["a"]).await;

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["g"],"deviceIds":["z","a","b"]}"#,
        )
        .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = body_json(response).await;
        assert_eq!(body["code"], "unknownDevices");
        assert_eq!(body["message"], "unknown device ids: 'b, z'");
        assert_unchanged(database).await;
        assert!(store_rx.try_recv().is_err(), "no event must be sent");
    }

    #[tokio::test]
    async fn update_zone_returns_503_service_unavailable_when_the_store_is_gone() {
        let Fixture { state, database, store_rx } = create_state(&["a"]).await;
        drop(store_rx);

        let response = call_update_zone(
            state,
            "downstairs",
            r#"{"baseRevision":0,"name":"Upstairs","groupIds":["g"],"deviceIds":["a"]}"#,
        )
        .await;

        assert_error(response, StatusCode::SERVICE_UNAVAILABLE, "storeUnavailable").await;
        assert_unchanged(database).await;
    }
}
