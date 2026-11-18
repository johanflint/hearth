use crate::domain::{BatteryState, Connectivity};
use crate::flow_engine::FlowEngineError;
use crate::integration::IntegrationStatus;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Clone, PartialEq)]
pub struct Attention {
    pub subject: Subject,
    pub issue: Issue,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    Device(String),
    Flow(String),
    Integration(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Issue {
    Connectivity(Connectivity),
    Battery(BatteryState),
    FlowFailed { error: String },
    Integration(IntegrationStatus),
}

impl Issue {
    pub fn is_same_kind(&self, other: &Issue) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttentionList {
    pub devices: Arc<[Attention]>,
    pub flows: Arc<[Attention]>,
    pub integrations: Arc<[Attention]>,
}

impl AttentionList {
    pub fn iter(&self) -> impl Iterator<Item = &Attention> {
        self.devices.iter().chain(self.flows.iter()).chain(self.integrations.iter())
    }
}

pub enum FlowRun {
    Succeeded { flow_id: String },
    Failed { flow_id: String, error: FlowEngineError },
}

#[derive(Clone, Debug)]
pub struct FlowRunReporter {
    tx: mpsc::Sender<FlowRun>,
}

impl FlowRunReporter {
    pub fn new(tx: mpsc::Sender<FlowRun>) -> Self {
        FlowRunReporter { tx }
    }

    pub fn succeeded(&self, flow_id: &str) {
        self.send(flow_id, FlowRun::Succeeded { flow_id: flow_id.to_string() });
    }

    pub fn failed(&self, flow_id: &str, error: FlowEngineError) {
        self.send(flow_id, FlowRun::Failed { flow_id: flow_id.to_string(), error });
    }

    fn send(&self, flow_id: &str, flow_run: FlowRun) {
        // Never block or fail a flow because of a full channel
        if self.tx.try_send(flow_run).is_err() {
            warn!("⚠️ Dropped the result of flow '{flow_id}'");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reporter(capacity: usize) -> (FlowRunReporter, mpsc::Receiver<FlowRun>) {
        let (tx, rx) = mpsc::channel(capacity);
        (FlowRunReporter::new(tx), rx)
    }

    #[test]
    fn succeeded_sends_a_success() {
        let (reporter, mut rx) = reporter(1);

        reporter.succeeded("a");

        assert!(matches!(rx.try_recv().unwrap(), FlowRun::Succeeded { flow_id } if flow_id == "a"));
    }

    #[test]
    fn failed_sends_the_error() {
        let (reporter, mut rx) = reporter(1);

        reporter.failed("a", FlowEngineError::MissingOutgoingNode("n".into()));

        assert!(matches!(
            rx.try_recv().unwrap(),
            FlowRun::Failed { flow_id, error: FlowEngineError::MissingOutgoingNode(node) }
                if flow_id == "a" && node == "n"
        ));
    }

    #[test]
    fn reporting_does_not_block_or_fail_when_the_channel_is_full() {
        let (reporter, mut rx) = reporter(1);
        reporter.succeeded("a");

        reporter.succeeded("b");

        assert!(matches!(rx.try_recv().unwrap(), FlowRun::Succeeded { flow_id } if flow_id == "a"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn reporting_does_not_fail_when_the_tracker_is_gone() {
        let (reporter, rx) = reporter(1);
        drop(rx);

        reporter.succeeded("a");
    }
}
