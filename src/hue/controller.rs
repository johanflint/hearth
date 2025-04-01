use crate::domain::commands::Command;
use crate::domain::controller::Controller;
use async_trait::async_trait;
use tracing::{info, instrument};

#[derive(Debug)]
pub struct HueController {}

pub const CONTROLLER_ID: &str = "hue";

#[async_trait]
impl Controller for HueController {
    fn id(&self) -> &'static str {
        CONTROLLER_ID
    }

    #[instrument(skip_all)]
    async fn execute(&self, command: Command) {
        info!(controller_id = self.id(), "{:#?}", command);
    }
}
