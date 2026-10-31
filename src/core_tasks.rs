use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct CoreTasks {
    pub store: JoinHandle<()>,
    pub scheduler: JoinHandle<()>,
    pub store_listener: JoinHandle<()>,
}
