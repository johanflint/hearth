mod list_presets;
mod preset_response;
mod retrieve_preset;

use crate::api::ApiState;
use crate::api::presets::list_presets::list_presets;
use crate::api::presets::retrieve_preset::retrieve_preset;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/presets", get(list_presets)).route("/api/presets/{id}", get(retrieve_preset))
}
