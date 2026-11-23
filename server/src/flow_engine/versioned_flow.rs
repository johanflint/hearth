use crate::flow_engine::flow::Flow;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct VersionedFlow {
    pub flow: Arc<Flow>,
    pub revision: u64,
}

impl VersionedFlow {
    pub fn into_flow(self) -> Arc<Flow> {
        self.flow
    }
}

impl Deref for VersionedFlow {
    type Target = Flow;

    fn deref(&self) -> &Self::Target {
        &self.flow
    }
}
