use crate::api::ApiState;
use crate::api::devices::device_view::DeviceView;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub(super) async fn list_devices(State(state): State<ApiState>) -> Result<Response, Response> {
    let devices: Vec<_> = state.device_service.list().into_iter().map(DeviceView::from).collect();

    Ok((StatusCode::OK, Json(ListDevicesResponse { devices })).into_response())
}

#[derive(Debug, Serialize)]
struct ListDevicesResponse {
    devices: Vec<DeviceView>,
}
