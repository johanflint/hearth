use crate::api::ApiState;
use crate::api::zones::zone_view::ZoneView;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub(super) async fn list_zones(State(state): State<ApiState>) -> Result<Response, Response> {
    let zones: Vec<_> = state.zone_service.list().into_iter().map(ZoneView::from).collect();

    Ok((StatusCode::OK, Json(ListZonesResponse { zones })).into_response())
}

#[derive(Debug, Serialize)]
struct ListZonesResponse {
    zones: Vec<ZoneView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::{body_json, create_state, create_state_with_snapshot};
    use crate::api::zones::router;
    use crate::domain::{Owner, Zone};
    use crate::flow_registry::FlowRegistry;
    use crate::store::StoreSnapshot;
    use axum::body::Body;
    use axum::http::Request;
    use rstest::rstest;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn call_list_zones(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/zones").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn zone(id: &str, name: &str, revision: u64, owner: Owner) -> Zone {
        Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: ids(&["group"]),
            device_ids: ids(&["device"]),
            revision,
            owner,
            external_id: Some("external".to_string()),
        }
    }

    fn snapshot(zones: Vec<Zone>) -> StoreSnapshot {
        let zones: HashMap<_, _> = zones.into_iter().map(|zone| (zone.id.clone(), Arc::new(zone))).collect();
        StoreSnapshot { zones: Arc::new(zones), ..Default::default() }
    }

    #[tokio::test]
    async fn list_zones_returns_an_empty_list_when_there_are_no_zones() {
        let state = create_state(FlowRegistry::new(vec![])).state;

        let response = call_list_zones(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "zones": [] }));
    }

    #[tokio::test]
    async fn list_zones_returns_the_zone_with_camel_case_fields() {
        let state = create_state_with_snapshot(snapshot(vec![zone("downstairs", "Downstairs", 3, Owner::Integration)]));

        let response = call_list_zones(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "zones": [{
                    "id": "downstairs",
                    "name": "Downstairs",
                    "groupIds": ["group"],
                    "deviceIds": ["device"],
                    "revision": 3,
                    "owner": "integration",
                }]
            })
        );
    }

    #[rstest]
    #[case::hearth(Owner::Hearth, "hearth")]
    #[case::integration(Owner::Integration, "integration")]
    #[tokio::test]
    async fn list_zones_returns_the_owner_of_the_zone(#[case] owner: Owner, #[case] expected: &str) {
        let state = create_state_with_snapshot(snapshot(vec![zone("zone", "Zone", 0, owner)]));

        let response = call_list_zones(state).await;

        assert_eq!(body_json(response).await["zones"][0]["owner"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn list_zones_returns_the_sorted_group_ids_of_the_zone(
        #[case] group_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let state = create_state_with_snapshot(snapshot(vec![Zone {
            group_ids: ids(group_ids),
            ..zone("zone", "Zone", 0, Owner::Hearth)
        }]));

        let response = call_list_zones(state).await;

        assert_eq!(body_json(response).await["zones"][0]["groupIds"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn list_zones_returns_the_sorted_device_ids_of_the_zone(
        #[case] device_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let state = create_state_with_snapshot(snapshot(vec![Zone {
            device_ids: ids(device_ids),
            ..zone("zone", "Zone", 0, Owner::Hearth)
        }]));

        let response = call_list_zones(state).await;

        assert_eq!(body_json(response).await["zones"][0]["deviceIds"], expected);
    }

    #[tokio::test]
    async fn list_zones_returns_the_zones_ordered_by_name() {
        let state = create_state_with_snapshot(snapshot(vec![
            zone("1", "Upstairs", 0, Owner::Integration),
            zone("2", "Garden", 0, Owner::Hearth),
            zone("3", "Downstairs", 0, Owner::Integration),
        ]));

        let response = call_list_zones(state).await;

        let body = body_json(response).await;
        let names: Vec<_> = body["zones"].as_array().expect("zones array").iter().map(|z| z["name"].clone()).collect();
        assert_eq!(names, vec![json!("Downstairs"), json!("Garden"), json!("Upstairs")]);
    }
}
