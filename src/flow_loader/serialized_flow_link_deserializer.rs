use crate::flow_engine;
use crate::flow_loader::serialized_flow::SerializedFlowLink;
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

impl<'de> Deserialize<'de> for SerializedFlowLink {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Deserialize::deserialize(deserializer)?;
        match value {
            Value::String(node_id) => Ok(SerializedFlowLink {
                node_id,
                value: flow_engine::Value::None,
            }),
            Value::Object(map) => {
                let node = map
                    .get("node")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Error::custom("missing or invalid field 'node'"))?
                    .to_owned();
                let value_value = map.get("value");
                let value = match value_value {
                    Some(v) => flow_engine::Value::deserialize(v).map_err(Error::custom)?,
                    None => flow_engine::Value::None,
                };

                Ok(SerializedFlowLink { node_id: node, value })
            }
            _ => Err(Error::custom("a node id string or an object with 'node' and 'value'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    #[test]
    fn deserializes_a_string() {
        let parsed = serde_json::from_value::<SerializedFlowLink>(json!("node_id")).unwrap();
        assert_eq!(
            parsed,
            SerializedFlowLink {
                node_id: "node_id".to_string(),
                value: flow_engine::Value::None
            }
        );
    }

    #[test]
    fn deserializes_a_link_without_value() {
        let parsed = serde_json::from_value::<SerializedFlowLink>(json!({ "node": "node_id" })).unwrap();
        assert_eq!(
            parsed,
            SerializedFlowLink {
                node_id: "node_id".to_string(),
                value: flow_engine::Value::None
            }
        );
    }

    #[rstest]
    #[case::with_boolean(json!(true), flow_engine::Value::Boolean(true))]
    #[case::with_number(json!(0.2), flow_engine::Value::Number(Number::Float(0.2)))]
    fn deserializes_a_link_with_value(#[case] value: Value, #[case] expected: flow_engine::Value) {
        let parsed = serde_json::from_value::<SerializedFlowLink>(json!({ "node": "node_id", "value": value })).unwrap();
        assert_eq!(
            parsed,
            SerializedFlowLink {
                node_id: "node_id".to_string(),
                value: expected
            }
        );
    }
}
