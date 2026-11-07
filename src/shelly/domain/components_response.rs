use serde::Deserialize;

// API: https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Shelly/#shellygetcomponents
#[derive(Debug, Deserialize)]
pub struct ComponentsResult {
    pub components: Vec<Component>,
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "RawComponent")]
pub enum Component {
    Switch { status: SwitchStatus },
    Mqtt { config: MqttConfig, status: MqttStatus },
    Unsupported, // ble, cloud, input, sys, wifi, ...
}

#[derive(Debug, Deserialize)]
struct RawComponent {
    key: String,
    status: serde_json::Value,
    #[serde(default)]
    config: serde_json::Value,
}

impl TryFrom<RawComponent> for Component {
    type Error = serde_json::Error;

    fn try_from(raw: RawComponent) -> Result<Self, Self::Error> {
        let kind = raw.key.split_once(':').map_or(raw.key.as_str(), |(kind, _)| kind);
        match kind {
            "switch" => Ok(Self::Switch { status: serde_json::from_value(raw.status)? }),
            "mqtt" => Ok(Self::Mqtt {
                config: serde_json::from_value(raw.config)?,
                status: serde_json::from_value(raw.status)?,
            }),
            _ => Ok(Self::Unsupported),
        }
    }
}

// Switch
#[derive(Debug, PartialEq, Deserialize)]
pub struct SwitchStatus {
    pub id: u32,
    pub output: bool,
}

// Mqtt, API: https://shelly-api-docs.shelly.cloud/gen2/ComponentsAndServices/Mqtt
#[derive(Debug, PartialEq, Deserialize)]
pub struct MqttConfig {
    pub enable: bool,
    pub server: Option<String>,
    pub client_id: Option<String>, // if None it uses the device id as client_id
    pub user: Option<String>,      // pass is write-only, never returned
    pub topic_prefix: Option<String>,
    pub rpc_ntf: bool,
    pub status_ntf: bool,
    pub enable_rpc: bool,
    pub enable_control: bool,
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct MqttStatus {
    pub connected: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plus_1_components() -> ComponentsResult {
        serde_json::from_str(include_str!("../../../tests/resources/shelly_components_plus1_response.json")).unwrap()
    }

    #[test]
    fn deserializes_the_components_of_a_plus_1() {
        let result = plus_1_components();

        assert_eq!(result.components.len(), 7);
        assert!(
            result
                .components
                .iter()
                .any(|c| matches!(c, Component::Switch { status: SwitchStatus { id: 0, output: false } }))
        );
    }

    #[test]
    fn treats_components_without_a_model_as_unsupported() {
        let unsupported = plus_1_components().components.iter().filter(|c| matches!(c, Component::Unsupported)).count();

        assert_eq!(unsupported, 5); // ble, cloud, input:0, sys, wifi
    }

    #[test]
    fn deserializes_the_mqtt_component_of_a_plus_1() {
        let result = plus_1_components();

        let mqtt = result.components.iter().find_map(|c| match c {
            Component::Mqtt { config, status } => Some((config, status)),
            _ => None,
        });

        let expected_config = MqttConfig {
            enable: false,
            server: None,
            client_id: Some("shellyplus1-aabbccddeeff".to_string()),
            user: None,
            topic_prefix: Some("shellyplus1-aabbccddeeff".to_string()),
            rpc_ntf: true,
            status_ntf: false,
            enable_rpc: false,
            enable_control: true,
        };
        assert_eq!(mqtt, Some((&expected_config, &MqttStatus { connected: false })));
    }

    #[test]
    fn fails_when_a_supported_component_is_malformed() {
        let json = r#"{"components":[{"key":"switch:0","status":{"id":0,"source":"init"},"config":{}}]}"#;

        let error = serde_json::from_str::<ComponentsResult>(json).unwrap_err();

        assert!(error.to_string().contains("output"), "unexpected error: {error}");
    }

    #[test]
    fn fails_when_the_mqtt_component_has_no_config() {
        let json = r#"{"components":[{"key":"mqtt","status":{"connected":false}}]}"#;

        let error = serde_json::from_str::<ComponentsResult>(json).unwrap_err();

        assert!(error.to_string().contains("invalid type: null"), "unexpected error: {error}");
    }
}
