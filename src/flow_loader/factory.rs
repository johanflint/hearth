use crate::flow_engine::Value;
use crate::flow_engine::flow::{ActionFlowNode, Flow, FlowLink, FlowNode, FlowNodeKind};
use crate::flow_loader::serialized_flow::{SerializedFlow, SerializedFlowNode};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use thiserror::Error;

pub fn from_json(json: &str) -> Result<Flow, FlowFactoryError> {
    let flow = serde_json::from_str::<SerializedFlow>(json)?;

    let num_start_nodes = flow.nodes.iter().filter(|node| matches!(node, SerializedFlowNode::StartNode(_))).count();
    if num_start_nodes == 0 {
        return Err(FlowFactoryError::MissingStartNode);
    }
    if num_start_nodes > 1 {
        return Err(FlowFactoryError::TooManyStartNodes(num_start_nodes));
    }

    let end_nodes: Vec<String> = flow
        .nodes
        .iter()
        .filter_map(|node| match node {
            SerializedFlowNode::EndNode(end_node) => Some(end_node.id.to_owned()),
            _ => None,
        })
        .collect();
    if end_nodes.len() == 0 {
        return Err(FlowFactoryError::MissingEndNode);
    }

    let mut nodes_map: HashMap<String, SerializedFlowNode> = flow.nodes.into_iter().map(|node| (node.id().to_owned(), node)).collect(); // Take ownership of the nodes
    let mut remaining_children: HashMap<String, usize> = HashMap::with_capacity(nodes_map.len());
    let mut child_to_parent: HashMap<String, String> = HashMap::new();

    for (id, node) in nodes_map.iter() {
        let out_count = node.outgoing_nodes().len();
        remaining_children.insert(id.clone(), out_count);

        let mut value_to_nodes: HashMap<&Value, Vec<String>> = HashMap::new();
        for link in node.outgoing_nodes().iter() {
            value_to_nodes.entry(&link.value).or_default().push(link.node_id.clone());

            // Parent linkage check
            if let Some(prev) = child_to_parent.insert(link.node_id.clone(), id.clone()) {
                // `prev` is the previously registered parent id
                return Err(FlowFactoryError::TooManyParentNodes {
                    node_id: link.node_id.clone(),
                    parent_nodes: vec![prev, id.clone()],
                });
            }
        }

        let duplicates: Vec<String> = value_to_nodes.values().filter(|node_ids| node_ids.len() > 1).flatten().cloned().collect();
        if !duplicates.is_empty() {
            return Err(FlowFactoryError::DuplicateLinkValues { node_id: id.clone(), duplicates });
        }
    }

    let mut nodes_to_visit: VecDeque<String> = VecDeque::from(end_nodes);
    let mut flow_node_map: HashMap<String, Arc<FlowNode>> = HashMap::with_capacity(nodes_map.len());
    let mut start_node: Option<Arc<FlowNode>> = None;

    while let Some(node_id) = nodes_to_visit.pop_back() {
        // Take the node out of the map (ownership)
        let serialized_node = match nodes_map.remove(&node_id) {
            Some(node) => node,
            None => continue,
        };

        // Unless it's a start node, ensure it has a parent
        if !matches!(serialized_node, SerializedFlowNode::StartNode(_)) {
            let has_parent = child_to_parent.get(serialized_node.id()).is_some();
            if !has_parent {
                return Err(FlowFactoryError::NoConnectingNode {
                    node: serialized_node.id().to_owned(),
                    flow: flow.name,
                });
            }
        }

        let outgoing_nodes = map_outgoing_nodes(&serialized_node, &flow_node_map)?;
        let node = to_flow_node(serialized_node, outgoing_nodes);

        let node_id = node.id().to_owned();
        let node_arc = Arc::new(node);

        if matches!(node_arc.kind(), FlowNodeKind::Start) {
            start_node = Some(node_arc.clone());
        }

        flow_node_map.insert(node_id.clone(), node_arc);

        // Ensure all children are handled, so outgoing nodes will be correct
        if let Some(parent_id) = child_to_parent.get(&node_id) {
            if let Some(count) = remaining_children.get_mut(parent_id) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    nodes_to_visit.push_back(parent_id.clone());
                }
            }
        }
    }

    if !nodes_map.is_empty() {
        return Err(FlowFactoryError::UnusedNodes {
            nodes: nodes_map.into_keys().collect(),
        });
    }

    let flow = Flow::new(
        flow.id,
        flow.name,
        flow.schedule,
        flow.trigger,
        start_node.ok_or_else(|| FlowFactoryError::MissingStartNode)?,
        flow_node_map,
    )
    .expect("Flow creation failed");
    Ok(flow)
}

