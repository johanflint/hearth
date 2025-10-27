use crate::flow_engine::context::Context;
use crate::flow_engine::expression::{ExpressionError, evaluate};
use crate::flow_engine::flow::{Flow, FlowNode, FlowNodeKind};
use crate::flow_engine::scope::Scope;
use crate::flow_engine::{SchedulerCommand, Value};
use ExecuteNodeResult::*;
use std::any::Any;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::mpsc::error::SendError;
use tokio::time::Instant;
use tracing::{debug, info, instrument, trace, warn};

#[instrument(fields(flow = flow.name()), skip_all)]
pub async fn execute(flow: &Flow, node_id: Option<String>, context: &Context, tx: Sender<SchedulerCommand>) -> Result<FlowExecutionReport, FlowEngineError> {
    debug!("⚖️ Evaluating trigger condition for flow...");
    let result = evaluate(flow.trigger(), context);
    match result {
        Ok(Value::Boolean(true)) => debug!("⚖️ Evaluating trigger condition for flow... true"),
        Ok(result) => {
            debug!(result = ?result, "⚖️ Evaluating trigger condition for flow... false, skipping execution");
            return Ok(FlowExecutionReport::empty());
        }
        Err(error) => {
            warn!("⚖️ Evaluating trigger condition for flow... failed, {}", error);
            return Err(FlowEngineError::FailedTriggerEvaluation(error));
        }
    }

    info!("▶️ Executing flow...");
    let start = Instant::now();

    let mut scope = Scope::new();

    let start_node = if let Some(node_id) = node_id {
        let node = flow.node_by_id(&node_id).ok_or_else(|| FlowEngineError::MissingProvidedStartNode(node_id))?;
        // Passed node_id may point to an end node if it follows a sleep node
        if matches!(node.kind(), FlowNodeKind::End) { None } else { Some(node) }
    } else {
        Some(flow.start_node())
    };

    let mut next_node = start_node;
    while let Some(node) = next_node {
        next_node = match execute_node(node, context, &mut scope).await? {
            Next(node) => Some(node),
            End => None,
            Sleep { duration, next } => {
                tx.send(SchedulerCommand::ScheduleOnce {
                    flow_id: flow.id().to_string(),
                    node_id: next.id().to_string(),
                    delay: duration,
                })
                .await?;
                None
            }
        }
    }

    let duration = Instant::now() - start;
    info!(duration = ?duration, "▶️ Executing flow... OK");

    Ok(FlowExecutionReport { scope: scope.take(), duration })
}

#[instrument(fields(node = node.id()), skip_all)]
async fn execute_node<'a>(node: &'a FlowNode, context: &Context, scope: &mut Scope) -> Result<ExecuteNodeResult<'a>, FlowEngineError> {
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

    if let FlowNodeKind::Sleep(duration) = node.kind() {
        return Ok(Sleep {
            duration: *duration,
            next: next_node,
        });
    }

    match next_node.kind() {
        FlowNodeKind::End => Ok(End),
        _ => Ok(Next(next_node)),
    }
}

