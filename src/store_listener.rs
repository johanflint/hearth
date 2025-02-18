use crate::store::DeviceMap;
use tokio::sync::watch::Receiver;
use tracing::{info, instrument};

#[instrument(skip(rx))]
pub async fn store_listener(mut rx: Receiver<DeviceMap>) {
    while rx.changed().await.is_ok() {
        let store: DeviceMap = rx.borrow().clone();
        // Note that the read_guard locks until it is dropped, can be avoided to clone the read_guard which is expensive
        let read_guard = store.read().await;
        info!("Updated store: {:?}", read_guard);
    }
}
