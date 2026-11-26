use crate::api::ApiState;
use crate::api::groups::group_view::GroupView;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub(super) async fn list_groups(State(state): State<ApiState>) -> Result<Response, Response> {
    let groups: Vec<GroupView> = state.group_service.list().into_iter().map(GroupView::from).collect();

    Ok((StatusCode::OK, Json(ListGroupsResponse { groups })).into_response())
}

#[derive(Debug, Serialize)]
struct ListGroupsResponse {
    groups: Vec<GroupView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::groups::router;
    use crate::api::test_support::{body_json, create_state, create_state_with_snapshot};
    use crate::domain::{Group, Owner};
    use crate::flow_registry::FlowRegistry;
    use crate::store::StoreSnapshot;
    use axum::body::Body;
    use axum::http::Request;
    use rstest::rstest;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn call_list_groups(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/groups").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn group(id: &str, name: &str, revision: u64, owner: Owner) -> Group {
        Group {
            id: id.to_string(),
            name: name.to_string(),
            device_ids: BTreeSet::from(["device".to_string()]),
            revision,
            owner,
            external_id: Some("external".to_string()),
        }
    }

    fn snapshot(groups: Vec<Group>) -> StoreSnapshot {
        let groups: HashMap<_, _> = groups.into_iter().map(|group| (group.id.clone(), Arc::new(group))).collect();
        StoreSnapshot { groups: Arc::new(groups), ..Default::default() }
    }

    #[tokio::test]
    async fn list_groups_returns_an_empty_list_when_there_are_no_groups() {
        let state = create_state(FlowRegistry::new(vec![])).state;

        let response = call_list_groups(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "groups": [] }));
    }

    #[tokio::test]
    async fn list_groups_returns_the_group_with_camel_case_fields() {
        let state =
            create_state_with_snapshot(snapshot(vec![group("living-room", "Living room", 3, Owner::Integration)]));

        let response = call_list_groups(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "groups": [{
                    "id": "living-room",
                    "name": "Living room",
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
    async fn list_groups_returns_the_owner_of_the_group(#[case] owner: Owner, #[case] expected: &str) {
        let state = create_state_with_snapshot(snapshot(vec![group("group", "Group", 0, owner)]));

        let response = call_list_groups(state).await;

        assert_eq!(body_json(response).await["groups"][0]["owner"], expected);
    }

    #[rstest]
    #[case::none(&[], json!([]))]
    #[case::sorted(&["b", "c", "a"], json!(["a", "b", "c"]))]
    #[tokio::test]
    async fn list_groups_returns_the_sorted_device_ids_of_the_group(
        #[case] device_ids: &[&str],
        #[case] expected: serde_json::Value,
    ) {
        let device_ids = device_ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>();
        let state = create_state_with_snapshot(snapshot(vec![Group {
            device_ids,
            ..group("group", "Group", 0, Owner::Hearth)
        }]));

        let response = call_list_groups(state).await;

        assert_eq!(body_json(response).await["groups"][0]["deviceIds"], expected);
    }

    #[tokio::test]
    async fn list_groups_returns_the_groups_ordered_by_name() {
        let state = create_state_with_snapshot(snapshot(vec![
            group("1", "Living room", 0, Owner::Integration),
            group("2", "Bedroom", 0, Owner::Hearth),
            group("3", "Kitchen", 0, Owner::Integration),
        ]));

        let response = call_list_groups(state).await;

        let body = body_json(response).await;
        let names: Vec<_> =
            body["groups"].as_array().expect("groups array").iter().map(|g| g["name"].clone()).collect();
        assert_eq!(names, vec![json!("Bedroom"), json!("Kitchen"), json!("Living room")]);
    }
}
