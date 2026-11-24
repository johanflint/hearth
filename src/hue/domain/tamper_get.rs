use crate::hue::domain::HueOwner;
use chrono::{DateTime, Utc};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_tamper_get
#[derive(Debug, Deserialize)]
pub struct TamperGet {
    pub id: String,
    pub owner: HueOwner,
    pub tamper_reports: Vec<TamperReport>,
}

#[derive(Debug, Deserialize)]
pub struct TamperReport {
    pub changed: DateTime<Utc>,
    #[allow(dead_code)]
    pub source: TamperSource,
    pub state: TamperState,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TamperSource {
    BatteryDoor,
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TamperState {
    Tampered,
    NotTampered,
}

#[derive(Debug, Deserialize)]
pub struct TamperChanged {
    #[allow(dead_code)]
    pub id: String,
    pub owner: HueOwner,
    pub tamper_reports: Option<Vec<TamperReport>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::tampered("tampered", TamperState::Tampered)]
    #[case::not_tampered("not_tampered", TamperState::NotTampered)]
    fn deserialize_tamper_get_with_a_report(#[case] state: &str, #[case] expected: TamperState) {
        let json = json!({
            "id": "3b7e1c94-2f6a-4d8e-9c15-7a0d4e8b6f32",
            "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
            "tamper_reports": [{ "changed": "2026-09-23T15:41:08.512Z", "source": "battery_door", "state": state }],
            "type": "tamper"
        });

        let tamper_get = serde_json::from_value::<TamperGet>(json).unwrap();

        assert_eq!(tamper_get.owner.rid, "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77");
        assert_eq!(tamper_get.tamper_reports.len(), 1);
        let report = &tamper_get.tamper_reports[0];
        assert_eq!(report.source, TamperSource::BatteryDoor);
        assert_eq!(report.state, expected);
        assert_eq!(report.changed, "2026-09-23T15:41:08.512Z".parse::<DateTime<Utc>>().unwrap());
    }

    #[test]
    fn deserialize_tamper_get_without_reports() {
        let json = json!({
            "id": "3b7e1c94-2f6a-4d8e-9c15-7a0d4e8b6f32",
            "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
            "tamper_reports": [],
            "type": "tamper"
        });

        let tamper_get = serde_json::from_value::<TamperGet>(json).unwrap();

        assert!(tamper_get.tamper_reports.is_empty());
    }

    #[test]
    fn deserialize_tamper_get_with_an_unknown_state_fails() {
        let json = json!({
            "id": "3b7e1c94-2f6a-4d8e-9c15-7a0d4e8b6f32",
            "owner": { "rid": "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77", "rtype": "device" },
            "tamper_reports": [{ "changed": "2026-09-23T15:41:08.512Z", "source": "battery_door", "state": "opened" }],
            "type": "tamper"
        });

        assert!(serde_json::from_value::<TamperGet>(json).is_err());
    }
}
