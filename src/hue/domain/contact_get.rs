use crate::hue::domain::Owner;
use chrono::{DateTime, Utc};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_contact_get
#[derive(Debug, Deserialize)]
pub struct ContactGet {
    pub id: String,
    pub owner: Owner,
    #[allow(dead_code)]
    pub enabled: bool,
    pub contact_report: Option<ContactReport>,
}

#[derive(Debug, Deserialize)]
pub struct ContactReport {
    pub changed: DateTime<Utc>,
    pub state: ContactState,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactState {
    Contact,    // The magnets touch, the door is closed
    NoContact,  // The magnets are apart, the door is open
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::closed("contact", ContactState::Contact)]
    #[case::open("no_contact", ContactState::NoContact)]
    fn deserialize_contact_get_with_a_report(#[case] state: &str, #[case] expected: ContactState) {
        let json = json!({
                    "id": "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21",
                    "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
                    "enabled": true,
                    "contact_report": { "changed": "2026-09-23T15:37:21.003Z", "state": state },
                    "type": "contact"
                });

        let contact_get = serde_json::from_value::<ContactGet>(json).unwrap();

        let report = contact_get.contact_report.unwrap();
        assert_eq!(contact_get.owner.rid, "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77");
        assert_eq!(report.state, expected);
        assert_eq!(report.changed, "2026-09-23T15:37:21.003Z".parse::<DateTime<Utc>>().unwrap());
    }

    #[test]
    fn deserialize_contact_get_without_a_report() {
        let json = json!({
                    "id": "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21",
                    "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
                    "enabled": true,
                    "type": "contact"
                });

        let contact_get = serde_json::from_value::<ContactGet>(json).unwrap();

        assert!(contact_get.contact_report.is_none());
    }

    #[test]
    fn deserialize_contact_get_with_an_unknown_state_fails() {
        let json = json!({
                    "id": "6d8f3b41-5b3c-4a52-8d1a-9f1b6a0c7e21",
                    "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
                    "enabled": true,
                    "contact_report": { "changed": "2026-09-23T15:37:21.003Z", "state": "ajar" },
                    "type": "contact"
                });

        assert!(serde_json::from_value::<ContactGet>(json).is_err());
    }
}
