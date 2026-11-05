use crate::domain::{Device, Event};
use crate::mdns_browser::{DiscoveredService, DiscoveredServices, ServiceKey};
use crate::shelly::domain::{ComponentsResult, DeviceInfo};
use crate::shelly::map_shelly::map_shelly;
use axum::http::StatusCode;
use reqwest::Client;
use serde::de::DeserializeOwned;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;
use tokio::task;
use tracing::{debug, warn};

pub fn probe_announced_devices(
    client: &Client,
    mdns_rx: WatchReceiver<DiscoveredServices>,
    tx: Sender<Event>,
) -> Result<(), ProbeError> {
    let client = client.clone();
    task::spawn(async move {
        listen(tx, &client, mdns_rx).await;
    });
    Ok(())
}

async fn listen(tx: Sender<Event>, client: &Client, mut mdns_rx: WatchReceiver<DiscoveredServices>) {
    let mut seen = HashSet::new();
    let known_device_ids = Arc::new(Mutex::new(HashSet::new()));
    loop {
        let new = take_new(&mdns_rx.borrow_and_update(), &mut seen);
        for service in new {
            let client = client.clone();
            let tx = tx.clone();
            let known_device_ids = Arc::clone(&known_device_ids);
            task::spawn(async move {
                probe_with_retry(&client, &service, tx, &known_device_ids).await;
            });
        }

        if mdns_rx.changed().await.is_err() {
            break; // all browse tasks ended, e.g. the daemon was shut down
        }
    }
}

fn take_new(services: &DiscoveredServices, seen: &mut HashSet<ServiceKey>) -> Vec<Arc<DiscoveredService>> {
    services
        .iter()
        .filter(|(service_key, _)| seen.insert((*service_key).clone()))
        .map(|(_, service)| Arc::clone(service))
        .collect()
}

