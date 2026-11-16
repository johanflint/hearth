use crate::api::ApiState;
use crate::api::devices::list_devices::list_devices;
use axum::Router;
use axum::routing::get;

mod color_view;
mod device_type_view;
mod device_view;
mod list_devices;
mod number_view;
mod property_type_view;
mod property_view;
mod unit_view;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/devices", get(list_devices))
}
