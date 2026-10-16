mod list_presets;

use crate::api::ApiState;
use crate::api::presets::list_presets::list_presets;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/presets", get(list_presets))
}
