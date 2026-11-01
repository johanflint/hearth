use crate::domain::{Controller, Device, Event, Group, Zone};
use async_trait::async_trait;
use std::error::Error;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
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
    /// Cheap, non-blocking snapshot of the integration's own state, without calling the external system
    fn status(&self) -> IntegrationStatus;
}

/// The devices, groups and zones an integration found when discovering.
#[derive(Debug, PartialEq)]
pub struct Discovery {
    pub devices: Vec<Device>,
    pub groups: Vec<Group>,
    pub zones: Vec<Zone>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IntegrationStatus {
    /// Working as expected
    Up,
    /// Was up and is reconnecting
    Degraded,
    /// Not connected: it never was, or its background tasks have stopped
    Down,
}

/// Written by an integration's background tasks, read by `Integration::status`
#[derive(Debug, Clone)]
pub struct SharedStatus(Arc<Mutex<IntegrationStatus>>);

impl SharedStatus {
    pub fn new(initial: IntegrationStatus) -> Self {
        Self(Arc::new(Mutex::new(initial)))
    }

    pub fn get(&self) -> IntegrationStatus {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set(&self, status: IntegrationStatus) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = status;
    }

    /// `Up` becomes `Degraded`. `Down` stays `Down`, as a connection that never existed isn't reconnecting but connecting.
    pub fn degrade(&self) {
        let mut status = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *status == IntegrationStatus::Up {
            *status = IntegrationStatus::Degraded;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn set_replaces_the_status_for_all_clones() {
        let status = SharedStatus::new(IntegrationStatus::Down);
        let clone = status.clone();

        clone.set(IntegrationStatus::Up);

        assert_eq!(status.get(), IntegrationStatus::Up);
    }

    #[rstest]
    #[case::up(IntegrationStatus::Up, IntegrationStatus::Degraded)]
    #[case::degraded(IntegrationStatus::Degraded, IntegrationStatus::Degraded)]
    #[case::down(IntegrationStatus::Down, IntegrationStatus::Down)]
    fn degrade_only_moves_up_to_degraded(#[case] initial: IntegrationStatus, #[case] expected: IntegrationStatus) {
        let status = SharedStatus::new(initial);

        status.degrade();

        assert_eq!(status.get(), expected);
    }

    #[test]
    fn get_recovers_from_a_poisoned_lock() {
        let status = SharedStatus::new(IntegrationStatus::Up);
        let clone = status.clone();
        let _ = std::thread::spawn(move || {
            let _guard = clone.0.lock().unwrap();
            panic!("poison the lock");
        })
        .join();

        assert_eq!(status.get(), IntegrationStatus::Up);
    }
}
