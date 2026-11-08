use crate::mdns_browser::{DiscoveredService, DiscoveredServices, ServiceKey};
use crate::shelly::domain::{Component, ComponentsResult, DeviceInfo};
use crate::shelly::fetch::{FetchError, fetch};
use reqwest::Client;
use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::watch;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::sync::watch::Sender;
use tokio::task;
use tokio::task::AbortHandle;
use tracing::{debug, warn};

#[derive(Debug)]
pub struct ProbedDevice {
    pub address: SocketAddr,
    pub info: DeviceInfo,
    pub components: Vec<Component>,
}

pub type ProbedDeviceMap = Arc<HashMap<String, Arc<ProbedDevice>>>;

pub fn probe_announced_devices(
    client: &Client,
    mdns_rx: WatchReceiver<DiscoveredServices>,
) -> WatchReceiver<ProbedDeviceMap> {
    let client = client.clone();
    let (tx, rx) = watch::channel(Arc::new(HashMap::new()));
    task::spawn(async move {
        listen(tx, &client, mdns_rx).await;
    });
    rx
}

async fn listen(tx: Sender<ProbedDeviceMap>, client: &Client, mut mdns_rx: WatchReceiver<DiscoveredServices>) {
    let mut seen: HashMap<ServiceKey, BTreeSet<SocketAddr>> = HashMap::new();
    let mut probes: HashMap<ServiceKey, AbortHandle> = HashMap::new();
    loop {
        let new = take_new_or_moved(&mdns_rx.borrow_and_update(), &mut seen);
        for (key, service) in new {
            let client = client.clone();
            let tx = tx.clone();
            let probe = task::spawn(async move {
                probe_with_retry(&client, &service, tx).await;
            });
            if let Some(previous) = probes.insert(key, probe.abort_handle()) {
                // Stop a previous prober that still has the old address
                previous.abort();
            }
        }

        if mdns_rx.changed().await.is_err() {
            break; // all browse tasks ended, e.g. the daemon was shut down
        }
    }
}

// New services, and known services that announce other addresses (e.g. after a new DHCP lease).
// Compares addresses because every re-resolve creates a new Arc with the same content.
fn take_new_or_moved(
    services: &HashMap<ServiceKey, Arc<DiscoveredService>>,
    seen: &mut HashMap<ServiceKey, BTreeSet<SocketAddr>>,
) -> Vec<(ServiceKey, Arc<DiscoveredService>)> {
    services
        .iter()
        .filter(|(key, service)| {
            seen.insert((*key).clone(), service.addresses.clone()).as_ref() != Some(&service.addresses)
        })
        .map(|(key, service)| (key.clone(), Arc::clone(service)))
        .collect()
}

