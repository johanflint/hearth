use crate::flow_engine::Expression::Literal;
use crate::flow_engine::action::Action;
use crate::flow_engine::{Expression, Value};
use std::fmt::Debug;
use std::sync::Arc;

#[derive(Debug)]
pub struct Flow {
    name: String,
    trigger: Expression,
    start_node: FlowNode,
}

impl Flow {
    pub fn new(name: String, trigger: Option<Expression>, start_node: FlowNode) -> Result<Self, String> {
        match start_node.kind {
            FlowNodeKind::Start => Ok(Flow {
                name,
                trigger: trigger.unwrap_or(Literal { value: Value::Boolean(true) }),
                start_node,
            }),
            _ => Err("start_node must be of type FlowNodeKind::Start".to_string()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn start_node(&self) -> &FlowNode {
        &self.start_node
    }
}

#[derive(Debug)]
pub struct FlowNode {
    id: String,
    outgoing_nodes: Vec<FlowLink>,
    kind: FlowNodeKind,
}

impl FlowNode {
    pub fn new(id: String, outgoing_nodes: Vec<FlowLink>, kind: FlowNodeKind) -> Self {
        FlowNode { id, outgoing_nodes, kind }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn outgoing_nodes(&self) -> &[FlowLink] {
        &self.outgoing_nodes
    }

    pub fn kind(&self) -> &FlowNodeKind {
        &self.kind
    }
}

#[derive(Debug)]
pub struct FlowLink {
    node: Arc<FlowNode>,
    #[allow(dead_code)]
    value: Option<String>,
}

impl FlowLink {
    pub fn new(node: Arc<FlowNode>, value: Option<String>) -> Self {
        FlowLink { node, value }
    }

    pub fn node(&self) -> &FlowNode {
        &self.node
    }
}

#[derive(Debug)]
pub enum FlowNodeKind {
    Start,
    End,
    Action(ActionFlowNode),
}

#[derive(Debug)]
pub struct ActionFlowNode {
    action: Box<dyn Action>,
}

impl ActionFlowNode {
    pub fn new(action: Box<dyn Action>) -> Self {
        ActionFlowNode { action }
    }

    pub fn action(&self) -> &dyn Action {
        self.action.as_ref()
    }
}
