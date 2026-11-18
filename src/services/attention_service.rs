use crate::attention::AttentionList;
use tokio::sync::watch;

#[derive(Debug)]
pub struct AttentionService {
    attention_rx: watch::Receiver<AttentionList>,
}

impl AttentionService {
    pub fn new(attention_rx: watch::Receiver<AttentionList>) -> Self {
        AttentionService { attention_rx }
    }

    pub fn list(&self) -> AttentionList {
        self.attention_rx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<AttentionList> {
        self.attention_rx.clone()
    }
}
