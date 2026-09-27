mod metrics;
mod state;

use axum::Router;
pub use state::ApiState;

pub fn router(state: ApiState) -> Router {
    Router::new()
        .merge(metrics::router())
        .with_state(state)
}
