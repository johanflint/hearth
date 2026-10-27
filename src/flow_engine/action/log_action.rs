use crate::flow_engine::Expression;
use crate::flow_engine::action::Action;
use crate::flow_engine::context::Context;
use crate::flow_engine::scope::Scope;
use action_macros::register_action;
use async_trait::async_trait;
use serde::Deserialize;
use std::any::Any;
use tracing::{info, instrument};

#[derive(Debug, Deserialize, Default, PartialEq)]
#[register_action]
pub struct LogAction {
    message: String,
}

#[cfg(test)]
impl LogAction {
    pub fn new(message: String) -> LogAction {
        LogAction { message }
    }
}

#[async_trait]
impl Action for LogAction {
    fn kind(&self) -> &'static str {
        "log"
    }

    #[instrument(fields(action = self.kind()), skip_all)]
    async fn execute(&self, _context: &Context, _scope: &mut Scope) {
        info!(target: "hearth::flow_log", "{}", self.message);
    }

    fn expressions(&self) -> Vec<&Expression> {
        vec![]
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preset_registry::PresetRegistry;
    use crate::store::StoreSnapshot;
    use pretty_assertions::assert_eq;
    use std::io;
    use std::sync::Arc;

    fn preset_registry() -> Arc<PresetRegistry> {
        Arc::new(PresetRegistry::new(vec![]))
    }

    #[test]
    fn deserialize_log_action() -> io::Result<()> {
        let json = r#"{
            "type": "log",
            "message": "Hello"
        }"#;

        let node = serde_json::from_str::<Box<dyn Action>>(json)?;

        let expected = LogAction { message: "Hello".to_string() };

        let action = node.as_any().downcast_ref::<LogAction>().unwrap();
        assert_eq!(&expected, action);

        Ok(())
    }

    #[test]
    fn log_action_has_nothing_to_validate() {
        assert_eq!(LogAction::new("message".to_string()).validate(&StoreSnapshot::default(), &preset_registry()), []);
    }
}
