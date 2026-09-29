use crate::flow_engine::VersionedFlow;
use crate::flow_engine::flow::Flow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub struct FlowRegistry {
    entries: RwLock<HashMap<String, VersionedFlow>>,
}

impl FlowRegistry {
    pub fn new(flows: Vec<VersionedFlow>) -> Self {
        let entries = flows
            .into_iter()
            .map(|versioned_flow| (versioned_flow.flow.id().to_string(), versioned_flow))
            .collect();

        Self { entries: RwLock::new(entries) }
    }

    pub fn reactive_flows(&self) -> Vec<VersionedFlow> {
        self.entries
            .read().expect("flow registry lock poisoned")
            .values()
            .filter(|entry| entry.schedule().is_none())
            .cloned()
            .collect()
    }

    pub fn scheduled_flows(&self) -> Vec<VersionedFlow> {
        self.entries
            .read().expect("flow registry lock poisoned")
            .values()
            .filter(|entry| entry.schedule().is_some())
            .cloned()
            .collect()
    }

    pub fn by_id(&self, id: &str) -> Option<VersionedFlow> {
        self.entries.read().expect("flow registry lock poisoned").get(id).cloned()
    }

    pub fn replace_existing(&self, flow: Flow) -> Option<u64> {
        let mut entries = self.entries.write().expect("flow registry lock poisoned");
        let existing_flow = entries.get(flow.id())?;
        let revision = existing_flow.revision.wrapping_add(1);

        entries.insert(flow.id().to_string(), VersionedFlow { flow: Arc::new(flow), revision });

        Some(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Schedule;
    use crate::flow_engine::flow::{Flow, FlowNode, FlowNodeKind};

    fn reactive_flow(id: &str) -> VersionedFlow {
        versioned_flow(id, None)
    }

    fn scheduled_flow(id: &str) -> VersionedFlow {
        versioned_flow(id, Some(Schedule::Cron("* * * * * *".to_string())))
    }

    fn versioned_flow(id: &str, schedule: Option<Schedule>) -> VersionedFlow {
        VersionedFlow {
            flow: Arc::new(flow(id, schedule)),
            revision: 0,
        }
    }

    fn flow(id: &str, schedule: Option<Schedule>) -> Flow {
        let start_node = FlowNode::new(format!("{id}_start"), vec![], FlowNodeKind::Start);
        Flow::new(id.to_string(), id.to_string(), schedule, None, Arc::new(start_node), HashMap::new()).unwrap()
    }

    #[test]
    fn reactive_flows_excludes_scheduled_flows() {
        let registry = FlowRegistry::new(vec![reactive_flow("reactive"), scheduled_flow("scheduled")]);
        let ids: Vec<_> = registry.reactive_flows().into_iter().map(|flow| flow.id().to_string()).collect();
        assert_eq!(ids, vec!["reactive"]);
    }

    #[test]
    fn scheduled_flows_excludes_reactive_flows() {
        let registry = FlowRegistry::new(vec![reactive_flow("reactive"), scheduled_flow("scheduled")]);
        let ids: Vec<_> = registry.scheduled_flows().into_iter().map(|flow| flow.id().to_string()).collect();
        assert_eq!(ids, vec!["scheduled"]);
    }

    #[test]
    fn by_id_returns_none_for_an_unknown_flow() {
        let registry = FlowRegistry::new(vec![]);
        assert!(registry.by_id("missing").is_none());
    }

    #[test]
    fn by_id_returns_the_flow_when_present() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        assert_eq!(registry.by_id("flow").unwrap().flow.id(), "flow");
    }

    #[test]
    fn by_id_starts_at_revision_zero() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        assert_eq!(registry.by_id("flow").unwrap().revision, 0);
    }
    
    #[test]
    fn replace_existing_returns_none_for_an_unknown_flow() {
        let registry = FlowRegistry::new(vec![]);
        assert!(registry.replace_existing(flow("missing", None)).is_none());
    }

    #[test]
    fn replace_existing_installs_the_new_flow_and_bumps_the_revision() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);

        let revision = registry.replace_existing(flow("flow", Some(Schedule::Cron("* * * * * *".to_string())))).unwrap();

        assert_eq!(revision, 1);
        let entry = registry.by_id("flow").unwrap();
        assert_eq!(entry.revision, 1);
        assert!(entry.flow.schedule().is_some());
    }

    #[test]
    fn replace_existing_bumps_the_revision_on_every_call() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);

        registry.replace_existing(flow("flow", None));
        let revision = registry.replace_existing(flow("flow", None)).unwrap();

        assert_eq!(revision, 2);
    }
}