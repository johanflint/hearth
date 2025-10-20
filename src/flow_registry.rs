use crate::flow_engine::flow::Flow;
use std::collections::HashMap;
use std::sync::Arc;

pub struct FlowRegistry {
    flows: Vec<Arc<Flow>>,
    by_id: HashMap<String, usize>,
}

impl FlowRegistry {
    pub fn new(flows: Vec<Flow>) -> Self {
        let by_id = flows.iter().enumerate().map(|(index, flow)| (flow.id().to_string(), index)).collect();
        let flow_arcs = flows.into_iter().map(|flow| Arc::new(flow)).collect();

        Self { flows: flow_arcs, by_id }
    }

    pub fn reactive_flows(&self) -> Vec<Arc<Flow>> {
        self.flows.iter().filter(|flow| flow.schedule().is_none()).cloned().collect()
    }

    pub fn scheduled_flows(&self) -> Vec<Arc<Flow>> {
        self.flows.iter().filter(|flow| flow.schedule().is_some()).cloned().collect()
    }

    pub fn by_id(&self, id: &str) -> Option<Arc<Flow>> {
        self.by_id.get(id).map(|&index| &self.flows[index]).cloned()
    }
}
