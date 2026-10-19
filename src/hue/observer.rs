use crate::app_config::AppConfig;
use crate::domain::{Event, InitiatedBy};
use crate::hue::corrections::is_unnoticeable_correction;
use crate::hue::domain::{ChangedProperty, DataType, LightChanged, ServerSentEventPayload, UnknownProperty};
use crate::hue::grouped_light_index::GroupedLightIndex;
use crate::hue::map_connectivity_changed::map_connectivity_changed;
use crate::hue::map_contact_sensors_changed::map_contact_sensors_changed;
use crate::hue::map_device_power_changed::map_device_power_changed;
use crate::hue::map_light_changed::map_light_changed_property;
use crate::hue::map_light_level_changed::map_light_level_changed;
use crate::hue::map_motion_sensors_changed::map_motion_sensors_changed;
use crate::hue::map_remotes_changed::map_remote_changed;
use crate::hue::map_temperature_changed::map_temperature_changed;
use crate::hue::pending_writes::PendingWrites;
use crate::integration::IntegrationError;
use crate::sse;
use crate::sse::{Config, ServerSentEvent};
use crate::store::StoreSnapshot;
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tokio::task::JoinError;
use tracing::{debug, info, instrument, trace, warn};

type HueEvent = ServerSentEvent<Vec<ServerSentEventPayload>>;

#[instrument(skip_all)]
pub async fn observe(
    tx: Sender<Event>,
    pending_writes: Arc<PendingWrites>,
    grouped_light_index: Arc<GroupedLightIndex>,
    store_rx: WatchReceiver<StoreSnapshot>,
    client: &Client,
    config: &AppConfig,
) -> Result<(), IntegrationError> {
    let sse_config = Config {
        url: config.hue().url().to_owned(),
        retry_ms: config.hue().retry_ms(),
        retry_max_delay: config.hue().retry_max_delay_ms(),
        stale_connection_timeout_ms: config.hue().stale_connection_timeout_ms(),
        send_timeout_ms: config.hue().send_timeout_ms(),
    };
    let buffer_size = config.core().store_buffer_size();
    let client = client.clone();
    task::spawn(supervise_hue_pipeline(
        tx,
        client,
        pending_writes,
        grouped_light_index,
        store_rx,
        sse_config,
        buffer_size,
    ));

    Ok(())
}

/// Owns the Hue SSE listener and event processor pair for the app's lifetime.
/// If either task ends - gracefully, with an error, or by panicking - the other
/// task is aborted and a fresh chanel and task pair is rebuilt.
async fn supervise_hue_pipeline(
    tx: Sender<Event>,
    client: Client,
    pending_writes: Arc<PendingWrites>,
    grouped_light_index: Arc<GroupedLightIndex>,
    store_rx: WatchReceiver<StoreSnapshot>,
    sse_config: Config,
    buffer_size: usize,
) {
    loop {
        let (sse_tx, sse_rx) = mpsc::channel::<HueEvent>(buffer_size);

        let mut processor = task::spawn(run_processor(
            sse_rx,
            tx.clone(),
            pending_writes.clone(),
            grouped_light_index.clone(),
            store_rx.clone(),
        ));
        let mut listener = {
            let cloned_client = client.clone();
            let cloned_sse_config = sse_config.clone();
            task::spawn(async move {
                if let Err(e) =
                    sse::listen::<Vec<ServerSentEventPayload>>(sse_tx, &cloned_client, &cloned_sse_config).await
                {
                    warn!("⚠️ SSE listener exited with error: {}", e);
                }
            })
        };

        tokio::select! {
            result = &mut processor => {
                warn!("⚠️ Hue event processor {}. Rebuilding Hue pipeline...", describe(result));
                listener.abort();
            }
            result = &mut listener => {
                warn!("⚠️ Hue SSE listener {}. Rebuilding Hue pipeline...", describe(result));
                processor.abort();
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await; // Avoid a tight crash loop
    }
}

async fn run_processor(
    mut sse_rx: Receiver<HueEvent>,
    tx: Sender<Event>,
    pending_writes: Arc<PendingWrites>,
    grouped_light_index: Arc<GroupedLightIndex>,
    store_rx: WatchReceiver<StoreSnapshot>,
) {
    while let Some(hue_event) = sse_rx.recv().await {
        if let Some(comment) = &hue_event.comment {
            info!("🔹 {}", comment);
        }
        if let Some(data) = hue_event.data {
            for payload in data {
                let data_type = &payload.r#type;
                for property in payload.data {
                    handle_changed_property(
                        tx.clone(),
                        property,
                        data_type,
                        pending_writes.clone(),
                        grouped_light_index.clone(),
                        &store_rx,
                    )
                    .await;
                }
            }
        }
    }
}

async fn handle_changed_property(
    tx: Sender<Event>,
    property: ChangedProperty,
    data_type: &DataType,
    pending_writes: Arc<PendingWrites>,
    grouped_light_index: Arc<GroupedLightIndex>,
    store_rx: &WatchReceiver<StoreSnapshot>,
) {
    match property {
        ChangedProperty::Light(property) => {
            let initiated_by = light_initiated_by(&property, &pending_writes, store_rx);
            debug!(light_id = property.id, ?initiated_by, "Light changed");
            for event in map_light_changed_property(property, initiated_by) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed light event: {}", e);
                });
            }
        }
        ChangedProperty::Motion(property) => {
            for event in map_motion_sensors_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed motion sensor event: {}", e);
                });
            }
        }
        ChangedProperty::LightLevel(property) => {
            for event in map_light_level_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed light level event: {}", e);
                });
            }
        }
        ChangedProperty::Button(property) => {
            for event in map_remote_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed button event: {}", e);
                });
            }
        }
        ChangedProperty::ZigbeeConnectivity(property) => {
            for event in map_connectivity_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed connectivity event: {}", e);
                });
            }
        }
        ChangedProperty::DevicePower(property) => {
            for event in map_device_power_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed device power event: {}", e);
                });
            }
        }
        ChangedProperty::Temperature(property) => {
            for event in map_temperature_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send changed temperature event: {}", e);
                });
            }
        }
        ChangedProperty::Contact(property) => {
            for event in map_contact_sensors_changed(property) {
                tx.send(event).await.unwrap_or_else(|e| {
                    warn!("⚠️ Unable to send change contact sensor event: {}", e);
                });
            }
        }
        ChangedProperty::Room(property) => match data_type {
            DataType::Add | DataType::Update => grouped_light_index.apply(property.id.clone(), property.into()),
            DataType::Delete => grouped_light_index.remove(&property.id),
            DataType::Error => {}
        },
        ChangedProperty::Unknown(UnknownProperty { property_type, value }) => {
            debug!("⚠️ Unknown changed property type '{}'", property_type);
            trace!("   Payload: {}", value);
        }
    }
}

