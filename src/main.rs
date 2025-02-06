use crate::sse_listen::listen;

mod sse_listen;

#[tokio::main]
async fn main() {
    println!("Hello, world!");
    listen("https://192.168.1.150/eventstream/clip/v2")
        .await
        .expect("Could not listen to SSE stream");
}
