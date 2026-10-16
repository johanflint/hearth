mod error;
mod flows;
mod metrics;
mod presets;
mod state;
#[cfg(test)]
mod test_support;

use axum::Router;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new() // Merge all routes
        .merge(metrics::router())
        .merge(flows::router())
        .merge(presets::router())
        .with_state(state)
}