async fn probe_with_retry(client: &Client, service: &DiscoveredService, tx: Sender<ProbedDeviceMap>) {
    let mut delay = Duration::from_secs(1);
    loop {
        match probe(client, service).await {
            Ok(probed_device) => {
                let id = probed_device.info.id.clone();
                let added = tx.send_if_modified(|devices| {
                    if devices.get(&id).is_some_and(|known| known.address == probed_device.address) {
                        return false;
                    }
                    Arc::make_mut(devices).insert(id.clone(), Arc::new(probed_device));
                    true
                });
                if !added {
                    debug!("Skipping '{}', device '{id}' is already known at that address", service.hostname);
                }
                return;
            }
            Err(err) if err.is_permanent() => {
                warn!("⚠️ Probing '{}' failed, giving up: {err}", service.hostname);
                return;
            }
            Err(err) => {
                warn!("⚠️ Probing '{}' failed, retrying in {delay:?}: {err}", service.hostname);
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(60));
    }
}

async fn probe(client: &Client, service: &DiscoveredService) -> Result<ProbedDevice, ProbeError> {
    let mut last_err = ProbeError::NoAddress;
    for address in &service.addresses {
        match probe_address(client, address).await {
            Ok(probed_device) => return Ok(probed_device),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

async fn probe_address(client: &Client, address: &SocketAddr) -> Result<ProbedDevice, ProbeError> {
    let device_info = fetch::<DeviceInfo>(client, address, "Shelly.GetDeviceInfo", &[]).await?;
    let components_response =
        fetch::<ComponentsResult>(client, address, "Shelly.GetComponents", &[("include", r#"["config","status"]"#)])
            .await?;

    Ok(ProbedDevice { address: *address, info: device_info, components: components_response.components })
}

#[derive(Error, Debug)]
pub enum ProbeError {
    #[error(transparent)]
    Fetch(#[from] FetchError),
    #[error("no address")]
    NoAddress,
}

impl ProbeError {
    // Whether the error is permanent or whether retrying helps
    fn is_permanent(&self) -> bool {
        match self {
            ProbeError::Fetch(err) => err.is_permanent(),
            ProbeError::NoAddress => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelly::test_support::{PLUS_1_COMPONENTS, PLUS_1_DEVICE_INFO, PLUS_1_ID, probed_device};
    use mockito::{Matcher, Mock, Server, ServerGuard};
    use reqwest::StatusCode;
    use rstest::rstest;
    use std::collections::BTreeSet;
    use tokio::sync::watch;

    fn service(hostname: &str, addresses: &[SocketAddr]) -> DiscoveredService {
        DiscoveredService {
            hostname: hostname.to_string(),
            addresses: BTreeSet::from_iter(addresses.iter().copied()),
            txt: HashMap::new(),
        }
    }

    async fn mock_device_info(server: &mut ServerGuard, status: usize, body: &str) -> Mock {
        server.mock("GET", "/rpc/Shelly.GetDeviceInfo").with_status(status).with_body(body).create_async().await
    }

    async fn mock_components(server: &mut ServerGuard) -> Mock {
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::UrlEncoded("include".to_string(), r#"["config","status"]"#.to_string()))
            .with_body(PLUS_1_COMPONENTS)
            .create_async()
            .await
    }

    fn key(hostname: &str) -> ServiceKey {
        ServiceKey { hostname: hostname.to_string() }
    }

    fn services_at(hostname: &str, address: &str) -> HashMap<ServiceKey, Arc<DiscoveredService>> {
        HashMap::from([(key(hostname), Arc::new(service(hostname, &[address.parse().unwrap()])))])
    }

    #[test]
    fn take_new_or_moved_returns_a_new_service() {
        let mut seen = HashMap::new();

        let new = take_new_or_moved(&services_at("a.local.", "192.168.1.10:80"), &mut seen);

        assert_eq!(new.len(), 1);
        assert_eq!(new[0].0, key("a.local."));
    }

    #[test]
    fn take_new_or_moved_skips_a_service_at_the_same_addresses() {
        let mut seen = HashMap::new();
        take_new_or_moved(&services_at("a.local.", "192.168.1.10:80"), &mut seen);

        let new = take_new_or_moved(&services_at("a.local.", "192.168.1.10:80"), &mut seen);

        assert!(new.is_empty());
    }

    #[test]
    fn take_new_or_moved_returns_a_service_at_other_addresses() {
        let mut seen = HashMap::new();
        take_new_or_moved(&services_at("a.local.", "192.168.1.10:80"), &mut seen);

        let moved = take_new_or_moved(&services_at("a.local.", "192.168.1.20:80"), &mut seen);

        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0].1.addresses, BTreeSet::from(["192.168.1.20:80".parse().unwrap()]));
    }

    #[rstest]
    #[case::permanent_fetch_error(
        ProbeError::Fetch(FetchError::UnexpectedResponse(StatusCode::UNAUTHORIZED, "url".to_string())),
        true
    )]
    #[case::transient_fetch_error(
        ProbeError::Fetch(FetchError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, "url".to_string())),
        false
    )]
    #[case::no_address(ProbeError::NoAddress, true)]
    fn is_permanent(#[case] error: ProbeError, #[case] expected: bool) {
        assert_eq!(error.is_permanent(), expected);
    }

    #[tokio::test]
    async fn probe_announced_devices_publishes_each_announced_device() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        let key = ServiceKey { hostname: "a.local.".to_string() };
        let services = HashMap::from([(key, Arc::new(service("a.local.", &[server.socket_address()])))]);
        let (_mdns_tx, mdns_rx) = watch::channel(Arc::new(services));

        let mut probed_rx = probe_announced_devices(&Client::new(), mdns_rx);

        probed_rx.changed().await.unwrap();
        assert!(probed_rx.borrow().contains_key(PLUS_1_ID));
    }

    #[tokio::test]
    async fn probe_address_returns_the_probed_device() {
        let mut server = Server::new_async().await;
        let device_info = mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        let components = mock_components(&mut server).await;
        let address = server.socket_address();

        let probed = probe_address(&Client::new(), &address).await.unwrap();

        assert_eq!(probed.address, address);
        assert_eq!(probed.info.id, PLUS_1_ID);
        assert_eq!(probed.components.len(), 7);
        device_info.assert();
        components.assert();
    }

    #[tokio::test]
    async fn probe_address_fails_on_an_unexpected_status() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 404, "").await;

        let error = probe_address(&Client::new(), &server.socket_address()).await.unwrap_err();

        assert!(
            matches!(error, ProbeError::Fetch(FetchError::UnexpectedResponse(StatusCode::NOT_FOUND, _))),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn probe_address_fails_when_the_components_are_protected() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        server.mock("GET", "/rpc/Shelly.GetComponents").match_query(Matcher::Any).with_status(401).create_async().await;

        let error = probe_address(&Client::new(), &server.socket_address()).await.unwrap_err();

        assert!(
            matches!(error, ProbeError::Fetch(FetchError::UnexpectedResponse(StatusCode::UNAUTHORIZED, _))),
            "got {error:?}"
        );
    }

    // Whether a device is supported is up to the consumers, the prober only probes
    #[tokio::test]
    async fn probe_address_returns_a_device_without_switches() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::Any)
            .with_body(r#"{"components":[{"key":"ble","status":{},"config":{}}]}"#)
            .create_async()
            .await;

        let probed = probe_address(&Client::new(), &server.socket_address()).await.unwrap();

        assert!(matches!(probed.components[..], [Component::Unsupported]), "got {:?}", probed.components);
    }

    #[tokio::test]
    async fn probe_without_addresses_fails_with_no_address() {
        let error = probe(&Client::new(), &service("a.local.", &[])).await.unwrap_err();

        assert!(matches!(error, ProbeError::NoAddress), "got {error:?}");
    }

    #[tokio::test]
    async fn probe_falls_back_to_the_next_address() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        // Addresses are tried in order, port 1 sorts first and refuses the connection
        let unreachable: SocketAddr = "127.0.0.1:1".parse().unwrap();

        let probed =
            probe(&Client::new(), &service("a.local.", &[unreachable, server.socket_address()])).await.unwrap();

        assert_eq!(probed.address, server.socket_address());
    }

    #[tokio::test]
    async fn probe_with_retry_publishes_the_probed_device() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        let (tx, rx) = watch::channel(ProbedDeviceMap::default());

        probe_with_retry(&Client::new(), &service("a.local.", &[server.socket_address()]), tx).await;

        assert_eq!(rx.borrow().keys().collect::<Vec<_>>(), [PLUS_1_ID]);
    }

    #[tokio::test]
    async fn probe_with_retry_keeps_a_device_known_at_the_same_address() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        let known = Arc::new(ProbedDevice { address: server.socket_address(), ..probed_device(PLUS_1_ID, &[]) });
        let (tx, rx) = watch::channel(Arc::new(HashMap::from([(PLUS_1_ID.to_string(), Arc::clone(&known))])));

        // Keep the sender alive, has_changed fails once it's dropped
        probe_with_retry(&Client::new(), &service("b.local.", &[server.socket_address()]), tx.clone()).await;

        assert!(!rx.has_changed().unwrap(), "receivers should not be notified");
        let probed = rx.borrow();
        assert_eq!(probed.len(), 1);
        assert!(Arc::ptr_eq(&probed[PLUS_1_ID], &known), "the known device should not be replaced");
    }

    #[tokio::test]
    async fn probe_with_retry_replaces_a_device_known_at_another_address() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        let known = Arc::new(probed_device(PLUS_1_ID, &[]));
        let (tx, rx) = watch::channel(Arc::new(HashMap::from([(PLUS_1_ID.to_string(), known)])));

        probe_with_retry(&Client::new(), &service("a.local.", &[server.socket_address()]), tx).await;

        let probed = rx.borrow();
        assert_eq!(probed.len(), 1);
        assert_eq!(probed[PLUS_1_ID].address, server.socket_address());
    }

    #[tokio::test]
    async fn probe_with_retry_gives_up_on_a_permanent_error() {
        let mut server = Server::new_async().await;
        let device_info = mock_device_info(&mut server, 404, "").await.expect(1);
        let (tx, rx) = watch::channel(ProbedDeviceMap::default());

        probe_with_retry(&Client::new(), &service("a.local.", &[server.socket_address()]), tx).await;

        device_info.assert();
        assert!(rx.borrow().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn probe_with_retry_retries_a_transient_error() {
        let mut server = Server::new_async().await;
        let failing = mock_device_info(&mut server, 500, "").await.expect(1);
        let (tx, mut rx) = watch::channel(ProbedDeviceMap::default());
        let address = server.socket_address();

        let handle = task::spawn(async move {
            probe_with_retry(&Client::new(), &service("a.local.", &[address]), tx).await;
        });

        // The first attempt fails; replace the failing mock before the retry fires
        while !failing.matched_async().await {
            tokio::task::yield_now().await;
        }
        failing.remove_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;

        rx.changed().await.unwrap();
        assert!(rx.borrow().contains_key(PLUS_1_ID));
        handle.await.unwrap();
    }
}