fn map_outgoing_nodes(serialized_node: &SerializedFlowNode, flow_node_map: &HashMap<String, Arc<FlowNode>>) -> Result<Vec<FlowLink>, FlowFactoryError> {
    serialized_node
        .outgoing_nodes()
        .iter()
        .map(|flow_link| {
            let node = flow_node_map.get(&flow_link.node_id).ok_or_else(|| FlowFactoryError::MissingNode {
                node_id: serialized_node.id().to_owned(),
                outgoing_node_id: flow_link.node_id.clone(),
            })?;

            Ok(FlowLink::new(Arc::clone(node), flow_link.value.clone()))
        })
        .collect()
}

// Must own serialized_node so the contents can be moved to avoid copying data
fn to_flow_node(serialized_node: SerializedFlowNode, outgoing_nodes: Vec<FlowLink>) -> FlowNode {
    match serialized_node {
        SerializedFlowNode::StartNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Start),
        SerializedFlowNode::EndNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::End),
        SerializedFlowNode::ConditionalNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Conditional(node.expression)),
        SerializedFlowNode::ActionNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Action(ActionFlowNode::new(node.action))),
        SerializedFlowNode::SleepNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Sleep(node.duration)),
    }
}

#[derive(Error, Debug)]
pub enum FlowFactoryError {
    #[error("json deserialization error: {0}")]
    Deserialization(#[from] serde_json::Error),
    #[error("missing start node")]
    MissingStartNode,
    #[error("only one start node is allowed, found {0}")]
    TooManyStartNodes(usize),
    #[error("missing end node")]
    MissingEndNode,
    #[error("no links found to node '{node}' in flow '{flow}'")]
    NoConnectingNode { node: String, flow: String },
    #[error("node '{node_id}' has a missing outgoing node to '{outgoing_node_id}'")]
    MissingNode { node_id: String, outgoing_node_id: String },
    #[error("node '{node_id}' too many parent nodes: {}", parent_nodes.join(", "))]
    TooManyParentNodes { node_id: String, parent_nodes: Vec<String> },
    #[error("unused nodes: {}", nodes.join(", "))]
    UnusedNodes { nodes: Vec<String> },
    #[error("duplicate outgoing link values for node '{node_id}', pointing to {}", duplicates.join(", "))]
    DuplicateLinkValues { node_id: String, duplicates: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Number;
    use crate::flow_engine::Expression::Literal;
    use crate::flow_engine::Value;
    use crate::flow_engine::action::{ControlDeviceAction, LogAction};
    use crate::flow_engine::property_value::PropertyValue::SetBooleanValue;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[tokio::test]
    async fn returns_an_error_if_an_unknown_node_type_is_found() {
        let json = include_str!("../../tests/resources/flows/invalid/unknownNodeTypeFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::Deserialization(_))));
    }

    #[tokio::test]
    async fn returns_an_error_if_no_start_node_is_found() {
        let json = r#"{ "id": "id", "name": "flow", "nodes": [] }"#;
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::MissingStartNode)));
    }

