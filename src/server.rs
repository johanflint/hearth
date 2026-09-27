use crate::api;
use crate::api::ApiState;
use axum::Router;
use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task;
use tracing::{error, info, warn};

pub async fn start(port: usize, state: ApiState) -> Result<(), ServerError> {
    let app = api::router(state);

    // Fatal: propagate so the process can be aborted. A port config being unusable is an environment problem
    let listener = bind_with_retries(port, 5, Duration::from_millis(500)).await?;
    task::spawn(supervise(app, port, listener));
    Ok(())
}

async fn supervise(app: Router, port: usize, mut listener: TcpListener) -> Result<(), ServerError> {
    loop {
        let app_clone = app.clone();

        let handle = task::spawn(async move {
            axum::serve(listener, app_clone).await
        });

        match handle.await {
            Ok(Ok(())) => warn!("Server exited cleanly. Restarting..."),
            Ok(Err(e)) => error!("Server error: {e}. Restarting..."),
            Err(join_err) => error!("Server task panicked: {join_err:?}. Restarting..."),
        }

        tokio::time::sleep(Duration::from_secs(1)).await; // Avoid a tight crash loop

        // Previous listener was consumed by axum::serve. Panics are not fatal as the rest of the app keeps running.
        listener = match bind_with_retries(port, 5, Duration::from_millis(500)).await {
            Ok(listener) => listener,
            Err(e) => {
                error!("❌ Giving up on restarting the server: {e}");
                return Ok(());
            }
        }
    }
}

async fn bind_with_retries(port: usize, max_attempts: u32, delay: Duration) -> Result<TcpListener, ServerError> {
    let address = format!("0.0.0.0:{port}");
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match TcpListener::bind(&address).await {
            Ok(listener) => {
                info!("Server listening on {address}");
                return Ok(listener);
            }
            Err(e) => {
                warn!("⚠️ Unable to bind to '{address}' (attempt {attempt}/{max_attempts}): {e}. Retrying...");
                last_error = Some(e);
                if attempt < max_attempts {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
    Err(ServerError::TcpError { address, attempts: max_attempts, error: last_error.unwrap() })
}

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("unable to bind to address '{address}' after {attempts} attempt(s): {error}")]
    TcpError { address: String, attempts: u32, error: std::io::Error },
}
