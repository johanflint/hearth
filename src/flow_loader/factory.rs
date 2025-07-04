use crate::flow_engine::flow::{ActionFlowNode, Flow, FlowLink, FlowNode, FlowNodeKind};
use crate::flow_loader::serialized_flow::{SerializedFlow, SerializedFlowNode};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use thiserror::Error;

pub fn from_json(json: &str) -> Result<Flow, FlowFactoryError> {
    let flow = serde_json::from_str::<SerializedFlow>(json)?;
    let mut nodes = flow.nodes; // Take ownership of nodes

    let num_start_nodes = nodes.iter().filter(|node| matches!(node, SerializedFlowNode::StartNode(_))).count();
    if num_start_nodes == 0 {
        return Err(FlowFactoryError::MissingStartNode);
    }
    if num_start_nodes > 1 {
        return Err(FlowFactoryError::TooManyStartNodes(num_start_nodes));
    }

    let end_nodes: Vec<SerializedFlowNode> = nodes._extract_if(|node| matches!(node, SerializedFlowNode::EndNode(_)));
    if end_nodes.len() == 0 {
        return Err(FlowFactoryError::MissingEndNode);
    }

    let mut nodes_to_visit: VecDeque<SerializedFlowNode> = VecDeque::from(end_nodes);
    nodes_to_visit.reserve(nodes.len()); // Reserve capacity to avoid multiple reallocations

    let mut flow_node_map: HashMap<String, Arc<FlowNode>> = HashMap::new();
    let mut start_node: Option<FlowNode> = None;

    while let Some(serialized_node) = nodes_to_visit.pop_back() {
        let incoming_nodes: Vec<SerializedFlowNode> = nodes._extract_if(|node| match node {
            SerializedFlowNode::StartNode(node) => node.outgoing_node == serialized_node.id(),
            SerializedFlowNode::EndNode(_) => false,
            SerializedFlowNode::ActionNode(node) => node.outgoing_node == serialized_node.id(),
        });

        if !matches!(serialized_node, SerializedFlowNode::StartNode(_)) && incoming_nodes.is_empty() {
            return Err(FlowFactoryError::NoConnectingNode {
                node: serialized_node.id().to_owned(),
                flow: flow.name,
            });
        }

        // Push elements to the front in reverse order to maintain original order
        for incoming_node in incoming_nodes.into_iter().rev() {
            nodes_to_visit.push_front(incoming_node);
        }

        let outgoing_nodes = map_outgoing_nodes(&serialized_node, &flow_node_map)?;
        let node = to_flow_node(serialized_node, outgoing_nodes);

        if matches!(node.kind(), FlowNodeKind::Start) {
            // Take ownership of the node as the start node is the last node to traverse
            start_node = Some(node);
        } else {
            flow_node_map.insert(node.id().to_owned(), Arc::new(node));
        }
    }

    if !nodes.is_empty() {
        return Err(FlowFactoryError::UnusedNodes {
            nodes: nodes.into_iter().map(|n| n.id().to_owned()).collect(),
        });
    }

    let flow = Flow::new(flow.name, flow.trigger, start_node.ok_or_else(|| FlowFactoryError::MissingStartNode)?).expect("Flow creation failed");
    Ok(flow)
}

fn map_outgoing_nodes(serialized_node: &SerializedFlowNode, flow_node_map: &HashMap<String, Arc<FlowNode>>) -> Result<Vec<FlowLink>, FlowFactoryError> {
    if let Some(outgoing_node_id) = serialized_node.outgoing_nodes() {
        let node = flow_node_map.get(outgoing_node_id).ok_or_else(|| FlowFactoryError::MissingNode {
            node_id: serialized_node.id().to_owned(),
            outgoing_node_id: outgoing_node_id.to_owned(),
        })?;

        return Ok(vec![FlowLink::new(node.clone(), None)]);
    }

    Ok(Vec::new())
}

// Must own serialized_node so the contents can be moved to avoid copying data
fn to_flow_node(serialized_node: SerializedFlowNode, outgoing_nodes: Vec<FlowLink>) -> FlowNode {
    match serialized_node {
        SerializedFlowNode::StartNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Start),
        SerializedFlowNode::EndNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::End),
        SerializedFlowNode::ActionNode(node) => FlowNode::new(node.id, outgoing_nodes, FlowNodeKind::Action(ActionFlowNode::new(node.action))),
    }
}

// This can be removed once Rust 1.87.0 comes out and extract_if is stabilized (https://github.com/rust-lang/rust/pull/137109)
trait ExtractIf<T> {
    fn _extract_if<F>(&mut self, predicate: F) -> Vec<T>
    where
        F: FnMut(&T) -> bool;
}

impl<T> ExtractIf<T> for Vec<T> {
    fn _extract_if<F>(&mut self, mut predicate: F) -> Vec<T>
    where
        F: FnMut(&T) -> bool,
    {
        let mut extracted = Vec::new();
        let mut i = 0;
        while i < self.len() {
            if predicate(&self[i]) {
                extracted.push(self.remove(i));
            } else {
                i += 1
            }
        }
        extracted
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
    #[error("unused nodes: {}", nodes.join(", "))]
    UnusedNodes { nodes: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::action::{ControlDeviceAction, LogAction};
    use crate::flow_engine::property_value::PropertyValue::SetBooleanValue;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn returns_an_error_if_an_unknown_node_type_is_found() {
        let json = include_str!("../../tests/resources/flows/invalid/unknownNodeTypeFlow.json");
        let result = from_json(json);
        assert!(matches!(result, Err(FlowFactoryError::Deserialization(_))));
    }

    #[tokio::test]
    async fn returns_an_error_if_no_start_node_is_found() {
        let json = r#"{ "name": "flow", "nodes": [] }"#;
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
    async fn creates_a_flow_with_a_start_and_end_node() {
        let json = include_str!("../../tests/resources/flows/emptyFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);
        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(end_node), None)], FlowNodeKind::Start);

        let expected = Flow::new("emptyFlow".to_string(), None, start_node).unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_an_action_node_of_type_log() {
        let json = include_str!("../../tests/resources/flows/logFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);

        let action_node = FlowNode::new(
            "logNode".to_string(),
            vec![FlowLink::new(Arc::new(end_node), None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(LogAction::new("Action is triggered".to_string())))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(action_node), None)], FlowNodeKind::Start);

        let expected = Flow::new("logFlow".to_string(), None, start_node).unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }

    #[tokio::test]
    async fn creates_a_flow_with_an_action_node_of_type_control_device() {
        let json = include_str!("../../tests/resources/flows/controlDeviceFlow.json");
        let flow = from_json(json).unwrap();

        let end_node = FlowNode::new("endNode".to_string(), vec![], FlowNodeKind::End);

        let action_node = FlowNode::new(
            "controlNode".to_string(),
            vec![FlowLink::new(Arc::new(end_node), None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(ControlDeviceAction::new(
                "42".to_string(),
                HashMap::from([("fan".to_string(), SetBooleanValue(true))]),
            )))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(action_node), None)], FlowNodeKind::Start);

        let expected = Flow::new("controlDeviceFlow".to_string(), None, start_node).unwrap();
        assert_eq!(format!("{:#?}", flow), format!("{:#?}", expected));
    }
}
