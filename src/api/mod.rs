mod error;
mod flows;
mod groups;
mod health;
mod metrics;
mod owner_view;
mod parse_request;
mod presets;
mod state;
#[cfg(test)]
mod test_support;
mod zones;

use axum::Router;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(metrics::router())
        .merge(flows::router())
        .merge(groups::router())
        .merge(zones::router())
        .merge(presets::router())
        .with_state(state)
}
