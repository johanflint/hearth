use crate::domain::{Controller, Device, Event, Group, Zone};
use async_trait::async_trait;
use std::error::Error;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::Sender;

// Send + Sync so `?` can propagate it from async fns that are required to be Send
pub type IntegrationError = Box<dyn Error + Send + Sync>;

#[async_trait]
pub trait Integration: Debug + Send + Sync {
    /// Name of the system integrating with
    fn name(&self) -> &'static str;
    fn enabled(&self) -> bool;
    async fn discover(&self) -> Result<Discovery, IntegrationError>;
    /// Starts observing in the background and returns once started; the integration supervises itself afterwards
    async fn observe(&self, tx: Sender<Event>) -> Result<(), IntegrationError>;
    fn controller(&self) -> Arc<dyn Controller>;
}

/// The devices, groups and zones an integration found when discovering.
#[derive(Debug, PartialEq)]
pub struct Discovery {
    pub devices: Vec<Device>,
    pub groups: Vec<Group>,
    pub zones: Vec<Zone>,
}
