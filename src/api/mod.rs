mod error;
mod flows;
mod health;
mod metrics;
mod parse_request;
mod presets;
mod state;
#[cfg(test)]
mod test_support;

use axum::Router;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(metrics::router())
        .merge(flows::router())
        .merge(presets::router())
        .with_state(state)
}
