use crate::api::ApiState;
use crate::api::devices::device_view::DeviceView;
use crate::api::devices::property_view::PropertyView;
use crate::domain::Device;
use crate::services::DeviceService;
use crate::store::{DeviceUpdate, PropertyChange};
use axum::extract::State;
use axum::response::Sse;
use axum::response::sse::{Event, KeepAlive};
use futures::{Stream, StreamExt, stream};
use serde::Serialize;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

const SNAPSHOT_EVENT_NAME: &str = "snapshot";
const PROPERTY_CHANGED_EVENT_NAME: &str = "propertyChanged";

pub(super) async fn device_events(
    State(state): State<ApiState>,
) -> Sse<impl Stream<Item = Result<Event, axum::Error>>> {
    let device_service = state.device_service;
    // Subscribe before reading the devices, so a change in between is sent twice instead of never
    let rx = device_service.subscribe();
    let snapshot = snapshot_event(&device_service);

    let updates = BroadcastStream::new(rx).map(move |update| match update {
        Ok(update) => update_event(&device_service, update),
        Err(BroadcastStreamRecvError::Lagged(_)) => snapshot_event(&device_service),
    });

    Sse::new(stream::once(async move { snapshot }).chain(updates)).keep_alive(KeepAlive::default())
}

fn snapshot_event(device_service: &DeviceService) -> Result<Event, axum::Error> {
    let devices = device_service.list().into_iter().map(DeviceView::from).collect();
    Event::default().event(SNAPSHOT_EVENT_NAME).json_data(SnapshotView { devices })
}

fn update_event(device_service: &DeviceService, update: DeviceUpdate) -> Result<Event, axum::Error> {
    match update {
        DeviceUpdate::PropertyChanged { device, changed } => property_changed_event(device_service, &device, changed),
        DeviceUpdate::DevicesDiscovered => snapshot_event(device_service),
    }
}

fn property_changed_event(
    device_service: &DeviceService,
    device: &Device,
    changed: PropertyChange,
) -> Result<Event, axum::Error> {
    let property = device.properties.get(&changed.property_id);
    let Some(property) = property else {
        // Can't happen, the store only reports changes to existing properties
        return snapshot_event(device_service);
    };

    let view = PropertyChangedView {
        device_id: changed.device_id,
        property_id: changed.property_id,
        property: PropertyView::from(property.as_ref()),
    };

    Event::default().event(PROPERTY_CHANGED_EVENT_NAME).json_data(view)
}

