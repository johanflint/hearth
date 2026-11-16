use crate::api::ApiState;
use crate::api::devices::device_events::device_events;
use crate::api::devices::list_devices::list_devices;
use crate::api::devices::retrieve_device::retrieve_device;
use axum::Router;
use axum::routing::get;

mod color_view;
mod device_events;
mod device_type_view;
mod device_view;
mod list_devices;
mod number_view;
mod property_type_view;
mod property_view;
mod retrieve_device;
#[cfg(test)]
mod test_support;
mod unit_view;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/devices", get(list_devices))
        .route("/api/devices/events", get(device_events))
        .route("/api/devices/{id}", get(retrieve_device))
}
