use mdns_sd::{ResolvedService, ScopedIp, ServiceDaemon, ServiceEvent};
use std::collections::{BTreeSet, HashMap};
use std::net::{SocketAddr, SocketAddrV6};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch::{Receiver as WatchReceiver, Sender, channel};
use tokio::task;
use tracing::{debug, info, warn};

pub struct MdnsBrowser {
    daemon: ServiceDaemon,
}

impl MdnsBrowser {
    pub fn spawn() -> Result<MdnsBrowser, MdnsBrowserError> {
        let daemon = ServiceDaemon::new().map_err(MdnsBrowserError::DaemonCreationFailed)?;
        Ok(MdnsBrowser { daemon })
    }

    pub fn browse(&self, service_type: &str) -> Result<WatchReceiver<DiscoveredServices>, MdnsBrowserError> {
        let receiver = self
            .daemon
            .browse(service_type)
            .map_err(|source| MdnsBrowserError::BrowseFailed { service_type: service_type.to_string(), source })?;
        let (tx, rx) = channel(DiscoveredServices::default());
        task::spawn(browse(receiver, tx));
        Ok(rx)
    }
}

impl Drop for MdnsBrowser {
    fn drop(&mut self) {
        if let Err(err) = self.daemon.shutdown() {
            warn!("⚠️ mDNS daemon shutdown failed: {err}");
        }
    }
}

async fn browse(receiver: mdns_sd::Receiver<ServiceEvent>, tx: Sender<DiscoveredServices>) {
    while let Ok(event) = receiver.recv_async().await {
        // Send modify also stores the value when there are no listeners, send doesn't
        tx.send_modify(|services| handle_event(event, Arc::make_mut(services)));
    }

    warn!("⚠️ mDNS browse channel terminated");
}

fn handle_event(event: ServiceEvent, services: &mut HashMap<ServiceKey, Arc<DiscoveredService>>) {
    match event {
        ServiceEvent::ServiceResolved(resolved) => {
            let service_type = resolved.ty_domain.clone();
            let key = ServiceKey { hostname: resolved.host.clone() };
            match services.insert(key.clone(), Arc::new(DiscoveredService::from(*resolved))) {
                None => info!("🔎 Discovered {service_type} on {}", key.hostname),
                Some(_) => debug!("🔎 Re-resolved {service_type} on {}", key.hostname),
            }
        }
        // Devices with weak signal leave and return, so ignore removals
        other => debug!("mDNS event: {other:?}"),
    }
}

#[derive(Error, Debug)]
pub enum MdnsBrowserError {
    #[error("mDNS daemon creation failed: {0}")]
    DaemonCreationFailed(#[source] mdns_sd::Error),
    #[error("browsing for '{service_type}' failed: {source}")]
    BrowseFailed { service_type: String, source: mdns_sd::Error },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServiceKey {
    pub hostname: String,
}

pub type DiscoveredServices = Arc<HashMap<ServiceKey, Arc<DiscoveredService>>>;

#[derive(Debug)]
pub struct DiscoveredService {
    pub hostname: String,
    pub port: u16,
    pub addresses: BTreeSet<SocketAddr>,
    #[allow(dead_code)]
    pub txt: HashMap<String, String>,
}

impl From<ResolvedService> for DiscoveredService {
    fn from(resolved: ResolvedService) -> Self {
        DiscoveredService {
            hostname: resolved.host,
            port: resolved.port,
            addresses: resolved.addresses.iter().map(|address| socket_addr(address, resolved.port)).collect(),
            txt: resolved.txt_properties.into_property_map_str(),
        }
    }
}

fn socket_addr(address: &ScopedIp, port: u16) -> SocketAddr {
    match address {
        ScopedIp::V6(v6) => SocketAddr::V6(SocketAddrV6::new(*v6.addr(), port, 0, v6.scope_id().index)),
        v4 => SocketAddr::new(v4.to_ip_addr(), port),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::ServiceInfo;
    use pretty_assertions::assert_eq;
    use std::net::IpAddr;

    const SHELLY: &str = "_shelly._tcp.local.";

    fn addr(address: &str) -> SocketAddr {
        address.parse().expect("valid socket address")
    }

    // ResolvedService is non_exhaustive, so it can only be created through ServiceInfo
    fn resolved(host: &str, addresses: &str, port: u16, txt: &[(&str, &str)]) -> ResolvedService {
        ServiceInfo::new(SHELLY, "instance", host, addresses, port, txt)
            .expect("valid service info")
            .as_resolved_service()
    }

    fn resolved_event(service: ResolvedService) -> ServiceEvent {
        ServiceEvent::ServiceResolved(Box::new(service))
    }

    fn key(host: &str) -> ServiceKey {
        ServiceKey { hostname: host.to_string() }
    }

    #[test]
    fn handle_event_inserts_resolved_service_by_host() {
        let mut services = HashMap::new();

        handle_event(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[("gen", "2")])), &mut services);

        assert_eq!(services.len(), 1);
        let service = &services[&key("shelly-1.local.")];
        assert_eq!(service.hostname, "shelly-1.local.");
        assert_eq!(service.addresses, BTreeSet::from([addr("192.168.1.10:80")]));
    }

    #[test]
    fn handle_event_keeps_services_of_different_hosts_separate() {
        let mut services = HashMap::new();

        handle_event(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[])), &mut services);
        handle_event(resolved_event(resolved("shelly-2.local.", "192.168.1.11", 80, &[])), &mut services);

