use crate::sse_listen::listen;
use reqwest::Client;

mod sse_listen;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let client = Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("Failed to build client");

    listen(&client, "https://192.168.1.150/eventstream/clip/v2")
        .await
        .expect("Could not listen to SSE stream");
}
