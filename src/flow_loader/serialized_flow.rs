use crate::flow_engine::action::Action;
use crate::flow_engine::{Expression, Schedule, Value};
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct SerializedFlow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) schedule: Option<Schedule>,
    pub(crate) trigger: Option<Expression>,
    pub(crate) nodes: Vec<SerializedFlowNode>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SerializedFlowNode {
    StartNode(SerializedStartFlowNode),
    EndNode(SerializedEndFlowNode),
    ConditionalNode(SerializedConditionalFlowNode),
    ActionNode(SerializedActionFlowNode),
    SleepNode(SerializedSleepFlowNode),
}

impl SerializedFlowNode {
    pub fn id(&self) -> &str {
        match self {
            SerializedFlowNode::StartNode(node) => &node.id,
            SerializedFlowNode::EndNode(node) => &node.id,
            SerializedFlowNode::ConditionalNode(node) => &node.id,
            SerializedFlowNode::ActionNode(node) => &node.id,
            SerializedFlowNode::SleepNode(node) => &node.id,
        }
    }

    pub fn outgoing_nodes(&self) -> Vec<&SerializedFlowLink> {
        match self {
            SerializedFlowNode::StartNode(node) => vec![&node.outgoing_node],
            SerializedFlowNode::EndNode(_) => vec![],
            SerializedFlowNode::ConditionalNode(node) => node.outgoing_nodes.iter().collect(),
            SerializedFlowNode::ActionNode(node) => vec![&node.outgoing_node],
            SerializedFlowNode::SleepNode(node) => vec![&node.outgoing_node],
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub struct SerializedStartFlowNode {
    pub(crate) id: String,
    pub(crate) outgoing_node: SerializedFlowLink,
}

#[derive(Debug, Deserialize)]
pub struct SerializedEndFlowNode {
    pub(crate) id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub struct SerializedConditionalFlowNode {
    pub(crate) id: String,
    pub(crate) outgoing_nodes: Vec<SerializedFlowLink>,
    pub(crate) expression: Expression,
}

#[derive(PartialEq, Debug)]
pub struct SerializedFlowLink {
    pub(crate) node_id: String,
    pub(crate) value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub struct SerializedActionFlowNode {
    pub(crate) id: String,
    pub(crate) outgoing_node: SerializedFlowLink,
    pub(crate) action: Box<dyn Action>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub struct SerializedSleepFlowNode {
    pub(crate) id: String,
    pub(crate) outgoing_node: SerializedFlowLink,
    #[serde(with = "humantime_serde")]
    pub(crate) duration: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::flow_engine::Expression::{EqualTo, Literal};
    use crate::flow_engine::Value;
    use crate::flow_engine::action::LogAction;

    #[tokio::test]
    async fn test_serialized_flow() {
        let json = include_str!("../../tests/resources/flows/logFlow.json");

        let flow = serde_json::from_str::<SerializedFlow>(json).unwrap();
        let expected = SerializedFlow {
            id: "01K7KK6H5R7Y72QJEJSJQCKMRQ".to_string(),
            name: "logFlow".to_string(),
            schedule: None,
            trigger: None,
            nodes: vec![
                SerializedFlowNode::StartNode(SerializedStartFlowNode {
                    id: "startNode".to_string(),
                    outgoing_node: SerializedFlowLink {
                        node_id: "logNode".to_string(),
                        value: Value::None,
                    },
                }),
                SerializedFlowNode::ActionNode(SerializedActionFlowNode {
                    id: "logNode".to_string(),
                    action: Box::new(LogAction::new("Action is triggered".to_string())),
                    outgoing_node: SerializedFlowLink {
                        node_id: "endNode".to_string(),
                        value: Value::None,
                    },
                }),
                SerializedFlowNode::EndNode(SerializedEndFlowNode { id: "endNode".to_string() }),
            ],
        };

        // As ActionFlowNode's action cannot implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn test_serialized_flow_with_trigger() {
        let json = include_str!("../../tests/resources/flows/logFlowWithTrigger.json");

        let flow = serde_json::from_str::<SerializedFlow>(json).unwrap();
        let expected = EqualTo {
            lhs: Box::new(Literal {
                value: Value::Number(Number::PositiveInt(1337)),
            }),
            rhs: Box::new(Literal {
                value: Value::Number(Number::Float(42.0)),
            }),
        };

        assert_eq!(flow.trigger, Some(expected));
        assert_eq!(flow.schedule, None);
    }

    #[tokio::test]
    async fn test_serialized_flow_with_sleep_node() {
        let json = include_str!("../../tests/resources/flows/sleepFlow.json");

        let flow = serde_json::from_str::<SerializedFlow>(json).unwrap();
        let expected = SerializedFlow {
            id: "01K7KK7E6GG26XZZDXSGFZCWQ4".to_string(),
            name: "sleepFlow".to_string(),
            schedule: None,
            trigger: None,
            nodes: vec![
                SerializedFlowNode::StartNode(SerializedStartFlowNode {
                    id: "startNode".to_string(),
                    outgoing_node: SerializedFlowLink {
                        node_id: "sleepNode".to_string(),
                        value: Value::None,
                    },
                }),
                SerializedFlowNode::SleepNode(SerializedSleepFlowNode {
                    id: "sleepNode".to_string(),
                    outgoing_node: SerializedFlowLink {
                        node_id: "endNode".to_string(),
                        value: Value::None,
                    },
                    duration: Duration::from_secs(3907),
                }),
                SerializedFlowNode::EndNode(SerializedEndFlowNode { id: "endNode".to_string() }),
            ],
        };

        // As ActionFlowNode's action cannot implement PartialEq, use debug print for comparison
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
        println!("{:?}", flow);
    }
}
