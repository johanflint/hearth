use crate::app_config::AppConfig;
use crate::hue::HueIntegration;
use crate::integration::{Integration, IntegrationError};
use crate::mdns_browser::DiscoveredServices;
use crate::mqtt::MqttService;
use crate::shelly::ShellyIntegration;
use crate::store::StoreSnapshot;
use std::sync::Arc;
use tokio::sync::watch::Receiver as WatchReceiver;
use tracing::{info, warn};

pub fn integrations(
    config: Arc<AppConfig>,
    store_rx: WatchReceiver<StoreSnapshot>,
    mdns_rx: Option<WatchReceiver<DiscoveredServices>>,
    mqtt_service: Option<MqttService>,
) -> Result<Vec<Arc<dyn Integration>>, IntegrationError> {
    let mut integrations: Vec<Arc<dyn Integration>> =
        vec![Arc::new(HueIntegration::new(config.clone(), store_rx.clone())?)];

    match mdns_rx {
        Some(mdns_rx) => integrations.push(Arc::new(ShellyIntegration::new(config, mdns_rx, mqtt_service, store_rx)?)),
        None => warn!("⚠️ Shelly integration skipped: mDNS is unavailable"),
    }
    Ok(integrations)
}

pub fn enabled_only(integrations: Vec<Arc<dyn Integration>>) -> Vec<Arc<dyn Integration>> {
    let (enabled, disabled): (Vec<_>, Vec<_>) = integrations.into_iter().partition(|i| i.enabled());
    for integration in disabled {
        info!("⏭️ Integration '{}' is disabled in config", integration.name());
    }
    enabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use tokio::sync::watch;

    fn hue_integration(enabled: bool) -> Arc<dyn Integration> {
        let config = Arc::new(AppConfigBuilder::new().hue_enabled(enabled).build());
        let (_store_tx, store_rx) = watch::channel(StoreSnapshot::default());
        Arc::new(HueIntegration::new(config, store_rx).unwrap())
    }

    #[test]
    fn enabled_only_keeps_the_enabled_integrations() {
        let integrations = enabled_only(vec![hue_integration(true)]);

        assert_eq!(integrations.len(), 1);
    }

    #[test]
    fn enabled_only_drops_the_disabled_integrations() {
        let integrations = enabled_only(vec![hue_integration(false), hue_integration(true)]);

        assert_eq!(integrations.len(), 1);
        assert!(integrations[0].enabled());
    }
}
