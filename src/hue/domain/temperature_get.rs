use crate::hue::domain::Owner;
use chrono::{DateTime, Utc};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_temperature_get
#[derive(Debug, Deserialize)]
pub struct TemperatureGet {
    #[allow(dead_code)]
    pub id: String,
    pub owner: Owner,
    #[allow(dead_code)]
    pub enabled: bool,
    pub temperature: Temperature,
}

#[derive(Debug, Deserialize)]
pub struct Temperature {
    // temperature and temperature_valid are deprecated and thus left out
    pub temperature_report: Option<TemperatureReport>,
}

#[derive(Debug, Deserialize)]
pub struct TemperatureReport {
    pub changed: DateTime<Utc>,
    pub temperature: f64, // degrees Celsius
}

#[derive(Debug, Deserialize)]
pub struct TemperatureChanged {
    #[allow(dead_code)]
    pub id: String,
    pub owner: Owner,
    pub temperature: Option<Temperature>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn deserialize_temperature_get_with_a_report() {
        let json = json!({
                    "id": "197f1e17-f04f-471d-b410-77b9267826e9",
                    "owner": { "rid": "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3", "rtype": "device" },
                    "enabled": true,
                    "temperature": {
                        "temperature": 21.34,
                        "temperature_valid": true,
                        "temperature_report": { "changed": "2026-09-23T15:37:21.003Z", "temperature": 21.34 }
                    },
                    "type": "temperature"
                });

        let temperature_get = serde_json::from_value::<TemperatureGet>(json).unwrap();

        let report = temperature_get.temperature.temperature_report.unwrap();
        assert_eq!(temperature_get.owner.rid, "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3");
        assert_eq!(report.temperature, 21.34);
        assert_eq!(report.changed, Utc.with_ymd_and_hms(2026, 9, 23, 15, 37, 21).unwrap() + chrono::Duration::milliseconds(3));
    }

    #[test]
    fn deserialize_temperature_get_without_a_report() {
        let json = json!({
                    "id": "197f1e17-f04f-471d-b410-77b9267826e9",
                    "owner": { "rid": "dbe91174-8fe5-4309-b7d7-3f15ec1e57d3", "rtype": "device" },
                    "enabled": true,
                    "temperature": { "temperature_valid": false },
                    "type": "temperature"
                });

        let temperature_get = serde_json::from_value::<TemperatureGet>(json).unwrap();

        assert!(temperature_get.temperature.temperature_report.is_none());
    }
}