use crate::domain::commands::Command;
use async_trait::async_trait;
use std::fmt::Debug;

#[async_trait]
pub trait Controller: Debug + Send + Sync {
    fn id(&self) -> &'static str;

    async fn execute(&self, command: Command);
}
