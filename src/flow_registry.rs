use crate::flow_engine::flow::Flow;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug)]
pub struct FlowRegistry {
    entries: RwLock<HashMap<String, RegistryEntry>>,
}

#[derive(Debug, Clone)]
pub(crate) struct RegistryEntry {
    pub(crate) flow: Arc<Flow>,
    pub(crate) revision: u64,
}

impl FlowRegistry {
    pub fn new(flows: Vec<Flow>) -> Self {
        let entries = flows
            .into_iter()
            .map(|flow| (flow.id().to_string(), RegistryEntry { flow: Arc::new(flow), revision: 0 }))
            .collect();

        Self { entries: RwLock::new(entries) }
    }

    pub fn reactive_flows(&self) -> Vec<Arc<Flow>> {
        self.entries
            .read().expect("flow registry lock poisoned")
            .values()
            .filter(|entry| entry.flow.schedule().is_none())
            .map(|entry| entry.flow.clone())
            .collect()
    }

    pub fn scheduled_flows(&self) -> Vec<Arc<Flow>> {
        self.entries
            .read().expect("flow registry lock poisoned")
            .values()
            .filter(|entry| entry.flow.schedule().is_some())
            .map(|entry| entry.flow.clone())
            .collect()
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<Flow>> {
        self.entries.read().expect("flow registry lock poisoned").get(id).map(|entry| entry.flow.clone())
    }

    pub fn by_id_with_revision(&self, id: &str) -> Option<RegistryEntry> {
        self.entries.read().expect("flow registry lock poisoned").get(id).cloned()
    }

    pub fn replace_existing(&self, flow: Flow) -> Option<u64> {
        let mut entries = self.entries.write().expect("flow registry lock poisoned");
        let existing_flow = entries.get(flow.id())?;
        let revision = existing_flow.revision + 1;

        entries.insert(flow.id().to_string(), RegistryEntry { flow: Arc::new(flow), revision });

        Some(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Schedule;
    use crate::flow_engine::flow::{FlowNode, FlowNodeKind};

    fn reactive_flow(id: &str) -> Flow {
        flow(id, None)
    }

    fn scheduled_flow(id: &str) -> Flow {
        flow(id, Some(Schedule::Cron("* * * * * *".to_string())))
    }

    fn flow(id: &str, schedule: Option<Schedule>) -> Flow {
        let start_node = FlowNode::new(format!("{id}_start"), vec![], FlowNodeKind::Start);
        Flow::new(id.to_string(), id.to_string(), schedule, None, Arc::new(start_node), HashMap::new()).unwrap()
    }

    #[test]
    fn reactive_flows_excludes_scheduled_flows() {
        let registry = FlowRegistry::new(vec![reactive_flow("reactive"), scheduled_flow("scheduled")]);
        let ids: Vec<_> = registry.reactive_flows().iter().map(|flow| flow.id().to_string()).collect();
        assert_eq!(ids, vec!["reactive"]);
    }

    #[test]
    fn scheduled_flows_excludes_reactive_flows() {
        let registry = FlowRegistry::new(vec![reactive_flow("reactive"), scheduled_flow("scheduled")]);
        let ids: Vec<_> = registry.scheduled_flows().iter().map(|flow| flow.id().to_string()).collect();
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
        assert_eq!(registry.by_id("flow").unwrap().id(), "flow");
    }

    #[test]
    fn by_id_with_revision_starts_at_revision_zero() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        assert_eq!(registry.by_id_with_revision("flow").unwrap().revision, 0);
    }

    #[test]
    fn by_id_with_revision_returns_none_for_an_unknown_flow() {
        let registry = FlowRegistry::new(vec![]);
        assert!(registry.by_id_with_revision("missing").is_none());
    }

    #[test]
    fn replace_existing_returns_none_for_an_unknown_flow() {
        let registry = FlowRegistry::new(vec![]);
        assert!(registry.replace_existing(reactive_flow("missing")).is_none());
    }

    #[test]
    fn replace_existing_installs_the_new_flow_and_bumps_the_revision() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);

        let revision = registry.replace_existing(scheduled_flow("flow")).unwrap();

        assert_eq!(revision, 1);
        let entry = registry.by_id_with_revision("flow").unwrap();
        assert_eq!(entry.revision, 1);
        assert!(entry.flow.schedule().is_some());
    }

    #[test]
    fn replace_existing_bumps_the_revision_on_every_call() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);

        registry.replace_existing(reactive_flow("flow"));
        let revision = registry.replace_existing(reactive_flow("flow")).unwrap();

        assert_eq!(revision, 2);
    }
}