async fn probe_with_retry(
    client: &Client,
    service: &DiscoveredService,
    tx: Sender<Event>,
    known_device_ids: &Mutex<HashSet<String>>,
) {
    let mut delay = Duration::from_secs(1);
    loop {
        match probe(client, service).await {
            Ok(device) => {
                if !known_device_ids.lock().expect("known device ids lock poisoned").insert(device.id.clone()) {
                    debug!("Skipping '{}', device '{}' is already known", service.hostname, device.id);
                    return;
                }
                if tx.send(Event::DiscoveredDevices(vec![device])).await.is_err() {
                    warn!("⚠️ Store unavailable, dropping '{}'", service.hostname);
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

async fn probe(client: &Client, service: &DiscoveredService) -> Result<Device, ProbeError> {
    let mut last_err = ProbeError::NoAddress;
    for address in &service.addresses {
        match probe_address(client, address).await {
            Ok(device) => return Ok(device),
            Err(err) => last_err = err,
        }
    }
    Err(last_err)
}

async fn probe_address(client: &Client, address: &SocketAddr) -> Result<Device, ProbeError> {
    let device_info = fetch::<DeviceInfo>(client, address, "Shelly.GetDeviceInfo", &[]).await?;
    let components_response =
        fetch::<ComponentsResult>(client, address, "Shelly.GetComponents", &[("include", r#"["config","status"]"#)])
            .await?;

    map_shelly(device_info, components_response, address.to_string()).ok_or(ProbeError::UnsupportedDevice)
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
    #[error("unsupported device")]
    UnsupportedDevice,
    #[error("unexpected status code {0} when calling {1}")]
    UnexpectedResponse(StatusCode, String),
}

impl ProbeError {
    // Whether the error is permanent or whether retrying helps
    fn is_permanent(&self) -> bool {
        match self {
            ProbeError::UnexpectedResponse(status, _) => status.is_client_error(),
            ProbeError::NoAddress => true,
            ProbeError::UnsupportedDevice => true,
            ProbeError::ClientError(_) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::DeviceType;
    use mockito::{Matcher, Mock, Server, ServerGuard};
    use rstest::rstest;
    use std::collections::{BTreeSet, HashMap};
    use tokio::sync::mpsc;

    const DEVICE_INFO: &str = include_str!("../../tests/resources/shelly_device_info_plus1_response.json");
    const COMPONENTS: &str = include_str!("../../tests/resources/shelly_components_plus1_response.json");
    const DEVICE_ID: &str = "shellyplus1-aabbccddeeff";

    fn service(hostname: &str, addresses: &[SocketAddr]) -> DiscoveredService {
        DiscoveredService {
            hostname: hostname.to_string(),
            addresses: BTreeSet::from_iter(addresses.iter().copied()),
            txt: HashMap::new(),
        }
    }

    fn service_key(hostname: &str) -> ServiceKey {
        ServiceKey { service_type: "_shelly._tcp.local.".to_string(), hostname: hostname.to_string() }
    }

    async fn mock_device_info(server: &mut ServerGuard, status: usize, body: &str) -> Mock {
        server.mock("GET", "/rpc/Shelly.GetDeviceInfo").with_status(status).with_body(body).create_async().await
    }

    async fn mock_components(server: &mut ServerGuard) -> Mock {
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::UrlEncoded("include".to_string(), r#"["config","status"]"#.to_string()))
            .with_body(COMPONENTS)
            .create_async()
            .await
    }

    #[test]
    fn take_new_returns_each_service_only_once() {
        let first = Arc::new(service("a.local.", &[]));
        let second = Arc::new(service("b.local.", &[]));
        let mut services = HashMap::from([(service_key("a.local."), first)]);
        let mut seen = HashSet::new();

        assert_eq!(take_new(&Arc::new(services.clone()), &mut seen).len(), 1);
        assert!(take_new(&Arc::new(services.clone()), &mut seen).is_empty());

        services.insert(service_key("b.local."), second);
        let new = take_new(&Arc::new(services), &mut seen);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].hostname, "b.local.");
    }

    #[rstest]
    #[case::unauthorized(ProbeError::UnexpectedResponse(StatusCode::UNAUTHORIZED, "url".to_string()), true)]
    #[case::not_found(ProbeError::UnexpectedResponse(StatusCode::NOT_FOUND, "url".to_string()), true)]
    #[case::server_error(ProbeError::UnexpectedResponse(StatusCode::INTERNAL_SERVER_ERROR, "url".to_string()), false)]
    #[case::no_address(ProbeError::NoAddress, true)]
    #[case::unsupported(ProbeError::UnsupportedDevice, true)]
    fn is_permanent(#[case] error: ProbeError, #[case] expected: bool) {
        assert_eq!(error.is_permanent(), expected);
    }

    #[tokio::test]
    async fn probe_address_returns_the_device() {
        let mut server = Server::new_async().await;
        let device_info = mock_device_info(&mut server, 200, DEVICE_INFO).await;
        let components = mock_components(&mut server).await;
        let address = server.socket_address();

        let device = probe_address(&Client::new(), &address).await.unwrap();

        assert_eq!(device.id, DEVICE_ID);
        assert_eq!(device.r#type, DeviceType::Switch);
        assert_eq!(device.address, Some(address.to_string()));
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
    async fn probe_address_fails_on_a_device_without_switches() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, DEVICE_INFO).await;
        server
            .mock("GET", "/rpc/Shelly.GetComponents")
            .match_query(Matcher::Any)
            .with_body(r#"{"components":[{"key":"ble","status":{},"config":{}}]}"#)
            .create_async()
            .await;

        let error = probe_address(&Client::new(), &server.socket_address()).await.unwrap_err();

        assert!(matches!(error, ProbeError::UnsupportedDevice), "got {error:?}");
    }

    #[tokio::test]
    async fn probe_without_addresses_fails_with_no_address() {
        let error = probe(&Client::new(), &service("a.local.", &[])).await.unwrap_err();

        assert!(matches!(error, ProbeError::NoAddress), "got {error:?}");
    }

    #[tokio::test]
    async fn probe_with_retry_sends_the_discovered_device() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, DEVICE_INFO).await;
        mock_components(&mut server).await;
        let (tx, mut rx) = mpsc::channel(1);
        let known_device_ids = Mutex::new(HashSet::new());

        probe_with_retry(&Client::new(), &service("a.local.", &[server.socket_address()]), tx, &known_device_ids).await;

        let Some(Event::DiscoveredDevices(devices)) = rx.recv().await else {
            panic!("expected a DiscoveredDevices event");
        };
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, DEVICE_ID);
        assert!(known_device_ids.lock().unwrap().contains(DEVICE_ID));
    }

    #[tokio::test]
    async fn probe_with_retry_skips_an_already_known_device() {
        let mut server = Server::new_async().await;
        mock_device_info(&mut server, 200, DEVICE_INFO).await;
        mock_components(&mut server).await;
        let (tx, mut rx) = mpsc::channel(1);
        let known_device_ids = Mutex::new(HashSet::from([DEVICE_ID.to_string()]));

        probe_with_retry(&Client::new(), &service("b.local.", &[server.socket_address()]), tx, &known_device_ids).await;

        assert!(rx.recv().await.is_none(), "no event expected for a known device");
    }

    #[tokio::test]
    async fn probe_with_retry_gives_up_on_a_permanent_error() {
        let mut server = Server::new_async().await;
        let device_info = mock_device_info(&mut server, 404, "").await.expect(1);
        let (tx, mut rx) = mpsc::channel(1);

        probe_with_retry(
            &Client::new(),
            &service("a.local.", &[server.socket_address()]),
            tx,
            &Mutex::new(HashSet::new()),
        )
        .await;

        device_info.assert();
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn probe_with_retry_retries_a_transient_error() {
        let mut server = Server::new_async().await;
        let failing = mock_device_info(&mut server, 500, "").await.expect(1);
        let (tx, mut rx) = mpsc::channel(1);
        let address = server.socket_address();
        let known_device_ids = Arc::new(Mutex::new(HashSet::new()));

        let handle = {
            let known_device_ids = Arc::clone(&known_device_ids);
            task::spawn(async move {
                probe_with_retry(&Client::new(), &service("a.local.", &[address]), tx, &known_device_ids).await;
            })
        };

        // The first attempt fails; replace the failing mock before the retry fires
        while !failing.matched_async().await {
            tokio::task::yield_now().await;
        }
        failing.remove_async().await;
        mock_device_info(&mut server, 200, DEVICE_INFO).await;
        mock_components(&mut server).await;

        let Some(Event::DiscoveredDevices(devices)) = rx.recv().await else {
            panic!("expected a DiscoveredDevices event");
        };
        assert_eq!(devices[0].id, DEVICE_ID);
        handle.await.unwrap();
    }
}
