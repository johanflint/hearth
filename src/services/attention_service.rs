use crate::attention::Attention;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug)]
pub struct AttentionService {
    attention_rx: watch::Receiver<Arc<[Attention]>>,
}

impl AttentionService {
    pub fn new(attention_rx: watch::Receiver<Arc<[Attention]>>) -> Self {
        AttentionService { attention_rx }
    }

    pub fn list(&self) -> Arc<[Attention]> {
        self.attention_rx.borrow().clone()
    }
}
