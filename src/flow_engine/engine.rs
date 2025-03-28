use crate::flow_engine::context::Context;
use crate::flow_engine::flow::{Flow, FlowNode, FlowNodeKind};
use crate::flow_engine::scope::Scope;
use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tokio::time::Instant;
use tracing::{debug, info, instrument, trace};

#[instrument(fields(flow = flow.name()), skip_all)]
pub async fn execute(flow: &Flow, context: &Context) -> Result<FlowExecutionReport, FlowEngineError> {
    info!("▶️ Executing flow...");
    let start = Instant::now();

    let mut scope = Scope::new();
    let mut next_node = Some(flow.start_node());
    while let Some(node) = next_node {
        next_node = execute_node(node, context, &mut scope).await?;
    }

    let duration = Instant::now() - start;
    info!(duration = ?duration, "▶️ Executing flow... OK");

    Ok(FlowExecutionReport { scope: scope.take(), duration })
}

#[instrument(fields(node = node.id()), skip_all)]
async fn execute_node<'a>(node: &'a FlowNode, context: &Context, scope: &mut Scope) -> Result<Option<&'a FlowNode>, FlowEngineError> {
    trace!("{:?}", node);

    let next_flow_link = match node.kind() {
        FlowNodeKind::Action(action_flow_node) => {
            info!("Executing action {}", action_flow_node.action().kind());
            action_flow_node.action().execute(context, scope).await;
            node.outgoing_nodes().first()
        }
        _ => node.outgoing_nodes().first(),
    };

    let next_node = next_flow_link.ok_or_else(|| FlowEngineError::MissingOutgoingNode(node.id().to_owned()))?.node();

    debug!("Next node: {}", next_node.id());
    match next_node.kind() {
        FlowNodeKind::End => Ok(None),
        _ => Ok(Some(next_node)),
    }
}

#[derive(Error, Debug)]
pub enum FlowEngineError {
    #[error("missing outgoing node for node '{0}'")]
    MissingOutgoingNode(String),
}

pub struct FlowExecutionReport {
    scope: HashMap<String, Box<dyn Any + Send + Sync>>,
    duration: Duration,
}

impl FlowExecutionReport {
    pub fn scope(&self) -> &HashMap<String, Box<dyn Any + Send + Sync>> {
        &self.scope
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::action::LogAction;
    use crate::flow_engine::flow::{ActionFlowNode, FlowLink, FlowNodeKind};
    use std::sync::Arc;
    use test_log::test;

    #[test(tokio::test)]
    async fn executes_a_flow_with_one_action_node() {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);

        let action = LogAction::new("Hello".to_string());
        let log_node = FlowNode::new(
            "log_node".to_string(),
            vec![FlowLink::new(Arc::new(end_node), None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(action))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(log_node), None)], FlowNodeKind::Start);
        let flow = Flow::new("flow".to_string(), start_node).unwrap();

        let result = execute(&flow, &Context::default()).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn fails_if_an_outgoing_node_is_missing() {
        let start_node = FlowNode::new("startNode".to_string(), vec![], FlowNodeKind::Start);
        let flow = Flow::new("flow".to_string(), start_node).unwrap();

        let result = execute(&flow, &Context::default()).await;
        assert!(matches!(result, Err(FlowEngineError::MissingOutgoingNode(_))));
    }
}
