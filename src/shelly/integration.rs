use crate::app_config::AppConfig;
use crate::domain::{Command, Controller, Event};
use crate::integration::{Discovery, Integration, IntegrationError, IntegrationStatus};
use crate::mdns_browser::DiscoveredServices;
use crate::mqtt::MqttService;
use crate::shelly::client::new_client;
use crate::shelly::mqtt_listener::mqtt_listen;
use crate::shelly::prober;
use crate::store::StoreSnapshot;
use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;

#[derive(Debug)]
pub struct ShellyIntegration {
    client: Client,
    config: Arc<AppConfig>,
    mdns_rx: WatchReceiver<DiscoveredServices>,
    mqtt_service: MqttService,
    store_rx: WatchReceiver<StoreSnapshot>,
}

impl ShellyIntegration {
    pub fn new(
        config: Arc<AppConfig>,
        mdns_rx: WatchReceiver<DiscoveredServices>,
        mqtt_service: MqttService,
        store_rx: WatchReceiver<StoreSnapshot>,
    ) -> Result<Self, IntegrationError> {
        let client = new_client(&config)?;
        Ok(ShellyIntegration { client, config, mdns_rx, mqtt_service, store_rx })
    }
}

#[async_trait]
impl Integration for ShellyIntegration {
    fn name(&self) -> &'static str {
        "Shelly"
    }

    fn enabled(&self) -> bool {
        self.config.shelly().enabled()
    }

    async fn discover(&self) -> Result<Discovery, IntegrationError> {
        Ok(Discovery { devices: vec![], groups: vec![], zones: vec![] })
    }

    async fn observe(&self, tx: Sender<Event>) -> Result<(), IntegrationError> {
        prober::probe_announced_devices(&self.client, self.mdns_rx.clone(), tx.clone())?;
        mqtt_listen(&self.mqtt_service, self.store_rx.clone(), tx)?;
        Ok(())
    }

    fn controller(&self) -> Arc<dyn Controller> {
        Arc::new(StubController)
    }

    fn status(&self) -> IntegrationStatus {
        IntegrationStatus::Up
    }
}

#[derive(Debug)]
struct StubController;

#[async_trait]
impl Controller for StubController {
    fn id(&self) -> &'static str {
        "stub"
    }

    async fn execute(&self, _command: Command) {}
}
