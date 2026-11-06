use crate::mdns_browser::{DiscoveredService, DiscoveredServices, ServiceKey};
use crate::shelly::domain::{Component, ComponentsResult, DeviceInfo};
use crate::shelly::take_new::take_new;
use axum::http::StatusCode;
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::watch;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::sync::watch::Sender;
use tokio::task;
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
    let mut seen = HashSet::new();
    loop {
        let new = take_new(&mdns_rx.borrow_and_update(), &mut seen);
        for service in new {
            let client = client.clone();
            let tx = tx.clone();
            task::spawn(async move {
                probe_with_retry(&client, &service, tx).await;
            });
        }

        if mdns_rx.changed().await.is_err() {
            break; // all browse tasks ended, e.g. the daemon was shut down
        }
    }
}

async fn probe_with_retry(client: &Client, service: &DiscoveredService, tx: Sender<ProbedDeviceMap>) {
    let mut delay = Duration::from_secs(1);
    loop {
        match probe(client, service).await {
            Ok(probed_device) => {
                let id = probed_device.info.id.clone();
                let added = tx.send_if_modified(|devices| {
                    if devices.contains_key(&id) {
                        return false;
                    }
                    Arc::make_mut(devices).insert(id.clone(), Arc::new(probed_device));
                    true
                });
                if !added {
                    debug!("Skipping '{}', device '{id}' is already known", service.hostname);
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

async fn fetch<T>(
    client: &Client,
    address: &SocketAddr,
    method: &'static str,
    params: &[(&str, &str)],
) -> Result<T, ProbeError>
where
    T: DeserializeOwned,
{
    let response = client
        .get(format!("http://{}/rpc/{}", address, method))
        .query(params)
        .send()
        .await?
        .error_for_status()
        .map_err(to_probe_error)?;

    Ok(response.json::<T>().await?)
}

fn to_probe_error(e: reqwest::Error) -> ProbeError {
    if let (Some(status), Some(url)) = (e.status(), e.url()) {
        ProbeError::UnexpectedResponse(status, url.to_string())
    } else {
        ProbeError::ClientError(e)
    }
}

#[derive(Error, Debug)]
pub enum ProbeError {
    #[error("client error: {0}")]
    ClientError(#[from] reqwest::Error),
    #[error("no address")]
    NoAddress,
    #[error("unexpected status code {0} when calling {1}")]
    UnexpectedResponse(StatusCode, String),
}

impl ProbeError {
    // Whether the error is permanent or whether retrying helps
    fn is_permanent(&self) -> bool {
        match self {
            ProbeError::UnexpectedResponse(status, _) => status.is_client_error(),
            ProbeError::NoAddress => true,
            ProbeError::ClientError(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shelly::test_support::{PLUS_1_COMPONENTS, PLUS_1_DEVICE_INFO, PLUS_1_ID, probed_device};
    use mockito::{Matcher, Mock, Server, ServerGuard};
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

    #[test]
    fn take_new_returns_each_service_only_once() {
        let first = Arc::new(service("a.local.", &[]));
        let second = Arc::new(service("b.local.", &[]));
        let mut services = HashMap::from([(ServiceKey { hostname: "a.local.".to_string() }, first)]);
        let mut seen = HashSet::new();

        assert_eq!(take_new(&Arc::new(services.clone()), &mut seen).len(), 1);
        assert!(take_new(&Arc::new(services.clone()), &mut seen).is_empty());

        services.insert(ServiceKey { hostname: "b.local.".to_string() }, second);
        let new = take_new(&Arc::new(services), &mut seen);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].hostname, "b.local.");
    }

    #[rstest]
    #[case::unauthorized(ProbeError::UnexpectedResponse(StatusCode::UNAUTHORIZED, "url".to_string()), true)]
    #[case::not_found(ProbeError::UnexpectedResponse(StatusCode::NOT_FOUND, "url".to_string()), true)]
    #[case::server_error(ProbeError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, "url".to_string()), false)]
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

        assert!(matches!(error, ProbeError::UnexpectedResponse(StatusCode::NOT_FOUND, _)), "got {error:?}");
    }

    #[tokio::test]
    async fn probe_address_fails_when_the_components_are_protected() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        server.mock("GET", "/rpc/Shelly.GetComponents").match_query(Matcher::Any).with_status(401).create_async().await;

        let error = probe_address(&Client::new(), &server.socket_address()).await.unwrap_err();

        assert!(matches!(error, ProbeError::UnexpectedResponse(StatusCode::UNAUTHORIZED, _)), "got {error:?}");
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
    async fn probe_with_retry_keeps_an_already_known_device() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, PLUS_1_DEVICE_INFO).await;
        mock_components(&mut server).await;
        let known = Arc::new(probed_device(PLUS_1_ID, &[]));
        let (tx, rx) = watch::channel(Arc::new(HashMap::from([(PLUS_1_ID.to_string(), Arc::clone(&known))])));

        probe_with_retry(&Client::new(), &service("b.local.", &[server.socket_address()]), tx).await;

        let probed = rx.borrow();
        assert_eq!(probed.len(), 1);
        assert!(Arc::ptr_eq(&probed[PLUS_1_ID], &known), "the known device should not be replaced");
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
