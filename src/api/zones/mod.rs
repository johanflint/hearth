use crate::api::ApiState;
use axum::Router;
use axum::routing::get;

mod list_zones;

use crate::api::zones::list_zones::list_zones;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/zones", get(list_zones))
}
