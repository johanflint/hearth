use crate::domain::Connectivity;
use tracing::warn;

pub fn map_connectivity_status(status: &str) -> Connectivity {
    match status {
        "connected" => Connectivity::Connected,
        "connectivity_issue" | "unidirectional_incoming" => Connectivity::Issues,
        "disconnected" => Connectivity::Disconnected,
        _ => {
            warn!("⚠️ Unrecognized connectivity status: '{status}', defaulting to unknown");
            Connectivity::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::maps_connected("connected", Connectivity::Connected)]
    #[case::maps_connectivity_issue("connectivity_issue", Connectivity::Issues)]
    #[case::maps_unidirectional_incoming("unidirectional_incoming", Connectivity::Issues)]
    #[case::maps_disconnected("disconnected", Connectivity::Disconnected)]
    #[case::maps_unknown("some_unrecognized_value", Connectivity::Unknown)]
    fn test_map_connectivity_status(#[case] value: String, #[case] expected: Connectivity) {
        let result = map_connectivity_status(&value);
        assert_eq!(result, expected);
    }
}