        assert_eq!(services.len(), 2);
        assert!(services.contains_key(&key("shelly-1.local.")));
        assert!(services.contains_key(&key("shelly-2.local.")));
    }

    #[test]
    fn handle_event_replaces_service_with_same_host() {
        let mut services = HashMap::new();

        handle_event(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[])), &mut services);
        handle_event(resolved_event(resolved("shelly-1.local.", "192.168.1.20", 8080, &[])), &mut services);

        assert_eq!(services.len(), 1);
        let service = &services[&key("shelly-1.local.")];
        assert_eq!(service.addresses, BTreeSet::from([addr("192.168.1.20:8080")]));
    }

    #[test]
    fn handle_event_ignores_removal_so_devices_with_weak_signal_are_kept() {
        let mut services = HashMap::new();
        handle_event(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[])), &mut services);

        handle_event(ServiceEvent::ServiceRemoved(SHELLY.to_string(), "shelly-1.local.".to_string()), &mut services);

        assert!(services.contains_key(&key("shelly-1.local.")));
    }

    #[test]
    fn handle_event_ignores_events_that_are_not_resolutions() {
        let mut services = HashMap::new();

        handle_event(ServiceEvent::SearchStarted(SHELLY.to_string()), &mut services);
        handle_event(ServiceEvent::ServiceFound(SHELLY.to_string(), "shelly-1.local.".to_string()), &mut services);
        handle_event(ServiceEvent::SearchStopped(SHELLY.to_string()), &mut services);

        assert!(services.is_empty());
    }

    #[test]
    fn discovered_service_is_built_from_resolved_service() {
        let service =
            DiscoveredService::from(resolved("shelly-1.local.", "192.168.1.10", 80, &[("gen", "2"), ("app", "Plus1")]));

        assert_eq!(service.hostname, "shelly-1.local.");
        assert_eq!(service.addresses, BTreeSet::from([addr("192.168.1.10:80")]));
        assert_eq!(
            service.txt,
            HashMap::from([("gen".to_string(), "2".to_string()), ("app".to_string(), "Plus1".to_string())])
        );
    }

    #[test]
    fn discovered_service_collects_ipv4_and_ipv6_addresses_in_sorted_order() {
        let service = DiscoveredService::from(resolved("shelly-1.local.", "fe80::1, 192.168.1.10, 10.0.0.1", 80, &[]));

        assert_eq!(
            service.addresses.into_iter().collect::<Vec<_>>(),
            vec![addr("10.0.0.1:80"), addr("192.168.1.10:80"), addr("[fe80::1]:80")]
        );
    }

    #[test]
    fn discovered_service_combines_every_address_with_the_service_port() {
        let service = DiscoveredService::from(resolved("shelly-1.local.", "192.168.1.10, 192.168.1.11", 8080, &[]));

        assert_eq!(service.addresses, BTreeSet::from([addr("192.168.1.10:8080"), addr("192.168.1.11:8080")]));
    }

    #[test]
    fn socket_addr_combines_an_ipv4_address_with_the_port() {
        let address = ScopedIp::from("192.168.1.10".parse::<IpAddr>().unwrap());

        assert_eq!(socket_addr(&address, 80), addr("192.168.1.10:80"));
    }

    #[test]
    fn socket_addr_combines_an_ipv6_address_with_the_port_and_a_zero_scope_when_the_scope_is_unknown() {
        let address = ScopedIp::from("fe80::1".parse::<IpAddr>().unwrap());

        let SocketAddr::V6(socket_addr) = socket_addr(&address, 80) else {
            panic!("expected an IPv6 socket address");
        };

        assert_eq!(socket_addr.port(), 80);
        assert_eq!(socket_addr.scope_id(), 0);
        assert_eq!(socket_addr.to_string(), "[fe80::1]:80");
    }

    #[test]
    fn discovered_service_without_txt_properties_has_empty_txt() {
        let service = DiscoveredService::from(resolved("shelly-1.local.", "192.168.1.10", 80, &[]));

        assert!(service.txt.is_empty());
    }

    #[test]
    fn daemon_creation_error_message_contains_the_source() {
        let err = MdnsBrowserError::DaemonCreationFailed(mdns_sd::Error::Msg("no socket".to_string()));

        assert_eq!(err.to_string(), "mDNS daemon creation failed: no socket");
    }

    #[test]
    fn browse_error_message_contains_service_type_and_source() {
        let err =
            MdnsBrowserError::BrowseFailed { service_type: SHELLY.to_string(), source: mdns_sd::Error::DaemonShutdown };

        assert_eq!(err.to_string(), "browsing for '_shelly._tcp.local.' failed: daemon has shut down");
    }

    #[tokio::test]
    async fn browse_publishes_resolved_services_to_listeners() {
        let (event_tx, event_rx) = flume::unbounded();
        let (tx, mut rx) = channel(DiscoveredServices::default());
        let task = task::spawn(browse(event_rx, tx));

        event_tx.send(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[]))).unwrap();
        rx.changed().await.unwrap();

        assert!(rx.borrow().contains_key(&key("shelly-1.local.")));

        drop(event_tx);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn browse_stores_services_when_there_are_no_listeners() {
        let (event_tx, event_rx) = flume::unbounded();
        let (tx, rx) = channel(DiscoveredServices::default());
        drop(rx);
        let observer = tx.clone();
        let task = task::spawn(browse(event_rx, tx));

        event_tx.send(resolved_event(resolved("shelly-1.local.", "192.168.1.10", 80, &[]))).unwrap();
        drop(event_tx);
        task.await.unwrap();

        assert!(observer.borrow().contains_key(&key("shelly-1.local.")));
    }

    #[tokio::test]
    async fn browse_ends_when_the_event_channel_is_closed() {
        let (event_tx, event_rx) = flume::unbounded();
        let (tx, _rx) = channel(DiscoveredServices::default());
        let task = task::spawn(browse(event_rx, tx));

        drop(event_tx);

        task.await.unwrap();
    }

    #[tokio::test]
    async fn browser_starts_each_browse_with_an_empty_snapshot() {
        let browser = MdnsBrowser::spawn().unwrap();

        let first = browser.browse(SHELLY).unwrap();
        let second = browser.browse("_hue._tcp.local.").unwrap();

        assert!(first.borrow().is_empty());
        assert!(second.borrow().is_empty());
    }

    #[tokio::test]
    async fn browser_fails_for_an_invalid_service_type() {
        let browser = MdnsBrowser::spawn().unwrap();

        let result = browser.browse("_shelly._tcp");

        assert!(
            matches!(result, Err(MdnsBrowserError::BrowseFailed { service_type, .. }) if service_type == "_shelly._tcp")
        );
    }
}
