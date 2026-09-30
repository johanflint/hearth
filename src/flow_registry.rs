use crate::flow_engine::VersionedFlow;
use std::collections::HashMap;
use std::sync::RwLock;

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

    pub fn replace_existing(&self, versioned_flow: VersionedFlow) -> ReplaceResult {
        let mut entries = self.entries.write().expect("flow registry lock poisoned");
        let Some(entry) = entries.get_mut(versioned_flow.flow.id()) else {
            return ReplaceResult::NotFound;
        };

        if versioned_flow.revision <= entry.revision {
            return ReplaceResult::Stale { current_revision: entry.revision };
        }

        *entry = versioned_flow;
        ReplaceResult::Replaced
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReplaceResult {
    Replaced,
    // A newer (or equal) revision is already stored; the registry is unchanged
    Stale { current_revision: u64 },
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow_engine::Schedule;
    use crate::flow_engine::flow::{Flow, FlowNode, FlowNodeKind};
    use std::sync::Arc;

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
    fn replace_existing_returns_not_found_for_an_unknown_flow() {
        let registry = FlowRegistry::new(vec![]);

        assert_eq!(registry.replace_existing(reactive_flow("missing")), ReplaceResult::NotFound);
        assert!(registry.by_id("missing").is_none());
    }

    #[test]
    fn replace_existing_installs_a_newer_revision() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        let replacement = VersionedFlow { flow: Arc::new(flow("flow", Some(Schedule::Cron("* * * * * *".to_string())))), revision: 1 };

        assert_eq!(registry.replace_existing(replacement), ReplaceResult::Replaced);

        let entry = registry.by_id("flow").unwrap();
        assert_eq!(entry.revision, 1);
        assert!(entry.flow.schedule().is_some());
    }

    #[test]
    fn replace_existing_accepts_a_revision_gap() {
        // Concurrent updates may land out of order; newest wins even if it skips a revision
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        let replacement = VersionedFlow { flow: Arc::new(flow("flow", None)), revision: 4 };

        assert_eq!(registry.replace_existing(replacement), ReplaceResult::Replaced);
        assert_eq!(registry.by_id("flow").unwrap().revision, 4);
    }

    #[test]
    fn replace_existing_rejects_an_older_revision() {
        let registry = FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(flow("flow", None)), revision: 2 }]);
        let stale = VersionedFlow { flow: Arc::new(flow("flow", None)), revision: 1 };

        assert_eq!(registry.replace_existing(stale), ReplaceResult::Stale { current_revision: 2 });

        let entry = registry.by_id("flow").unwrap();
        assert_eq!(entry.revision, 2);
        assert!(entry.flow.schedule().is_none());
    }

    #[test]
    fn replace_existing_rejects_the_same_revision() {
        let registry = FlowRegistry::new(vec![reactive_flow("flow")]);
        let same = VersionedFlow { flow: Arc::new(flow("flow", None)), revision: 0 };

        assert_eq!(registry.replace_existing(same), ReplaceResult::Stale { current_revision: 0 });
        assert!(registry.by_id("flow").unwrap().flow.schedule().is_none());
    }
}