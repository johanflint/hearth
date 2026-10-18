use crate::domain::Value;
use crate::flow_engine::Expression::Literal;
use crate::flow_engine::action::Action;
use crate::flow_engine::{Expression, Schedule};
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

    /// Iterates over all nodes depth-first, starting at the start node.
    pub fn walk(&self) -> impl Iterator<Item = &FlowNode> + '_ {
        self.start_node.walk()
    }

    pub fn walk_expressions(&self) -> impl Iterator<Item = &Expression> + '_ {
        let nodes = self.walk().flat_map(|node| node.expressions());
        std::iter::once(self.trigger()).chain(nodes).flat_map(Expression::walk)
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

    /// Iterates over this node and its descendants depth-first, following outgoing links in order.
    pub fn walk(&self) -> impl Iterator<Item = &FlowNode> + '_ {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            stack.extend(node.outgoing_nodes.iter().rev().map(FlowLink::node));
            Some(node)
        })
    }

    pub fn expressions(&self) -> Vec<&Expression> {
        match self.kind() {
            FlowNodeKind::Start | FlowNodeKind::End | FlowNodeKind::Sleep(_) => vec![],
            FlowNodeKind::Conditional(expression) => vec![expression],
            FlowNodeKind::Action(action) => action.action().expressions(),
        }
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

    pub fn value(&self) -> &Value {
        &self.value
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::action::ControlDeviceAction;
    use crate::flow_engine::property_command::PropertyCommand;

    fn node(id: &str, children: Vec<FlowNode>) -> FlowNode {
        let links = children.into_iter().map(|child| FlowLink::new(Arc::new(child), Value::None)).collect();
        FlowNode::new(id.to_string(), links, FlowNodeKind::End)
    }

    #[test]
    fn walk_visits_all_nodes_depth_first_in_link_order() {
        let tree = node("start", vec![node("a", vec![node("c", vec![])]), node("b", vec![])]);

        let ids: Vec<&str> = tree.walk().map(FlowNode::id).collect();

        assert_eq!(ids, ["start", "a", "c", "b"]);
    }

    fn control_device(preset_ids: &[&str]) -> ActionFlowNode {
        let property = preset_ids
            .iter()
            .map(|id| (id.to_string(), PropertyCommand { value: preset(id), transition: None }))
            .collect();
        ActionFlowNode::new(Box::new(ControlDeviceAction::new("device".to_string(), None, property)))
    }

    fn preset(id: &str) -> Expression {
        Expression::PresetValue { id: id.to_string() }
    }

    fn link(node: FlowNode) -> FlowLink {
        FlowLink::new(Arc::new(node), Value::None)
    }

    fn flow(trigger: Option<Expression>, start_node: FlowNode) -> Flow {
        Flow::new("flow".to_string(), "flow".to_string(), None, trigger, Arc::new(start_node), HashMap::new()).unwrap()
    }

    fn preset_ids<'a>(expressions: impl Iterator<Item = &'a Expression>) -> Vec<&'a str> {
        expressions
            .filter_map(|e| match e {
                Expression::PresetValue { id } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn flow_node_expressions_is_empty_for_nodes_without_expressions() {
        for kind in [FlowNodeKind::Start, FlowNodeKind::End, FlowNodeKind::Sleep(Duration::from_secs(1))] {
            let node = FlowNode::new("node".to_string(), vec![], kind);
            assert!(node.expressions().is_empty(), "got {:?}", node.expressions());
        }
    }

    #[test]
    fn flow_node_expressions_returns_the_condition_of_a_conditional() {
        let node = FlowNode::new("node".to_string(), vec![], FlowNodeKind::Conditional(preset("condition")));

        assert_eq!(preset_ids(node.expressions().into_iter()), ["condition"]);
    }

    #[test]
    fn flow_node_expressions_returns_the_expressions_of_an_action() {
        let action = control_device(&["a"]);
        let node = FlowNode::new("node".to_string(), vec![], FlowNodeKind::Action(action));

        assert_eq!(preset_ids(node.expressions().into_iter()), ["a"]);
    }

    #[test]
    fn walk_expressions_visits_the_trigger_conditionals_and_actions_and_their_sub_expressions() {
        let nested = Expression::Not { expression: Box::new(preset("nested")) };
        let action = control_device(&["action"]);
        let action_node = FlowNode::new("action".to_string(), vec![], FlowNodeKind::Action(action));
        let conditional =
            FlowNode::new("conditional".to_string(), vec![link(action_node)], FlowNodeKind::Conditional(nested));
        let start = FlowNode::new("start".to_string(), vec![link(conditional)], FlowNodeKind::Start);
        let flow = flow(Some(preset("trigger")), start);

        assert_eq!(preset_ids(flow.walk_expressions()), ["trigger", "nested", "action"]);
    }

    #[test]
    fn walk_expressions_only_visits_the_default_trigger_for_a_flow_without_expressions() {
        let start = FlowNode::new("start".to_string(), vec![], FlowNodeKind::Start);
        let flow = flow(None, start);

        assert!(preset_ids(flow.walk_expressions()).is_empty());
        assert_eq!(flow.walk_expressions().count(), 1);
    }
}