enum ExecuteNodeResult<'a> {
    Next(&'a FlowNode),
    End,
    Sleep { duration: Duration, next: &'a FlowNode },
}

#[derive(Error, Debug)]
pub enum FlowEngineError {
    #[error("missing outgoing node for node '{0}'")]
    MissingOutgoingNode(String),
    #[error("evaluation of the flow trigger failed: {0}")]
    FailedTriggerEvaluation(ExpressionError),
    #[error(transparent)]
    FailedScheduleSleepCommand(#[from] SendError<SchedulerCommand>),
    #[error("missing provided start node '{0}'")]
    MissingProvidedStartNode(String),
}

pub struct FlowExecutionReport {
    scope: HashMap<String, Box<dyn Any + Send + Sync>>,
    duration: Duration,
}

impl FlowExecutionReport {
    #[cfg(test)]
    pub fn new(scope: HashMap<String, Box<dyn Any + Send + Sync>>, duration: Duration) -> Self {
        FlowExecutionReport { scope, duration }
    }

    pub fn empty() -> Self {
        FlowExecutionReport {
            scope: HashMap::with_capacity(0),
            duration: Duration::ZERO,
        }
    }

    pub fn scope(&self) -> &HashMap<String, Box<dyn Any + Send + Sync>> {
        &self.scope
    }

    pub fn take_from_scope<T: 'static + Send + Sync>(mut self, k: &str) -> Option<T> {
        self.scope.remove(k).and_then(|v| v.downcast::<T>().ok().map(|boxed| *boxed))
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Expression;
    use crate::flow_engine::action::LogAction;
    use crate::flow_engine::flow::{ActionFlowNode, FlowLink, FlowNodeKind};
    use std::sync::Arc;
    use test_log::test;
    use tokio::sync::mpsc;

    #[test(tokio::test)]
    async fn executes_a_flow_with_one_action_node() {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);

        let action = LogAction::new("Hello".to_string());
        let log_node = FlowNode::new(
            "log_node".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Action(ActionFlowNode::new(Box::new(action))),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(log_node), Value::None)], FlowNodeKind::Start);
        let flow = Flow::new("id".to_string(), "flow".to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap();

        let (scheduler_tx, _scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
        let result = execute(&flow, None, &Context::default(), scheduler_tx).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn skips_execution_if_the_trigger_returns_false() {
        let start_node = FlowNode::new("startNode".to_string(), vec![], FlowNodeKind::Start);
        let flow = Flow::new(
            "id".to_string(),
            "flow".to_string(),
            None,
            Some(Expression::Literal { value: Value::Boolean(false) }),
            Arc::new(start_node),
            HashMap::new(),
        )
        .unwrap();

        let (scheduler_tx, _scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
        let result = execute(&flow, None, &Context::default(), scheduler_tx).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.scope.is_empty());
        assert_eq!(result.duration, Duration::ZERO);
    }

    #[test(tokio::test)]
    async fn fails_if_the_start_node_id_cannot_be_found() {
        let start_node = FlowNode::new("startNode".to_string(), vec![], FlowNodeKind::Start);
        let flow = Flow::new("id".to_string(), "flow".to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap();

        let (scheduler_tx, _scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
        let result = execute(&flow, Some("unknown".to_string()), &Context::default(), scheduler_tx).await;
        assert!(matches!(result, Err(FlowEngineError::MissingProvidedStartNode(_))));
    }

    #[test(tokio::test)]
    async fn fails_if_an_outgoing_node_is_missing() {
        let start_node = FlowNode::new("startNode".to_string(), vec![], FlowNodeKind::Start);
        let flow = Flow::new("id".to_string(), "flow".to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap();

        let (scheduler_tx, _scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
        let result = execute(&flow, None, &Context::default(), scheduler_tx).await;
        assert!(matches!(result, Err(FlowEngineError::MissingOutgoingNode(_))));
    }

    #[test(tokio::test)]
    async fn sends_a_schedule_once_command_for_a_sleep_node() {
        let end_node = FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End);

        let sleep_node = FlowNode::new(
            "sleep_node".to_string(),
            vec![FlowLink::new(Arc::new(end_node), Value::None)],
            FlowNodeKind::Sleep(Duration::from_secs(42)),
        );

        let start_node = FlowNode::new("startNode".to_string(), vec![FlowLink::new(Arc::new(sleep_node), Value::None)], FlowNodeKind::Start);
        let flow = Flow::new("id".to_string(), "flow".to_string(), None, None, Arc::new(start_node), HashMap::new()).unwrap();

        let (scheduler_tx, mut scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);

        execute(&flow, None, &Context::default(), scheduler_tx).await.unwrap();
        let received_command = scheduler_rx.recv().await;
        if let Some(SchedulerCommand::ScheduleOnce { flow_id, node_id, delay }) = received_command {
            assert_eq!(flow_id, "id");
            assert_eq!(node_id, "end_node");
            assert_eq!(delay, Duration::from_secs(42));
        } else {
            panic!("Expected ScheduleOnce command");
        }
    }

    #[test(tokio::test)]
    async fn resumes_execution_at_the_specified_node_id() {
        let end_node = Arc::new(FlowNode::new("end_node".to_string(), vec![], FlowNodeKind::End));

        let sleep_node = Arc::new(FlowNode::new(
            "sleep_node".to_string(),
            vec![FlowLink::new(end_node.clone(), Value::None)],
            FlowNodeKind::Sleep(Duration::from_secs(42)),
        ));

        let start_node = Arc::new(FlowNode::new(
            "startNode".to_string(),
            vec![FlowLink::new(sleep_node.clone(), Value::None)],
            FlowNodeKind::Start,
        ));
        let nodes_by_id = HashMap::from([
            (start_node.id().to_string(), start_node.clone()),
            (sleep_node.id().to_string(), sleep_node.clone()),
            (end_node.id().to_string(), end_node.clone()),
        ]);
        let flow = Flow::new("id".to_string(), "flow".to_string(), None, None, start_node, nodes_by_id).unwrap();

        let (scheduler_tx, mut scheduler_rx) = mpsc::channel::<SchedulerCommand>(32);
        let result = execute(&flow, Some("end_node".to_string()), &Context::default(), scheduler_tx).await.unwrap();

        // Ensure that nothing was scheduled
        assert!(scheduler_rx.try_recv().is_err(), "Expected no scheduler commands to be sent");
        assert!(result.scope.is_empty());
    }
}
