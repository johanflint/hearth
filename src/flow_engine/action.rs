use async_trait::async_trait;
use std::fmt::Debug;
use tracing::{info, instrument};

#[async_trait]
pub trait Action: Debug + Send + Sync {
    fn kind(&self) -> &'static str;

    async fn execute(&self);
}

#[derive(Debug)]
pub struct LogAction {
    message: String,
}

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

    #[instrument(skip(self))]
    async fn execute(&self) {
        info!("{}", self.message);
    }
}
