use strum::EnumIter;

/// The connectivity of a `Device`. Not all mesh protocols support the
/// intermediary state `Issues`, Zigbee does but Z-Wave and Thread do not.
#[derive(EnumIter, Debug, PartialEq)]
pub enum Connectivity {
    Connected,
    Issues,
    Disconnected,
    Unknown,
}

impl Connectivity {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Connectivity::Connected => "connected",
            Connectivity::Issues => "issues",
            Connectivity::Disconnected => "disconnected",
            Connectivity::Unknown => "unknown",
        }
    }
}