fn light_initiated_by(
    change: &LightChanged,
    pending_writes: &PendingWrites,
    store_rx: &WatchReceiver<StoreSnapshot>,
) -> InitiatedBy {
    if pending_writes.confirm(change, Instant::now()) {
        return InitiatedBy::Hearth;
    }

    let snapshot = store_rx.borrow().clone(); // Cheap: clones the Arc<DeviceMap>
    let device = snapshot.devices.get(&change.owner.rid);
    if device.is_some_and(|device| is_unnoticeable_correction(change, device)) {
        return InitiatedBy::Device;
    }

    InitiatedBy::External
}

fn describe(result: Result<(), JoinError>) -> &'static str {
    match result {
        Ok(()) => "ended gracefully",
        Err(e) if e.is_panic() => "panicked",
        Err(_) => "was cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Device;
    use crate::domain::property::CartesianCoordinate;
    use crate::hue::domain::LightRequest;
    use crate::test_support::DeviceBuilder;
    use serde_json::json;
    use std::collections::HashMap;
    use tokio::sync::watch;

    const LIGHT_ID: &str = "light_id";
    const DEVICE_ID: &str = "device_id";

    fn store_rx(device: Device) -> WatchReceiver<StoreSnapshot> {
        let snapshot = StoreSnapshot { devices: Arc::new(HashMap::from([(device.id.clone(), Arc::new(device))])) };
        watch::channel(snapshot).1
    }

    fn device() -> Device {
        DeviceBuilder::new(DEVICE_ID)
            .with_color_property("color", CartesianCoordinate::new(0.3691, 0.3719), None)
            .build()
    }

    fn color_changed(x: f64, y: f64) -> LightChanged {
        serde_json::from_value(json!({ "id": LIGHT_ID, "owner": { "rid": DEVICE_ID, "rtype": "device" }, "color": { "xy": { "x": x, "y": y } } })).unwrap()
    }

    #[test]
    fn light_initiated_by_recognizes_an_echo_of_a_hearth_write() {
        let pending_writes = PendingWrites::default();
        let request = LightRequest::new(None, None, None, Some(CartesianCoordinate::new(0.5, 0.4)), None);
        pending_writes.record(LIGHT_ID, &request, Instant::now());

        assert_eq!(
            light_initiated_by(&color_changed(0.5, 0.4), &pending_writes, &store_rx(device())),
            InitiatedBy::Hearth
        );
    }

    #[test]
    fn light_initiated_by_recognizes_an_unnoticeable_correction_by_the_device() {
        assert_eq!(
            light_initiated_by(&color_changed(0.3669, 0.3706), &PendingWrites::default(), &store_rx(device())),
            InitiatedBy::Device
        );
    }

    #[test]
    fn light_initiated_by_treats_a_noticeable_change_as_external() {
        assert_eq!(
            light_initiated_by(&color_changed(0.5, 0.4), &PendingWrites::default(), &store_rx(device())),
            InitiatedBy::External
        );
    }

    #[test]
    fn light_initiated_by_treats_a_change_to_an_unknown_device_as_external() {
        let empty_store_rx = watch::channel(StoreSnapshot::default()).1;

        assert_eq!(
            light_initiated_by(&color_changed(0.3669, 0.3706), &PendingWrites::default(), &empty_store_rx),
            InitiatedBy::External
        );
    }
}