#[derive(Debug, Serialize)]
struct SnapshotView {
    devices: Vec<DeviceView>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PropertyChangedView {
    device_id: String,
    property_id: String,
    property: PropertyView,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::devices::router;
    use crate::api::devices::test_support::{device, snapshot_with_devices};
    use crate::api::test_support::{body_json, create_state_with_snapshot};
    use crate::domain::{Device, InitiatedBy};
    use crate::store::StoreSnapshot;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use axum::response::Response;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{broadcast, watch};
    use tokio::time::{Instant, timeout};
    use tower::ServiceExt;

    struct Fixture {
        state: ApiState,
        snapshot_tx: watch::Sender<StoreSnapshot>,
        updates_tx: broadcast::Sender<DeviceUpdate>,
    }

    /// The API state with the devices in the store, and control over the updates its clients receive
    fn fixture(devices: Vec<Device>) -> Fixture {
        fixture_with_capacity(devices, 8)
    }

    fn fixture_with_capacity(devices: Vec<Device>, capacity: usize) -> Fixture {
        let snapshot = snapshot_with_devices(devices);
        let (snapshot_tx, snapshot_rx) = watch::channel(snapshot.clone());
        let (updates_tx, updates_rx) = broadcast::channel(capacity);
        let mut state = create_state_with_snapshot(snapshot);
        state.device_service = Arc::new(DeviceService::new(snapshot_rx, updates_rx));
        Fixture { state, snapshot_tx, updates_tx }
    }

    async fn call_device_events(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/devices/events").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    async fn next_frame(body: &mut Body) -> Option<String> {
        let frame = timeout(Duration::from_secs(1), body.frame()).await.expect("expected a frame within a second")?;
        Some(String::from_utf8(frame.unwrap().into_data().unwrap().to_vec()).unwrap())
    }

    /// The name and data of the next event
    async fn next_event(body: &mut Body) -> (String, Value) {
        let text = next_frame(body).await.expect("expected the stream to be open");
        let name = text.lines().find_map(|line| line.strip_prefix("event: ")).expect("expected an event name");
        let data = text.lines().find_map(|line| line.strip_prefix("data: ")).expect("expected event data");
        (name.to_string(), serde_json::from_str(data).unwrap())
    }

    fn lamp(on: bool) -> Device {
        DeviceBuilder::new("lamp").with_boolean_property("on", on).build()
    }

    fn property_changed(device: Device, property_id: &str) -> DeviceUpdate {
        DeviceUpdate::PropertyChanged {
            changed: PropertyChange {
                device_id: device.id.clone(),
                property_id: property_id.to_string(),
                initiated_by: InitiatedBy::External,
            },
            device: Arc::new(device),
        }
    }

    #[tokio::test]
    async fn device_events_responds_with_an_event_stream() {
        let response = call_device_events(fixture(vec![]).state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/event-stream");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
    }

    #[tokio::test]
    async fn device_events_starts_with_an_empty_snapshot_without_devices() {
        let mut body = call_device_events(fixture(vec![]).state).await.into_body();

        assert_eq!(next_event(&mut body).await, ("snapshot".to_string(), json!({ "devices": [] })));
    }

    #[tokio::test]
    async fn device_events_starts_with_a_snapshot_like_the_device_list() {
        let fixture = fixture(vec![device("2", "Lamp"), device("1", "Ceiling"), lamp(true)]);
        let list_request = Request::builder().method("GET").uri("/api/devices").body(Body::empty()).unwrap();
        let list = body_json(router().with_state(fixture.state.clone()).oneshot(list_request).await.unwrap()).await;

        let mut body = call_device_events(fixture.state).await.into_body();

        assert_eq!(next_event(&mut body).await, ("snapshot".to_string(), list));
    }

    #[tokio::test]
    async fn device_events_sends_the_changed_property() {
        let fixture = fixture(vec![lamp(false)]);
        let mut body = call_device_events(fixture.state).await.into_body();
        next_event(&mut body).await;

        fixture.updates_tx.send(property_changed(lamp(true), "on")).unwrap();

        assert_eq!(
            next_event(&mut body).await,
            (
                "propertyChanged".to_string(),
                json!({
                    "deviceId": "lamp",
                    "propertyId": "on",
                    "property": { "name": "on", "propertyType": "on", "readonly": false, "kind": "boolean", "value": true }
                })
            )
        );
    }

    #[tokio::test]
    async fn device_events_sends_the_value_of_the_update_instead_of_the_latest_value() {
        // Clients receive every intermediate value, in order
        let fixture = fixture(vec![lamp(false)]);
        let mut body = call_device_events(fixture.state).await.into_body();
        next_event(&mut body).await;

        fixture.updates_tx.send(property_changed(lamp(true), "on")).unwrap();
        fixture.updates_tx.send(property_changed(lamp(false), "on")).unwrap();

        assert_eq!(next_event(&mut body).await.1["property"]["value"], true);
        assert_eq!(next_event(&mut body).await.1["property"]["value"], false);
    }

    #[tokio::test]
    async fn device_events_sends_a_snapshot_when_devices_are_discovered() {
        let fixture = fixture(vec![device("1", "Ceiling")]);
        let mut body = call_device_events(fixture.state).await.into_body();
        next_event(&mut body).await;

        fixture.snapshot_tx.send(snapshot_with_devices(vec![device("1", "Ceiling"), device("2", "Lamp")])).unwrap();
        fixture.updates_tx.send(DeviceUpdate::DevicesDiscovered).unwrap();

        let (name, data) = next_event(&mut body).await;
        assert_eq!(name, "snapshot");
        assert_eq!(data["devices"][0]["id"], "1");
        assert_eq!(data["devices"][1]["id"], "2");
    }

    #[tokio::test]
    async fn device_events_sends_a_snapshot_when_the_changed_property_is_unknown() {
        let fixture = fixture(vec![lamp(false)]);
        let mut body = call_device_events(fixture.state).await.into_body();
        next_event(&mut body).await;

        fixture.updates_tx.send(property_changed(lamp(true), "unknown")).unwrap();

        let (name, data) = next_event(&mut body).await;
        assert_eq!(name, "snapshot");
        assert_eq!(data["devices"][0]["id"], "lamp");
    }

    #[tokio::test]
    async fn device_events_sends_a_snapshot_when_the_client_missed_updates() {
        let fixture = fixture_with_capacity(vec![lamp(false)], 1);
        let mut body = call_device_events(fixture.state).await.into_body();
        next_event(&mut body).await;

        // The second update pushes the first out of the full channel before the client reads it
        fixture.updates_tx.send(property_changed(lamp(true), "on")).unwrap();
        fixture.updates_tx.send(property_changed(lamp(false), "on")).unwrap();

        assert_eq!(next_event(&mut body).await.0, "snapshot");
        assert_eq!(next_event(&mut body).await.0, "propertyChanged");
    }

    #[tokio::test]
    async fn device_events_ends_when_the_store_stops() {
        let Fixture { state, updates_tx, .. } = fixture(vec![]);
        let mut body = call_device_events(state).await.into_body();
        next_event(&mut body).await;

        drop(updates_tx);

        assert_eq!(next_frame(&mut body).await, None);
    }

    #[tokio::test(start_paused = true)]
    async fn device_events_sends_a_keep_alive_while_idle() {
        let fixture = fixture(vec![]);
        let mut body = call_device_events(fixture.state).await.into_body();
        body.frame().await.unwrap().unwrap();
        let start = Instant::now();

        // No timeout, the paused clock skips ahead to the keep-alive
        let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();

        assert_eq!(frame, ":\n\n");
        assert_eq!(start.elapsed(), Duration::from_secs(15));
    }
}
