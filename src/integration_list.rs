use crate::app_config::AppConfig;
use crate::hue::HueIntegration;
use crate::integration::{Integration, IntegrationError};
use crate::store::StoreSnapshot;
use std::sync::Arc;
use tokio::sync::watch::Receiver as WatchReceiver;

pub fn integrations(
    config: Arc<AppConfig>,
    store_rx: WatchReceiver<StoreSnapshot>,
) -> Result<Vec<Arc<dyn Integration>>, IntegrationError> {
    Ok(vec![Arc::new(HueIntegration::new(config, store_rx)?)])
}
