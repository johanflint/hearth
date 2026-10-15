use std::fmt;
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

impl fmt::Display for Connectivity {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::connected(Connectivity::Connected, "connected")]
    #[case::issues(Connectivity::Issues, "issues")]
    #[case::disconnected(Connectivity::Disconnected, "disconnected")]
    #[case::unknown(Connectivity::Unknown, "unknown")]
    fn display(#[case] connectivity: Connectivity, #[case] expected: &str) {
        assert_eq!(connectivity.to_string(), expected);
    }
}