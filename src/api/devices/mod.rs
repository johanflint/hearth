use crate::api::ApiState;
use crate::api::devices::list_devices::list_devices;
use axum::Router;
use axum::routing::get;

mod device_type_view;
mod device_view;
mod list_devices;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/devices", get(list_devices))
}
