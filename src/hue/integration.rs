use crate::app_config::AppConfig;
use crate::domain::{Controller, Device, Event};
use crate::hue;
use crate::hue::client::new_client;
use crate::hue::controller::HueController;
use crate::hue::grouped_light_index::GroupedLightIndex;
use crate::hue::pending_writes::PendingWrites;
use crate::integration::{Integration, IntegrationError};
use crate::store::StoreSnapshot;
use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;

#[derive(Debug)]
pub struct HueIntegration {
    client: Client,
    config: Arc<AppConfig>,
    store_rx: WatchReceiver<StoreSnapshot>,
    pending_writes: Arc<PendingWrites>,
    grouped_light_index: Arc<GroupedLightIndex>,
}

impl HueIntegration {
    pub fn new(config: Arc<AppConfig>, store_rx: WatchReceiver<StoreSnapshot>) -> Result<Self, IntegrationError> {
        let client = new_client(&config)?;
        let pending_writes = Arc::new(PendingWrites::default());
        let grouped_light_index = Arc::new(GroupedLightIndex::default());
        Ok(HueIntegration { client, config, store_rx, pending_writes, grouped_light_index })
    }
}

#[async_trait]
impl Integration for HueIntegration {
    fn name(&self) -> &'static str {
        "Hue"
    }

    fn enabled(&self) -> bool {
        self.config.hue().enabled()
    }

    async fn discover(&self) -> Result<Vec<Device>, IntegrationError> {
        let discovery = hue::discover(&self.client, &self.config).await?;
        self.grouped_light_index.replace_all(discovery.grouped_lights);
        Ok(discovery.devices)
    }

    async fn observe(&self, tx: Sender<Event>) -> Result<(), IntegrationError> {
        hue::observe(tx, self.pending_writes.clone(), self.store_rx.clone(), &self.client, &self.config).await
    }

    fn controller(&self) -> Arc<dyn Controller> {
        Arc::new(HueController::new(self.client.clone(), self.pending_writes.clone(), self.config.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::AppConfigBuilder;
    use crate::hue::controller::CONTROLLER_ID;
    use crate::hue::discoverer::DiscoverError;
    use rstest::rstest;
    use tokio::sync::{mpsc, watch};

    fn hue_integration(hue_url: String) -> HueIntegration {
        let config = Arc::new(AppConfigBuilder::new().hue_url(hue_url).build());
        let (_store_tx, store_rx) = watch::channel(StoreSnapshot::default());
        HueIntegration::new(config, store_rx).unwrap()
    }

    #[rstest]
    #[case::enabled(true)]
    #[case::disabled(false)]
    fn enabled_follows_the_hue_config(#[case] enabled: bool) {
        let config = Arc::new(AppConfigBuilder::new().hue_enabled(enabled).build());
        let (_store_tx, store_rx) = watch::channel(StoreSnapshot::default());
        let integration = HueIntegration::new(config, store_rx).unwrap();

        assert_eq!(integration.enabled(), enabled);
    }

    #[test]
    fn controller_is_the_hue_controller() {
        let integration = hue_integration("https://hue.url".to_string());

        assert_eq!(integration.controller().id(), CONTROLLER_ID);
    }

    #[tokio::test]
    async fn discover_returns_the_discover_error_when_hue_responds_unexpectedly() {
        let mut server = mockito::Server::new_async().await;
        let mock = server.mock("GET", "/clip/v2/resource/device").with_status(500).create_async().await;
        let integration = hue_integration(server.url());

        let error = integration.discover().await.unwrap_err();

        assert!(
            matches!(error.downcast_ref::<DiscoverError>(), Some(DiscoverError::UnexpectedResponse(..))),
            "got {error}"
        );
        mock.assert();
    }

    #[tokio::test]
    async fn observe_returns_once_the_observer_is_started() {
        let integration = hue_integration("https://hue.url".to_string());
        let (tx, _rx) = mpsc::channel(1);

        assert!(integration.observe(tx).await.is_ok());
    }
}
