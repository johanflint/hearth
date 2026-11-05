use crate::domain::{Command, Controller, Event};
use crate::integration::{Discovery, Integration, IntegrationError, IntegrationStatus};
use crate::mdns_browser::DiscoveredServices;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;

#[derive(Debug)]
pub struct ShellyIntegration {
    mdns_rx: WatchReceiver<DiscoveredServices>,
}

impl ShellyIntegration {
    pub fn new(mdns_rx: WatchReceiver<DiscoveredServices>) -> Result<Self, IntegrationError> {
        Ok(ShellyIntegration { mdns_rx })
    }
}

#[async_trait]
impl Integration for ShellyIntegration {
    fn name(&self) -> &'static str {
        "Shelly"
    }

    fn enabled(&self) -> bool {
        true
    }

    async fn discover(&self) -> Result<Discovery, IntegrationError> {
        Ok(Discovery { devices: vec![], groups: vec![], zones: vec![] })
    }

    async fn observe(&self, _tx: Sender<Event>) -> Result<(), IntegrationError> {
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
