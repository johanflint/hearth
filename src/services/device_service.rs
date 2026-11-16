use crate::domain::Device;
use crate::store::StoreSnapshot;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct DeviceService {
    snapshot_rx: Receiver<StoreSnapshot>,
}

impl DeviceService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>) -> Self {
        DeviceService { snapshot_rx }
    }

    pub fn list(&self) -> Vec<Arc<Device>> {
        let device_map = self.snapshot_rx.borrow().devices.clone();
        let mut devices: Vec<_> = device_map.values().cloned().collect();
        devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        devices
    }

    pub fn retrieve_device(&self, id: &str) -> Result<Arc<Device>, RetrieveDeviceError> {
        self.snapshot_rx.borrow().devices.get(id).cloned().ok_or(RetrieveDeviceError::NotFound)
    }
}

#[derive(Debug, Error)]
pub enum RetrieveDeviceError {
    #[error("device not found")]
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::DeviceBuilder;
    use std::collections::HashMap;
    use tokio::sync::watch;

    fn device(id: &str, name: &str) -> Arc<Device> {
        Arc::new(Device { name: name.to_string(), ..DeviceBuilder::new(id).build() })
    }

    fn snapshot(devices: Vec<Arc<Device>>) -> StoreSnapshot {
        let devices: HashMap<_, _> = devices.into_iter().map(|device| (device.id.clone(), device)).collect();
        StoreSnapshot { devices: Arc::new(devices), ..Default::default() }
    }

    fn ids(devices: &[Arc<Device>]) -> Vec<&str> {
        devices.iter().map(|device| device.id.as_str()).collect()
    }

    #[test]
    fn list_returns_nothing_for_an_empty_snapshot() {
        let (_tx, rx) = watch::channel(StoreSnapshot::default());

        assert!(DeviceService::new(rx).list().is_empty());
    }

    #[test]
    fn list_orders_the_devices_by_name_instead_of_id() {
        let (_tx, rx) =
            watch::channel(snapshot(vec![device("1", "Remote"), device("2", "Ceiling"), device("3", "Lamp")]));

        let devices = DeviceService::new(rx).list();

        assert_eq!(ids(&devices), vec!["2", "3", "1"]);
    }

    #[test]
    fn list_orders_devices_with_the_same_name_by_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![device("c", "Lamp"), device("a", "Lamp"), device("b", "Lamp")]));

        let devices = DeviceService::new(rx).list();

        assert_eq!(ids(&devices), vec!["a", "b", "c"]);
    }

    #[test]
    fn list_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![device("1", "Lamp")]));
        let service = DeviceService::new(rx);

        tx.send(snapshot(vec![device("1", "Lamp"), device("2", "Ceiling")])).unwrap();

        assert_eq!(ids(&service.list()), vec!["2", "1"]);
    }

    #[test]
    fn retrieve_device_returns_the_device_with_the_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![device("1", "Lamp"), device("2", "Ceiling")]));

        let found = DeviceService::new(rx).retrieve_device("2").unwrap();

        assert_eq!(found.name, "Ceiling");
    }

    #[test]
    fn retrieve_device_returns_not_found_for_an_unknown_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![device("1", "Lamp")]));

        let result = DeviceService::new(rx).retrieve_device("unknown");

        assert!(matches!(result, Err(RetrieveDeviceError::NotFound)));
    }

    #[test]
    fn retrieve_device_does_not_match_on_the_name() {
        let (_tx, rx) = watch::channel(snapshot(vec![device("1", "Lamp")]));

        let result = DeviceService::new(rx).retrieve_device("Lamp");

        assert!(matches!(result, Err(RetrieveDeviceError::NotFound)));
    }

    #[test]
    fn retrieve_device_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![device("1", "Lamp")]));
        let service = DeviceService::new(rx);

        tx.send(snapshot(vec![device("1", "Lamp"), device("2", "Ceiling")])).unwrap();

        assert!(service.retrieve_device("2").is_ok());
    }
}
