mod list_groups;

use crate::api::ApiState;
use crate::api::groups::list_groups::list_groups;
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/groups", get(list_groups))
}