    #[tokio::test]
    async fn returns_an_error_if_multiple_start_nodes_are_found() {
        let json = include_str!("../../tests/resources/flows/invalid/multipleStartNodesFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::TooManyStartNodes(2))));
    }

    #[tokio::test]
    async fn returns_an_error_if_no_end_node_is_found() {
        let json = include_str!("../../tests/resources/flows/invalid/missingEndNodeFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::MissingEndNode)));
    }

    #[tokio::test]
    async fn returns_an_error_if_a_node_is_not_connected() {
        let json = include_str!("../../tests/resources/flows/invalid/unconnectedNodeFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::NoConnectingNode { .. })));
    }

    #[tokio::test]
    async fn returns_an_error_if_not_all_nodes_are_connected() {
        let json = include_str!("../../tests/resources/flows/invalid/unusedNodesFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::UnusedNodes { .. })));
    }

    #[tokio::test]
    async fn returns_an_error_if_a_node_has_multiple_parent_nodes() {
        let json = include_str!("../../tests/resources/flows/invalid/multipleParentNodesFlow.json");
        let result = from_json(json);
        match result {
            Err(FlowFactoryError::TooManyParentNodes { node_id, parent_nodes }) => {
                assert_eq!(node_id, "endNode");
                assert_eq!(parent_nodes, vec!["conditionalNode", "conditionalNode"]);
            }
            other => panic!("expected FlowFactoryError::TooManyParentNodes, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn returns_an_error_if_a_node_has_duplicate_link_values() {
        assert_eq!(Value::Boolean(true), Value::Boolean(true));
        let json = include_str!("../../tests/resources/flows/invalid/duplicateLinkValuesFlow.json");
        let result = from_json(json);
        match result {
            Err(FlowFactoryError::DuplicateLinkValues { node_id, duplicates }) => {
                assert_eq!(node_id, "conditionalNode");
                println!("{:?}", duplicates);
                assert_eq!(duplicates, vec!["endNodeTrue", "endNodeFalse", "endNodeFalseAgain"]);
            }
            other => panic!("expected FlowFactoryError::DuplicateLinkValues, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn creates_a_flow_with_a_start_and_end_node() {
        let json = include_str!("../../tests/resources/flows/emptyFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(end_node), Value::None)], FlowNodeKind::Start);

        let expected = Flow::new(
            "01K7KK65D87SZGGZE7VB8QYT20".to_string(),
            "emptyFlow".to_string(),
            None,
            None,
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_an_action_node_of_type_log() {
        let json = include_str!("../../tests/resources/flows/logFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);

        let action_node = FlowNode::new(
            "logNode".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(LogAction::new("Action is triggered".to_string())))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(action_node), Value::None)], FlowNodeKind::Start);

        let expected = Flow::new(
            "01K7KK6H5R7Y72QJEJSJQCKMRQ".to_string(),
            "logFlow".to_string(),
            None,
            None,
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_an_action_node_of_type_control_device() {
        let json = include_str!("../../tests/resources/flows/controlDeviceFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);

        let action_node = FlowNode::new(
            "controlNode".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(ControlDeviceAction::new(
                "42".to_string(),
                HashMap::from([("fan".to_string(), SetBooleanValue(true))]),
            )))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(action_node), Value::None)], FlowNodeKind::Start);

        let expected = Flow::new(
            "01K7KK5FC54SN8D4QYVNEGFYG4".to_string(),
            "controlDeviceFlow".to_string(),
            None,
            None,
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_a_conditional_node() {
        let json = include_str!("../../tests/resources/flows/conditionalFlow.json");
        let flow = from_json(json).unwrap();

        let end_node_true = FlowNode::new("endNodeTrue".to_string(), vec![], FlowNodeKind::End);
        let end_node_false = FlowNode::new("endNodeFalse".to_string(), vec![], FlowNodeKind::End);

        let conditional_node = FlowNode::new(
            "conditionalNode".to_string(),
            vec![
                FlowLink::new(Arc::new(end_node_true), Value::Boolean(true)),
                FlowLink::new(Arc::new(end_node_false), Value::Boolean(false)),
            ],
            FlowNodeKind::Conditional(Literal {
                value: Value::Number(Number::Float(42.0)),
            }),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(conditional_node), Value::None)], FlowNodeKind::Start);

        let expected = Flow::new(
            "01K8JSTTCC831M6TERRH41D595".to_string(),
            "conditionalFlow".to_string(),
            None,
            None,
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_a_sleep_node() {
        let json = include_str!("../../tests/resources/flows/sleepFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);

        let sleep_node = FlowNode::new(
            "sleepNode".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Sleep(Duration::from_secs(3907)),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(sleep_node), Value::None)], FlowNodeKind::Start);

        let expected = Flow::new(
            "01K7KK7E6GG26XZZDXSGFZCWQ4".to_string(),
            "sleepFlow".to_string(),
            None,
            None,
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }
}

#[cfg(test)]
impl std::fmt::Debug for Flow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Ignores the nodes_by_id field as the order is not deterministic and all nodes are reachable by start node
        f.debug_struct("Flow")
            .field("id", &self.id())
            .field("name", &self.name())
            .field("schedule", &self.schedule())
            .field("trigger", &self.trigger())
            .field("start_node", &self.start_node())
            .finish()
    }
}
