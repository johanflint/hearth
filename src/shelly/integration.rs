use crate::app_config::AppConfig;
use crate::domain::{Command, Controller, Event};
use crate::integration::{Discovery, Integration, IntegrationError, IntegrationStatus};
use crate::mdns_browser::MdnsBrowser;
use crate::mqtt::MqttService;
use crate::mqtt_security::MqttSecurity;
use crate::shelly::client::new_client;
use crate::shelly::controller::ShellyController;
use crate::shelly::mqtt_listener::mqtt_listen;
use crate::shelly::prober::ProbedDeviceMap;
use crate::shelly::{prober, provisioner, registrar};
use crate::store::StoreSnapshot;
use async_trait::async_trait;
use reqwest::Client;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;
use tracing::warn;

const SERVICE_TYPE: &str = "_shelly._tcp.local.";

#[derive(Debug)]
pub struct ShellyIntegration {
    client: Client,
    config: Arc<AppConfig>,
    probed_rx: WatchReceiver<ProbedDeviceMap>,
    mqtt_service: MqttService,
    mqtt_security: Arc<MqttSecurity>,
    store_rx: WatchReceiver<StoreSnapshot>,
}

impl ShellyIntegration {
    pub fn new(
        config: Arc<AppConfig>,
        mdns_browser: &MdnsBrowser,
        mqtt_service: MqttService,
        store_rx: WatchReceiver<StoreSnapshot>,
    ) -> Result<Self, IntegrationError> {
        let client = new_client(&config)?;
        let mdns_rx = mdns_browser.browse(SERVICE_TYPE)?;
        let probed_rx = prober::probe_announced_devices(&client, mdns_rx);
        let mqtt_security = Arc::new(MqttSecurity::new(mqtt_service.clone())?);
        Ok(ShellyIntegration { client, config, probed_rx, mqtt_service, mqtt_security, store_rx })
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
        registrar::register_probed_devices(self.probed_rx.clone(), tx.clone());
        if is_loopback_address(self.config.mqtt().advertised_host()) {
            warn!(
                "⚠️ The advertised MQTT host is '{}', devices can't reach the broker there, use the LAN address",
                self.config.mqtt().advertised_host()
            );
        } else {
            provisioner::provision_announced_devices(
                &self.client,
                Arc::clone(&self.config),
                Arc::clone(&self.mqtt_security),
                self.probed_rx.clone(),
            );
        }
        mqtt_listen(&self.mqtt_service, self.store_rx.clone(), tx)?;
        Ok(())
    }

    fn controller(&self) -> Arc<dyn Controller> {
        Arc::new(ShellyController::new(self.client.clone(), self.probed_rx.clone()))
    }

    fn status(&self) -> IntegrationStatus {
        IntegrationStatus::Up
    }
}

fn is_loopback_address(host: &str) -> bool {
    host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::ipv4_loopback("127.0.0.1", true)]
    #[case::other_ipv4_loopback("127.1.2.3", true)]
    #[case::ipv6_loopback("::1", true)]
    #[case::localhost("localhost", true)]
    #[case::lan_address("192.168.1.36", false)]
    #[case::unspecified_address("0.0.0.0", false)]
    #[case::hostname("broker.local", false)]
    fn detects_loopback_addresses(#[case] host: &str, #[case] expected: bool) {
        assert_eq!(is_loopback_address(host), expected);
    }
}
