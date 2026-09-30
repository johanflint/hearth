mod flows;
mod metrics;
mod state;
mod error;

use axum::Router;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .merge(metrics::router())
        .merge(flows::router())
        .with_state(state)
}
