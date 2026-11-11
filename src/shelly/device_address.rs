use std::fmt;
use std::fmt::Formatter;

/// Address where a Shelly device is reachable. The hostname is resolved from the mDNS cache
/// on every request by the `MdnsResolver`, so a new DHCP lease or a changed interface scope
/// id (for IPv6) is picked up without probing again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAddress {
    hostname: String,
    port: u16,
}

impl DeviceAddress {
    pub fn new(hostname: &str, port: u16) -> Self {
        // mDNS hostname is fully qualified ("name.local."), strip the trailing dot
        Self { hostname: hostname.trim_end_matches('.').to_string(), port }
    }
}

impl fmt::Display for DeviceAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.hostname, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_strips_the_trailing_dot_of_a_fully_qualified_hostname() {
        assert_eq!(DeviceAddress::new("shelly.local.", 80), DeviceAddress::new("shelly.local", 80));
    }

    #[test]
    fn displays_as_hostname_and_port() {
        assert_eq!(DeviceAddress::new("shelly.local.", 8080).to_string(), "shelly.local:8080");
    }

    #[test]
    fn addresses_with_another_port_differ() {
        assert_ne!(DeviceAddress::new("shelly.local", 80), DeviceAddress::new("shelly.local", 81));
    }
}
