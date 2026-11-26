use crate::mdns_browser::DiscoveredServices;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::sync::watch::Receiver as WatchReceiver;

pub struct MdnsResolver {
    services_rx: WatchReceiver<DiscoveredServices>,
}

impl MdnsResolver {
    pub fn new(services_rx: WatchReceiver<DiscoveredServices>) -> MdnsResolver {
        MdnsResolver { services_rx }
    }
}

impl Resolve for MdnsResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let addresses = self
            .services_rx
            .borrow()
            .values()
            .find(|s| s.hostname.trim_end_matches('.').eq_ignore_ascii_case(name.as_str()))
            .map(|s| s.addresses.iter().copied().collect::<Vec<_>>());
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addresses = addresses.ok_or_else(|| format!("unknown mDNS host '{host}'"))?;
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mdns_browser::{DiscoveredService, ServiceKey};
    use std::collections::{BTreeSet, HashMap};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::sync::watch;

    fn services(hostname: &str, addresses: &[&str]) -> DiscoveredServices {
        let service = DiscoveredService {
            hostname: hostname.to_string(),
            port: 80,
            addresses: BTreeSet::from_iter(addresses.iter().map(|a| a.parse::<SocketAddr>().unwrap())),
            txt: HashMap::new(),
        };
        Arc::new(HashMap::from([(ServiceKey { hostname: hostname.to_string() }, Arc::new(service))]))
    }

    async fn resolve(resolver: &MdnsResolver, name: &str) -> Result<Vec<SocketAddr>, String> {
        let addrs = resolver.resolve(name.parse().unwrap()).await.map_err(|err| err.to_string())?;
        Ok(addrs.collect())
    }

    #[tokio::test]
    async fn resolves_a_hostname_without_its_trailing_dot() {
        let (_tx, rx) = watch::channel(services("shelly.local.", &["192.168.1.35:80"]));

        let addresses = resolve(&MdnsResolver::new(rx), "shelly.local").await.unwrap();

        assert_eq!(addresses, ["192.168.1.35:80".parse().unwrap()]);
    }

    #[tokio::test]
    async fn resolves_ignoring_the_case_of_the_hostname() {
        let (_tx, rx) = watch::channel(services("ShellyPresence-E8F60A7937EC.local.", &["192.168.1.35:80"]));

        let addresses = resolve(&MdnsResolver::new(rx), "shellypresence-e8f60a7937ec.local").await.unwrap();

        assert_eq!(addresses.len(), 1);
    }

    #[tokio::test]
    async fn resolves_both_ipv4_and_scoped_ipv6_addresses() {
        let (_tx, rx) = watch::channel(services("shelly.local.", &["192.168.1.35:80", "[fe80::1%4]:80"]));

        let addresses = resolve(&MdnsResolver::new(rx), "shelly.local").await.unwrap();

        assert_eq!(addresses.len(), 2);
        assert!(addresses.iter().any(|a| matches!(a, SocketAddr::V6(v6) if v6.scope_id() == 4)), "got {addresses:?}");
    }

    #[tokio::test]
    async fn fails_for_an_unknown_hostname() {
        let (_tx, rx) = watch::channel(services("shelly.local.", &["192.168.1.35:80"]));

        let error = resolve(&MdnsResolver::new(rx), "other.local").await.unwrap_err();

        assert_eq!(error, "unknown mDNS host 'other.local'");
    }

    #[tokio::test]
    async fn picks_up_addresses_announced_after_creation() {
        let (tx, rx) = watch::channel(services("shelly.local.", &["192.168.1.35:80"]));
        let resolver = MdnsResolver::new(rx);

        tx.send(services("shelly.local.", &["192.168.1.99:80"])).unwrap();

        let addresses = resolve(&resolver, "shelly.local").await.unwrap();
        assert_eq!(addresses, ["192.168.1.99:80".parse().unwrap()]);
    }
}
