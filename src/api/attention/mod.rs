use crate::api::ApiState;
use crate::api::attention::attention_events::attention_events;
use crate::api::attention::list_items::list_items;
use axum::Router;
use axum::routing::get;

mod attention_events;
mod list_items;

pub fn router() -> Router<ApiState> {
    Router::new().route("/api/attention", get(list_items)).route("/api/attention/events", get(attention_events))
}
