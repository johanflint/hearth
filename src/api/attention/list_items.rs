use crate::api::ApiState;
use crate::attention::{Attention, AttentionList, Issue, Subject};
use crate::services::{DeviceService, FlowService};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::Serialize;

pub(super) async fn list_items(State(state): State<ApiState>) -> Result<Response, Response> {
    let response = attention_response(&state.attention_service.list(), &state.device_service, &state.flow_service);

    Ok((StatusCode::OK, Json(response)).into_response())
}

pub(super) fn attention_response(
    list: &AttentionList,
    device_service: &DeviceService,
    flow_service: &FlowService,
) -> AttentionResponse {
    let items = list.iter().map(|attention| AttentionView::new(attention, &device_service, &flow_service)).collect();
    AttentionResponse { items }
}

#[derive(Debug, Serialize)]
pub(super) struct AttentionResponse {
    items: Vec<AttentionView>,
}

#[derive(Debug, Serialize)]
struct AttentionView {
    subject: SubjectView,
    issue: IssueView,
    since: DateTime<Utc>,
}

impl AttentionView {
    fn new(attention: &Attention, device_service: &DeviceService, flow_service: &FlowService) -> AttentionView {
        AttentionView {
            subject: SubjectView::new(&attention.subject, device_service, flow_service),
            issue: (&attention.issue).into(),
            since: attention.since,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum SubjectView {
    Device { id: String, name: Option<String> },
    Flow { id: String, name: Option<String> },
    Integration { name: String },
}

impl SubjectView {
    fn new(subject: &Subject, device_service: &DeviceService, flow_service: &FlowService) -> SubjectView {
        match subject {
            Subject::Device(id) => SubjectView::Device {
                id: id.clone(),
                // The device can disappear between the attention snapshot and this lookup
                name: device_service.retrieve_device(id).ok().map(|device| device.name.clone()),
            },
            Subject::Flow(id) => SubjectView::Flow {
                id: id.clone(),
                name: flow_service.registered_flow(id).map(|flow| flow.name().to_string()),
            },
            Subject::Integration(name) => SubjectView::Integration { name: name.clone() },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum IssueView {
    Connectivity { state: String },
    Battery { state: String },
    Flow { error: String },
    Integration { state: String },
}

impl From<&Issue> for IssueView {
    fn from(issue: &Issue) -> Self {
        match issue {
            Issue::Connectivity(connectivity) => IssueView::Connectivity { state: connectivity.to_string() },
            Issue::Battery(battery_state) => IssueView::Battery { state: battery_state.to_string() },
            Issue::FlowFailed { error } => IssueView::Flow { error: error.to_string() },
            Issue::Integration(integration_status) => IssueView::Integration { state: integration_status.to_string() },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::attention::router;
    use crate::api::test_support::{
        VALID_FLOW_ID, VALID_FLOW_JSON, body_json, create_state_with_attention, create_state_with_flows,
    };
    use crate::attention::AttentionList;
    use crate::domain::{BatteryState, Connectivity, Device};
    use crate::flow_engine::VersionedFlow;
    use crate::flow_loader;
    use crate::flow_loader::SerializedFlow;
    use crate::flow_registry::FlowRegistry;
    use crate::integration::IntegrationStatus;
    use crate::services::AttentionService;
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::TimeZone;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn snapshot_with_devices(devices: Vec<Device>) -> StoreSnapshot {
        let devices: HashMap<_, _> = devices.into_iter().map(|device| (device.id.clone(), Arc::new(device))).collect();
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn attention(device_id: &str, issue: Issue) -> Attention {
        Attention {
            subject: Subject::Device(device_id.into()),
            issue,
            since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        }
    }

    fn integration_attention(name: &str, status: IntegrationStatus) -> Attention {
        Attention {
            subject: Subject::Integration(name.into()),
            issue: Issue::Integration(status),
            since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
        }
    }

    fn devices(items: Vec<Attention>) -> AttentionList {
        AttentionList { devices: items.into(), ..Default::default() }
    }

    async fn call_list_items(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/attention").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn list_items_returns_no_items_when_nothing_needs_attention() {
        let state = create_state_with_attention(StoreSnapshot::default(), AttentionList::default());

        let response = call_list_items(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "items": [] }));
    }

    #[tokio::test]
    async fn list_items_returns_the_items_with_the_name_of_the_device() {
        let snapshot = snapshot_with_devices(vec![
            Device { name: "Kitchen lamp".into(), ..DeviceBuilder::new("1").build() },
            Device { name: "Door sensor".into(), ..DeviceBuilder::new("2").build() },
        ]);
        let items = vec![
            attention("1", Issue::Connectivity(Connectivity::Disconnected)),
            attention("2", Issue::Battery(BatteryState::Low)),
        ];

        let response = call_list_items(create_state_with_attention(snapshot, devices(items))).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "items": [
                    {
                        "subject": { "type": "device", "id": "1", "name": "Kitchen lamp" },
                        "issue": { "type": "connectivity", "state": "disconnected" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                    {
                        "subject": { "type": "device", "id": "2", "name": "Door sensor" },
                        "issue": { "type": "battery", "state": "low" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                ]
            })
        );
    }

    #[tokio::test]
    async fn list_items_returns_the_integrations_after_the_devices() {
        let snapshot =
            snapshot_with_devices(vec![Device { name: "Door sensor".into(), ..DeviceBuilder::new("1").build() }]);
        let list = AttentionList {
            devices: vec![attention("1", Issue::Battery(BatteryState::Low))].into(),
            integrations: vec![
                integration_attention("hue", IntegrationStatus::Down),
                integration_attention("shelly", IntegrationStatus::Degraded),
            ]
            .into(),
            ..Default::default()
        };

        let response = call_list_items(create_state_with_attention(snapshot, list)).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "items": [
                    {
                        "subject": { "type": "device", "id": "1", "name": "Door sensor" },
                        "issue": { "type": "battery", "state": "low" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                    {
                        "subject": { "type": "integration", "name": "hue" },
                        "issue": { "type": "integration", "state": "down" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                    {
                        "subject": { "type": "integration", "name": "shelly" },
                        "issue": { "type": "integration", "state": "degraded" },
                        "since": "2026-01-01T12:00:00Z",
                    },
                ]
            })
        );
    }

    #[tokio::test]
    async fn list_items_returns_the_failed_flows_between_the_devices_and_the_integrations() {
        let list = AttentionList {
            flows: vec![Attention {
                subject: Subject::Flow("night-light".into()),
                issue: Issue::FlowFailed { error: "broken".into() },
                since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
            }]
            .into(),
            integrations: vec![integration_attention("hue", IntegrationStatus::Down)].into(),
            ..Default::default()
        };

        let response = call_list_items(create_state_with_attention(StoreSnapshot::default(), list)).await;

        let body = body_json(response).await;
        assert_eq!(
            body["items"][0],
            json!({
                "subject": { "type": "flow", "id": "night-light", "name": null },
                "issue": { "type": "flow", "error": "broken" },
                "since": "2026-01-01T12:00:00Z",
            })
        );
        assert_eq!(body["items"][1]["subject"]["type"], "integration");
    }

    #[tokio::test]
    async fn list_items_returns_the_name_of_a_registered_flow() {
        let payload: SerializedFlow = serde_json::from_str(VALID_FLOW_JSON).unwrap();
        let flow = VersionedFlow { flow: Arc::new(flow_loader::from_json(payload).unwrap()), revision: 1 };
        let (store_tx, _store_rx) = tokio::sync::mpsc::channel(8);
        let (mut state, _database) =
            create_state_with_flows(FlowRegistry::new(vec![flow]), StoreSnapshot::default(), store_tx);
        let list = AttentionList {
            flows: vec![Attention {
                subject: Subject::Flow(VALID_FLOW_ID.into()),
                issue: Issue::FlowFailed { error: "broken".into() },
                since: Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap(),
            }]
            .into(),
            ..Default::default()
        };
        let (_, attention_rx) = tokio::sync::watch::channel(list);
        state.attention_service = Arc::new(AttentionService::new(attention_rx));

        let response = call_list_items(state).await;

        let body = body_json(response).await;
        assert_eq!(body["items"][0]["subject"], json!({ "type": "flow", "id": VALID_FLOW_ID, "name": "logFlow" }));
    }

    #[tokio::test]
    async fn list_items_returns_a_null_name_for_a_device_that_is_gone() {
        let items = vec![attention("gone", Issue::Battery(BatteryState::Critical))];

        let response = call_list_items(create_state_with_attention(StoreSnapshot::default(), devices(items))).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await["items"][0]["subject"],
            json!({ "type": "device", "id": "gone", "name": null })
        );
    }
}
