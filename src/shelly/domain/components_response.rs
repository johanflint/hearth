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
    Unsupported, // ble, cloud, input, mqtt, sys, wifi, ...
}

#[derive(Debug, Deserialize)]
struct RawComponent {
    key: String,
    status: serde_json::Value,
}

impl TryFrom<RawComponent> for Component {
    type Error = serde_json::Error;

    fn try_from(raw: RawComponent) -> Result<Self, Self::Error> {
        let kind = raw.key.split_once(':').map_or(raw.key.as_str(), |(kind, _)| kind);
        match kind {
            "switch" => Ok(Self::Switch { status: serde_json::from_value(raw.status)? }),
            _ => Ok(Self::Unsupported),
        }
    }
}

// Switch
#[derive(Debug, Deserialize)]
pub struct SwitchStatus {
    pub id: u32,
    pub output: bool,
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

        assert_eq!(unsupported, 6); // ble, cloud, input:0, mqtt, sys, wifi
    }

    #[test]
    fn fails_when_a_supported_component_is_malformed() {
        let json = r#"{"components":[{"key":"switch:0","status":{"id":0,"source":"init"},"config":{}}]}"#;

        let error = serde_json::from_str::<ComponentsResult>(json).unwrap_err();

        assert!(error.to_string().contains("output"), "unexpected error: {error}");
    }
}
