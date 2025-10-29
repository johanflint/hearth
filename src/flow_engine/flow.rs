use crate::flow_engine::Expression::Literal;
use crate::flow_engine::action::Action;
use crate::flow_engine::{Expression, Schedule, Value};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

#[cfg_attr(not(test), derive(Debug))]
pub struct Flow {
    id: String,
    name: String,
    schedule: Option<Schedule>,
    trigger: Expression,
    start_node: Arc<FlowNode>,
    nodes_by_id: HashMap<String, Arc<FlowNode>>,
}

impl Flow {
    pub fn new(
        id: String,
        name: String,
        schedule: Option<Schedule>,
        trigger: Option<Expression>,
        start_node: Arc<FlowNode>,
        nodes_by_id: HashMap<String, Arc<FlowNode>>,
    ) -> Result<Self, String> {
        match start_node.kind {
            FlowNodeKind::Start => Ok(Flow {
                id,
                name,
                schedule,
                trigger: trigger.unwrap_or(Literal { value: Value::Boolean(true) }),
                start_node,
                nodes_by_id,
            }),
            _ => Err("start_node must be of type FlowNodeKind::Start".to_string()),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn start_node(&self) -> &FlowNode {
        &self.start_node
    }

    pub fn trigger(&self) -> &Expression {
        &self.trigger
    }

    pub fn schedule(&self) -> Option<Schedule> {
        self.schedule.clone()
    }

    pub fn node_by_id(&self, id: &str) -> Option<&FlowNode> {
        self.nodes_by_id.get(id).map(|node| node.as_ref())
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
    value: Value,
}

impl FlowLink {
    pub fn new(node: Arc<FlowNode>, value: Value) -> Self {
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
    Conditional(Expression),
    Action(ActionFlowNode),
    Sleep(Duration),
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
