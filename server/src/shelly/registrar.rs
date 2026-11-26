use crate::domain::{Device, Event};
use crate::shelly::map_shelly::map_shelly;
use crate::shelly::prober::ProbedDeviceMap;
use crate::shelly::take_new::take_new;
use std::collections::HashSet;
use tokio::sync::mpsc;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tracing::{debug, warn};

pub fn register_probed_devices(mut probed_rx: WatchReceiver<ProbedDeviceMap>, tx: mpsc::Sender<Event>) {
    task::spawn(async move {
        let mut seen = HashSet::new();
        loop {
            let probed_devices = take_new(&probed_rx.borrow_and_update(), &mut seen);
            let devices: Vec<Device> = probed_devices
                .iter()
                .filter_map(|probed| {
                    let device = map_shelly(probed);
                    if device.is_none() {
                        debug!("Ignoring probed device '{}' as it is not supported", probed.info.id);
                    }
                    device
                })
                .collect();

            if !devices.is_empty() && tx.send(Event::DiscoveredDevices(devices)).await.is_err() {
                warn!("⚠️ Store unavailable, stopped registering Shelly devices");
                break;
            }

            if probed_rx.changed().await.is_err() {
                break; // Prober stopped
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelly::prober::ProbedDevice;
    use crate::shelly::test_support::{ble_component, probed_device, switch_component};
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::watch;

    fn probed_map(devices: Vec<ProbedDevice>) -> ProbedDeviceMap {
        Arc::new(devices.into_iter().map(|device| (device.info.id.clone(), Arc::new(device))).collect())
    }

    fn probed_switch(id: &str) -> ProbedDevice {
        probed_device(id, &[switch_component(0, true)])
    }

    async fn next_device_ids(rx: &mut mpsc::Receiver<Event>) -> Vec<String> {
        let Some(Event::DiscoveredDevices(devices)) = rx.recv().await else {
            panic!("expected a DiscoveredDevices event");
        };
        let mut ids: Vec<String> = devices.into_iter().map(|device| device.id).collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn registers_the_probed_devices_at_once() {
        let (_probed_tx, probed_rx) = watch::channel(probed_map(vec![probed_switch("a"), probed_switch("b")]));
        let (tx, mut rx) = mpsc::channel(1);

        register_probed_devices(probed_rx, tx);

        assert_eq!(next_device_ids(&mut rx).await, ["a", "b"]);
    }

    #[tokio::test]
    async fn registers_devices_probed_later_only_once() {
        let (probed_tx, probed_rx) = watch::channel(probed_map(vec![probed_switch("a")]));
        let (tx, mut rx) = mpsc::channel(1);
        register_probed_devices(probed_rx, tx);
        assert_eq!(next_device_ids(&mut rx).await, ["a"]);

        probed_tx.send_modify(|devices| {
            Arc::make_mut(devices).insert("b".to_string(), Arc::new(probed_switch("b")));
        });

        assert_eq!(next_device_ids(&mut rx).await, ["b"]);
    }

    #[tokio::test]
    async fn ignores_unsupported_devices() {
        let (probed_tx, probed_rx) = watch::channel(probed_map(vec![probed_device("ble", &[ble_component()])]));
        let (tx, mut rx) = mpsc::channel(1);
        register_probed_devices(probed_rx, tx);

        probed_tx.send_modify(|devices| {
            Arc::make_mut(devices).insert("a".to_string(), Arc::new(probed_switch("a")));
        });

        assert_eq!(next_device_ids(&mut rx).await, ["a"]);
    }

    #[tokio::test]
    async fn stops_when_the_prober_stops() {
        let (probed_tx, probed_rx) = watch::channel(ProbedDeviceMap::default());
        let (tx, mut rx) = mpsc::channel(1);
        register_probed_devices(probed_rx, tx);

        drop(probed_tx);

        assert!(rx.recv().await.is_none(), "the registrar should drop its sender once it stops");
    }

    #[tokio::test(start_paused = true)]
    async fn stops_when_the_store_is_unavailable() {
        let (probed_tx, probed_rx) = watch::channel(probed_map(vec![probed_switch("a")]));
        let (tx, rx) = mpsc::channel(1);
        drop(rx);

        register_probed_devices(probed_rx, tx);

        tokio::time::timeout(Duration::from_secs(1), probed_tx.closed())
            .await
            .expect("the registrar should drop its receiver once it stops");
    }

    #[test]
    fn take_new_returns_each_entry_only_once() {
        let mut map = HashMap::from([("a", Arc::new(1))]);
        let mut seen = HashSet::new();

        assert_eq!(take_new(&map, &mut seen), [Arc::new(1)]);
        assert!(take_new(&map, &mut seen).is_empty());

        map.insert("b", Arc::new(2));
        assert_eq!(take_new(&map, &mut seen), [Arc::new(2)]);
    }
}
