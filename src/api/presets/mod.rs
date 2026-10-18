mod create_preset;
mod delete_preset;
mod list_presets;
mod preset_response;
mod retrieve_preset;
mod update_preset;

use crate::api::ApiState;
use crate::api::presets::create_preset::create_preset;
use crate::api::presets::delete_preset::delete_preset;
use crate::api::presets::list_presets::list_presets;
use crate::api::presets::retrieve_preset::retrieve_preset;
use crate::api::presets::update_preset::update_preset;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/presets", get(list_presets).post(create_preset))
        .route("/api/presets/{id}", get(retrieve_preset).put(update_preset).delete(delete_preset))
}
