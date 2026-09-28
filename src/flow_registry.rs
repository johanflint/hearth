use crate::flow_engine::flow::Flow;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct FlowRegistry {
    entries: HashMap<String, RegistryEntry>,
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

        Self { entries }
    }

    pub fn reactive_flows(&self) -> Vec<Arc<Flow>> {
        self.entries.values()
            .filter(|entry| entry.flow.schedule().is_none())
            .map(|entry| entry.flow.clone())
            .collect()
    }

    pub fn scheduled_flows(&self) -> Vec<Arc<Flow>> {
        self.entries.values()
            .filter(|entry| entry.flow.schedule().is_some())
            .map(|entry| entry.flow.clone())
            .collect()
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<Flow>> {
        self.entries.get(id).map(|entry| entry.flow.clone())
    }

    pub fn by_id_with_revision(&self, id: &str) -> Option<RegistryEntry> {
        self.entries.get(id).cloned()
    }
}
