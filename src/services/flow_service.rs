use crate::domain::{FlowValidationError, validate};
use crate::flow_engine::flow::Flow;
use crate::flow_engine::{SchedulerCommand, VersionedFlow};
use crate::flow_loader;
use crate::flow_loader::{FlowFactoryError, SerializedFlow};
use crate::flow_registry::{FlowRegistry, RegisterResult, UnregisterResult};
use crate::flow_repository::{DeleteError, FlowRepository, FlowRepositoryError, InsertError, StoredFlow, UpdateError};
use crate::preset_registry::PresetRegistry;
use crate::store::StoreSnapshot;
use serde::Deserialize;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc::{OwnedPermit, Sender};
use tokio::sync::watch::Receiver;
use tokio::task::JoinError;
use tracing::debug;

#[derive(Debug)]
pub struct FlowService {
    flow_repository: Arc<FlowRepository>,
    flow_registry: Arc<FlowRegistry>,
    preset_registry: Arc<PresetRegistry>,
    scheduler_tx: Sender<SchedulerCommand>,
    snapshot_rx: Receiver<StoreSnapshot>,
}

impl FlowService {
    pub fn new(
        flow_repository: Arc<FlowRepository>,
        flow_registry: Arc<FlowRegistry>,
        preset_registry: Arc<PresetRegistry>,
        scheduler_tx: Sender<SchedulerCommand>,
        snapshot_rx: Receiver<StoreSnapshot>,
    ) -> Self {
        FlowService { flow_repository, flow_registry, preset_registry, scheduler_tx, snapshot_rx }
    }

    pub async fn list(&self) -> Result<Vec<StoredFlow>, ListFlowsError> {
        Ok(self.flow_repository.list().await?)
    }

    pub async fn retrieve_flow(&self, id: &str) -> Result<StoredFlow, RetrieveFlowError> {
        self.flow_repository.by_id(id).await?.ok_or(RetrieveFlowError::NotFound)
    }

    pub async fn create_flow(&self, document: serde_json::Value) -> Result<CreatedFlow, CreateFlowError> {
        let flow = self.validate(&document)?;
        let permit = self.reserve_scheduler_permit().await.ok_or(CreateFlowError::SchedulerUnavailable)?;

        let flow_registry = Arc::clone(&self.flow_registry);
        let flow_repository = Arc::clone(&self.flow_repository);
        run_to_completion(async move {
            let id = flow.id().to_string();
            let revision = flow_repository.insert(&id, document).await?;
            register_and_reconcile(&flow_registry, flow, revision, permit);
            Ok(CreatedFlow { id, revision })
        })
        .await
    }

    pub async fn update_flow(
        &self,
        id: &str,
        base_revision: u64,
        document: serde_json::Value,
    ) -> Result<u64, UpdateFlowError> {
        let flow = self.validate(&document)?;
        if id != flow.id() {
            return Err(UpdateFlowError::IdMismatch { id: id.to_string(), flow_id: flow.id().to_string() });
        }
        let permit = self.reserve_scheduler_permit().await.ok_or(UpdateFlowError::SchedulerUnavailable)?;

        let flow_registry = Arc::clone(&self.flow_registry);
        let flow_repository = Arc::clone(&self.flow_repository);
        let id = id.to_string();
        run_to_completion(async move {
            let revision = flow_repository.update(&id, base_revision, document).await?;
            register_and_reconcile(&flow_registry, flow, revision, permit);
            Ok(revision)
        })
        .await
    }

    pub async fn delete_flow(&self, id: &str, base_revision: u64) -> Result<(), DeleteFlowError> {
        let permit = self.reserve_scheduler_permit().await.ok_or(DeleteFlowError::SchedulerUnavailable)?;

        // Known race, accepted: an update that wrote this revision to the repository, but hasn't registered it yet,
        // re-adds the flow to the registry after this delete removes it. The flow then keeps running until the
        // next boot, which loads from the repository. Rare (needs a concurrent update and delete of the same flow),
        // so not worth a per-flow lock.
        let flow_repository = Arc::clone(&self.flow_repository);
        let flow_registry = Arc::clone(&self.flow_registry);
        let id = id.to_string();
        run_to_completion(async move {
            flow_repository.delete(&id, base_revision).await?;

            // The repository is authoritative, so the flow is deleted no matter the registry
            match flow_registry.unregister(&id, base_revision) {
                UnregisterResult::Deleted => {
                    // The reconcile will detect that the flow is gone from the registry and cancel any schedules
                    permit.send(SchedulerCommand::Reconcile { flow_id: id, revision: base_revision });
                }
                UnregisterResult::NotFound => {
                    debug!("Flow '{id}' was not registered, e.g. because it failed to load at boot")
                }
                UnregisterResult::Stale { current_revision } => {
                    debug!("Flow '{id}' revision {base_revision} is superseded by revision {current_revision}")
                }
            };
            Ok(())
        })
        .await
    }

    pub fn validate(&self, document: &serde_json::Value) -> Result<Flow, ValidateError> {
        // Structural validation
        let flow = flow_loader::from_json(SerializedFlow::deserialize(document)?)?;

        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap: clones the Arc<DeviceMap>

        // Semantic validation
        validate(&flow, &store_snapshot, &self.preset_registry).map_err(ValidateError::ValidationFailed)?;

        Ok(flow)
    }

    /// Reserves capacity before mutating anything, so an unavailable scheduler is rejected
    /// up front and the `Reconcile` send after the commit is infallible
    async fn reserve_scheduler_permit(&self) -> Option<OwnedPermit<SchedulerCommand>> {
        self.scheduler_tx.clone().reserve_owned().await.ok()
    }
}

/// Spawned so a client disconnect can't cancel the request between the repository write and the registry update,
/// the commit always runs to completion
async fn run_to_completion<T, E>(commit: impl Future<Output = Result<T, E>> + Send + 'static) -> Result<T, E>
where
    T: Send + 'static,
    E: From<JoinError> + Send + 'static,
{
    tokio::spawn(commit).await?
}

// The flow repository owns the revision; the registry mirrors it
fn register_and_reconcile(
    flow_registry: &FlowRegistry,
    flow: Flow,
    revision: u64,
    permit: OwnedPermit<SchedulerCommand>,
) {
    let id = flow.id().to_string();
    match flow_registry.register(VersionedFlow { flow: Arc::new(flow), revision }) {
        // Always reconcile: the scheduler derives the desired schedule state itself
        // Send on a reserved permit is synchronous and infallible
        RegisterResult::Added | RegisterResult::Replaced => {
            permit.send(SchedulerCommand::Reconcile { flow_id: id, revision });
        }
        // A concurrent write with a newer revision already landed and sent its own Reconcile
        RegisterResult::Stale { current_revision } => {
            debug!("Flow '{id}' revision {revision} is superseded by revision {current_revision}");
        }
    }
}

#[derive(Debug, Error)]
pub enum ListFlowsError {
    #[error(transparent)]
    Internal(#[from] FlowRepositoryError),
}

#[derive(Debug, Error)]
pub enum RetrieveFlowError {
    #[error("flow not found")]
    NotFound,
    #[error(transparent)]
    Internal(#[from] FlowRepositoryError),
}

#[derive(Debug)]
pub struct CreatedFlow {
    pub id: String,
    pub revision: u64,
}

#[derive(Debug, Error)]
pub enum CreateFlowError {
    #[error(transparent)]
    Invalid(#[from] ValidateError),
    #[error("scheduler is unavailable")]
    SchedulerUnavailable,
    #[error("flow already exists")]
    AlreadyExists,
    #[error(transparent)]
    Internal(FlowRepositoryError),
    #[error("flow commit task failed: {0}")]
    CommitFailed(#[from] JoinError),
}

impl From<InsertError> for CreateFlowError {
    fn from(err: InsertError) -> Self {
        match err {
            InsertError::AlreadyExists => CreateFlowError::AlreadyExists,
            InsertError::Repository(err) => CreateFlowError::Internal(err),
        }
    }
}

#[derive(Debug, Error)]
pub enum UpdateFlowError {
    #[error(transparent)]
    Invalid(#[from] ValidateError),
    #[error("flow id '{flow_id}' does not match id '{id}'")]
    IdMismatch { id: String, flow_id: String },
    #[error("scheduler is unavailable")]
    SchedulerUnavailable,
    #[error("flow not found")]
    NotFound,
    #[error(
        "revision conflict: update is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Internal(FlowRepositoryError),
    #[error("flow commit task failed: {0}")]
    CommitFailed(#[from] JoinError),
}

impl From<UpdateError> for UpdateFlowError {
    fn from(err: UpdateError) -> Self {
        match err {
            UpdateError::NotFound => UpdateFlowError::NotFound,
            UpdateError::RevisionConflict { base_revision, current_revision } => {
                UpdateFlowError::RevisionConflict { base_revision, current_revision }
            }
            UpdateError::Repository(err) => UpdateFlowError::Internal(err),
        }
    }
}

#[derive(Debug, Error)]
pub enum DeleteFlowError {
    #[error("scheduler is unavailable")]
    SchedulerUnavailable,
    #[error("flow not found")]
    NotFound,
    #[error(
        "revision conflict: delete is based on revision {base_revision}, but the current revision is {current_revision}"
    )]
    RevisionConflict { base_revision: u64, current_revision: u64 },
    #[error(transparent)]
    Internal(FlowRepositoryError),
    #[error("flow commit task failed: {0}")]
    CommitFailed(#[from] JoinError),
}

impl From<DeleteError> for DeleteFlowError {
    fn from(err: DeleteError) -> Self {
        match err {
            DeleteError::NotFound => DeleteFlowError::NotFound,
            DeleteError::RevisionConflict { base_revision, current_revision } => {
                DeleteFlowError::RevisionConflict { base_revision, current_revision }
            }
            DeleteError::Repository(err) => DeleteFlowError::Internal(err),
        }
    }
}

#[derive(Debug, Error)]
pub enum ValidateError {
    #[error("invalid flow document: {0}")]
    InvalidDocument(#[from] serde_json::Error),
    #[error(transparent)]
    InvalidFlow(#[from] FlowFactoryError),
    #[error(transparent)]
    ValidationFailed(FlowValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::domain::{Group, Location, Owner, Preset, Problem, ValidationIssue, Value, VersionedPreset};
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;
    use tokio::sync::{mpsc, watch};

    struct Fixture {
        service: FlowService,
        flow_repository: Arc<FlowRepository>,
        flow_registry: Arc<FlowRegistry>,
        scheduler_rx: mpsc::Receiver<SchedulerCommand>,
    }

    fn flow_document() -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[{"id":"startNode","type":"startNode","outgoingNode":"endNode"},{"id":"endNode","type":"endNode"}]})
    }

    /// Deserializes, but fails validation with a missing end node
    fn invalid_flow_document() -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[
            {"id":"startNode","type":"startNode","outgoingNode":"logNode"},
            {"id":"logNode","type":"actionNode","outgoingNode":"","action":{"type":"log","message":""}}
        ]})
    }

    fn flow() -> Flow {
        flow_loader::from_json(SerializedFlow::deserialize(&flow_document()).expect("valid document"))
            .expect("valid flow")
    }

    /// A flow whose only action controls the group
    fn flow_document_with_group_action(group_id: &str) -> serde_json::Value {
        json!({"id":"flow","name":"Test","nodes":[
            {"id":"startNode","type":"startNode","outgoingNode":"groupNode"},
            {"id":"groupNode","type":"actionNode","outgoingNode":"endNode","action":{"type":"controlGroup","groupId":group_id,"property":{}}},
            {"id":"endNode","type":"endNode"}
        ]})
    }

    fn unknown_group_issue(group_id: &str) -> ValidationIssue {
        Problem::UnknownGroup { group_id: group_id.to_string() }.at(Location::Node { node_id: "groupNode".to_string() })
    }

    fn snapshot_with_group(group_id: &str) -> StoreSnapshot {
        let group = Group {
            id: group_id.to_string(),
            name: "Room".to_string(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: Owner::Hearth,
            external_id: None,
        };
        StoreSnapshot {
            groups: Arc::new(HashMap::from([(group_id.to_string(), Arc::new(group))])),
            ..Default::default()
        }
    }

    fn create_service(flow_registry: FlowRegistry) -> Fixture {
        create_service_with_presets(flow_registry, vec![])
    }

    fn create_service_with_presets(flow_registry: FlowRegistry, presets: Vec<VersionedPreset>) -> Fixture {
        create_service_with(flow_registry, presets, StoreSnapshot::default())
    }

    fn create_service_with_snapshot(flow_registry: FlowRegistry, snapshot: StoreSnapshot) -> Fixture {
        create_service_with(flow_registry, vec![], snapshot)
    }

    fn create_service_with(
        flow_registry: FlowRegistry,
        presets: Vec<VersionedPreset>,
        snapshot: StoreSnapshot,
    ) -> Fixture {
        let database = Database::open(Path::new(":memory:")).expect("failed to open in-memory database");
        let flow_repository = Arc::new(FlowRepository::new(database));
        let flow_registry = Arc::new(flow_registry);
        let preset_registry = Arc::new(PresetRegistry::new(presets));
        let (scheduler_tx, scheduler_rx) = mpsc::channel(8);
        let (_, snapshot_rx) = watch::channel(snapshot);

        let service = FlowService::new(
            Arc::clone(&flow_repository),
            Arc::clone(&flow_registry),
            preset_registry,
            scheduler_tx,
            snapshot_rx,
        );
        Fixture { service, flow_repository, flow_registry, scheduler_rx }
    }

    mod list {
        use super::*;

        #[tokio::test]
        async fn returns_an_empty_list_for_an_empty_repository() {
            let Fixture { service, .. } = create_service(FlowRegistry::new(vec![]));

            let flows = service.list().await.expect("list succeeded");

            assert!(flows.is_empty(), "got {flows:?}");
        }

        #[tokio::test]
        async fn returns_the_stored_flows() {
            let Fixture { service, flow_repository, .. } = create_service(FlowRegistry::new(vec![]));
            flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            flow_repository.update("flow", 0, flow_document()).await.expect("update the repository");
            flow_repository.insert("other", flow_document()).await.expect("seed repository");

            let mut flows = service.list().await.expect("list succeeded");

            flows.sort_by(|a, b| a.id.cmp(&b.id));
            let ids_and_revisions: Vec<(&str, u64)> =
                flows.iter().map(|flow| (flow.id.as_str(), flow.revision)).collect();
            assert_eq!(ids_and_revisions, vec![("flow", 1), ("other", 0)]);
        }
    }

    mod retrieve_flow {
        use super::*;

        #[tokio::test]
        async fn returns_the_stored_flow() {
            let Fixture { service, flow_repository, .. } = create_service(FlowRegistry::new(vec![]));
            flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            flow_repository.update("flow", 0, flow_document()).await.expect("update the repository");

            let stored_flow = service.retrieve_flow("flow").await.expect("by_id succeeded");

            assert_eq!(stored_flow.id, "flow");
            assert_eq!(stored_flow.revision, 1);
            assert_eq!(stored_flow.document, flow_document());
        }

        #[tokio::test]
        async fn returns_not_found_for_an_unknown_flow() {
            let Fixture { service, flow_repository, .. } = create_service(FlowRegistry::new(vec![]));
            flow_repository.insert("other", flow_document()).await.expect("seed repository");

            let result = service.retrieve_flow("flow").await;

            assert!(matches!(result, Err(RetrieveFlowError::NotFound)), "got {result:?}");
        }
    }

    mod validate {
        use super::*;

        fn flow_document_with_preset_trigger(preset_id: &str) -> serde_json::Value {
            let mut document = flow_document();
            document["trigger"] = json!({"type":"presetValue","id":preset_id});
            document
        }

        fn presets() -> Vec<VersionedPreset> {
            let preset = Preset { id: "known".to_string(), name: "Known".to_string(), value: Value::Boolean(true) };
            vec![VersionedPreset { preset: Arc::new(preset), revision: 0 }]
        }

        #[test]
        fn accepts_a_valid_flow() {
            let Fixture { service, .. } = create_service(FlowRegistry::new(vec![]));

            let result = service.validate(&flow_document());

            assert!(result.is_ok(), "got {result:?}");
        }

        #[test]
        fn accepts_a_known_preset() {
            let Fixture { service, .. } = create_service_with_presets(FlowRegistry::new(vec![]), presets());

            let result = service.validate(&flow_document_with_preset_trigger("known"));

            assert!(result.is_ok(), "got {result:?}");
        }

        #[test]
        fn reports_an_unknown_preset() {
            let Fixture { service, .. } = create_service_with_presets(FlowRegistry::new(vec![]), presets());

            let result = service.validate(&flow_document_with_preset_trigger("unknown"));

            let Err(ValidateError::ValidationFailed(error)) = result else { panic!("got {result:?}") };
            assert_eq!(error.issues(), [Problem::PresetNotFound { id: "unknown".to_string() }.at(Location::Trigger)]);
        }

        #[test]
        fn accepts_a_known_group() {
            let Fixture { service, .. } =
                create_service_with_snapshot(FlowRegistry::new(vec![]), snapshot_with_group("room"));

            let result = service.validate(&flow_document_with_group_action("room"));

            assert!(result.is_ok(), "got {result:?}");
        }

        #[test]
        fn reports_an_unknown_group() {
            let Fixture { service, .. } =
                create_service_with_snapshot(FlowRegistry::new(vec![]), snapshot_with_group("room"));

            let result = service.validate(&flow_document_with_group_action("unknown"));

            let Err(ValidateError::ValidationFailed(error)) = result else { panic!("got {result:?}") };
            assert_eq!(error.issues(), [unknown_group_issue("unknown")]);
        }

        #[test]
        fn rejects_a_structurally_invalid_flow() {
            let Fixture { service, .. } = create_service(FlowRegistry::new(vec![]));

            let result = service.validate(&invalid_flow_document());

            assert!(matches!(result, Err(ValidateError::InvalidFlow(_))), "got {result:?}");
        }
    }

    mod create_flow {
        use super::*;

        #[tokio::test]
        async fn stores_registers_and_reconciles_a_new_flow() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } =
                create_service(FlowRegistry::new(vec![]));

            let created_flow = service.create_flow(flow_document()).await.expect("create_flow succeeded");

            assert_eq!((created_flow.id.as_str(), created_flow.revision), ("flow", 0));
            let stored_flow = flow_repository.by_id("flow").await.expect("by_id succeeded").expect("flow stored");
            assert_eq!(stored_flow.document, flow_document());
            assert_eq!(flow_registry.by_id("flow").expect("flow in registry").revision, 0);
            assert!(
                matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { flow_id, revision: 0 }) if flow_id == "flow")
            );
        }

        #[tokio::test]
        async fn returns_already_exists_for_an_existing_flow() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } =
                create_service(FlowRegistry::new(vec![]));
            flow_repository.insert("flow", flow_document()).await.expect("seed repository");

            let result = service.create_flow(flow_document()).await;

            assert!(matches!(result, Err(CreateFlowError::AlreadyExists)), "got {result:?}");
            assert!(flow_registry.by_id("flow").is_none(), "registry must be unchanged");
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        }

        #[tokio::test]
        async fn returns_invalid_document_for_a_document_that_is_not_a_flow() {
            let Fixture { service, flow_repository, .. } = create_service(FlowRegistry::new(vec![]));

            let result = service.create_flow(json!({"id": "flow"})).await;

            assert!(
                matches!(result, Err(CreateFlowError::Invalid(ValidateError::InvalidDocument(_)))),
                "got {result:?}"
            );
            assert!(flow_repository.list().await.unwrap().is_empty(), "nothing must be stored");
        }

        #[tokio::test]
        async fn returns_invalid_flow_for_a_semantically_invalid_flow() {
            let Fixture { service, flow_repository, mut scheduler_rx, .. } = create_service(FlowRegistry::new(vec![]));
            let document = json!({"id":"flow","name":"Test","nodes":[
                {"id":"startNode","type":"startNode","outgoingNode":"logNode"},
                {"id":"logNode","type":"actionNode","outgoingNode":"","action":{"type":"log","message":""}}
            ]});

            let result = service.create_flow(document).await;

            assert!(
                matches!(
                    result,
                    Err(CreateFlowError::Invalid(ValidateError::InvalidFlow(FlowFactoryError::MissingEndNode)))
                ),
                "got {result:?}"
            );
            assert!(flow_repository.list().await.unwrap().is_empty(), "nothing must be stored");
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        }

        #[tokio::test]
        async fn stores_a_flow_with_a_known_group() {
            let Fixture { service, flow_repository, scheduler_rx: _scheduler_rx, .. } =
                create_service_with_snapshot(FlowRegistry::new(vec![]), snapshot_with_group("room"));

            let result = service.create_flow(flow_document_with_group_action("room")).await;

            assert!(result.is_ok(), "got {result:?}");
            assert!(flow_repository.by_id("flow").await.unwrap().is_some(), "flow must be stored");
        }

        #[tokio::test]
        async fn returns_validation_failed_for_a_flow_with_an_unknown_group() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } =
                create_service_with_snapshot(FlowRegistry::new(vec![]), snapshot_with_group("room"));
            let capacity = service.scheduler_tx.capacity();

            let result = service.create_flow(flow_document_with_group_action("unknown")).await;

            let Err(CreateFlowError::Invalid(ValidateError::ValidationFailed(error))) = result else {
                panic!("got {result:?}")
            };
            assert_eq!(error.issues(), [unknown_group_issue("unknown")]);
            assert!(flow_repository.list().await.unwrap().is_empty(), "nothing must be stored");
            assert!(flow_registry.by_id("flow").is_none(), "registry must be unchanged");
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
            assert_eq!(service.scheduler_tx.capacity(), capacity, "no permit must be held");
        }

        #[tokio::test]
        async fn returns_scheduler_unavailable_when_the_scheduler_is_gone() {
            let Fixture { service, flow_repository, flow_registry, scheduler_rx } =
                create_service(FlowRegistry::new(vec![]));
            drop(scheduler_rx);

            let result = service.create_flow(flow_document()).await;

            assert!(matches!(result, Err(CreateFlowError::SchedulerUnavailable)), "got {result:?}");
            assert!(flow_repository.list().await.unwrap().is_empty(), "nothing must be stored");
            assert!(flow_registry.by_id("flow").is_none(), "registry must be unchanged");
        }

        #[tokio::test]
        async fn stores_but_does_not_reconcile_when_the_registry_has_a_newer_revision() {
            let newer = VersionedFlow { flow: Arc::new(flow()), revision: 5 };
            let Fixture { service, flow_registry, mut scheduler_rx, .. } =
                create_service(FlowRegistry::new(vec![newer]));
            let capacity = service.scheduler_tx.capacity();

            let created_flow = service.create_flow(flow_document()).await.expect("create_flow succeeded");

            assert_eq!(created_flow.revision, 0);
            assert_eq!(
                flow_registry.by_id("flow").expect("flow in registry").revision,
                5,
                "registry must be unchanged"
            );
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
            assert_eq!(service.scheduler_tx.capacity(), capacity, "permit must be released");
        }
    }

    mod update_flow {
        use super::*;

        /// Seeds the repository and registry with the flow at revision 0, like at boot
        async fn create_seeded_service() -> Fixture {
            let fixture =
                create_service(FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(flow()), revision: 0 }]));
            fixture.flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            fixture
        }

        async fn assert_unchanged(fixture: &mut Fixture, repository_revision: u64) {
            let stored_flow =
                fixture.flow_repository.by_id("flow").await.expect("by_id succeeded").expect("flow stored");
            assert_eq!(stored_flow.revision, repository_revision, "the repository must be unchanged");
            assert_eq!(
                fixture.flow_registry.by_id("flow").expect("flow in registry").revision,
                0,
                "registry must be unchanged"
            );
            assert!(fixture.scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        }

        #[tokio::test]
        async fn stores_registers_and_reconciles_the_flow() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } = create_seeded_service().await;

            let revision = service.update_flow("flow", 0, flow_document()).await.expect("update_flow succeeded");

            assert_eq!(revision, 1);
            let stored_flow = flow_repository.by_id("flow").await.expect("by_id succeeded").expect("flow stored");
            assert_eq!((stored_flow.revision, stored_flow.document), (1, flow_document()));
            assert_eq!(flow_registry.by_id("flow").expect("flow in registry").revision, 1);
            assert!(
                matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { flow_id, revision: 1 }) if flow_id == "flow")
            );
        }

        #[tokio::test]
        async fn returns_invalid_document_for_a_document_that_is_not_a_flow() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.update_flow("flow", 0, json!({"id": "flow"})).await;

            assert!(
                matches!(result, Err(UpdateFlowError::Invalid(ValidateError::InvalidDocument(_)))),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture, 0).await;
        }

        #[tokio::test]
        async fn returns_invalid_flow_for_a_semantically_invalid_flow() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.update_flow("flow", 0, invalid_flow_document()).await;

            assert!(
                matches!(
                    result,
                    Err(UpdateFlowError::Invalid(ValidateError::InvalidFlow(FlowFactoryError::MissingEndNode)))
                ),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture, 0).await;
        }

        #[tokio::test]
        async fn returns_validation_failed_for_a_flow_with_an_unknown_group() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.update_flow("flow", 0, flow_document_with_group_action("unknown")).await;

            let Err(UpdateFlowError::Invalid(ValidateError::ValidationFailed(error))) = result else {
                panic!("got {result:?}")
            };
            assert_eq!(error.issues(), [unknown_group_issue("unknown")]);
            assert_unchanged(&mut fixture, 0).await;
        }

        #[tokio::test]
        async fn returns_id_mismatch_when_the_id_does_not_match_the_flow_id() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.update_flow("other", 0, flow_document()).await;

            assert!(
                matches!(&result, Err(UpdateFlowError::IdMismatch { id, flow_id }) if id == "other" && flow_id == "flow"),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture, 0).await;
        }

        #[tokio::test]
        async fn returns_scheduler_unavailable_when_the_scheduler_is_gone() {
            let Fixture { service, flow_repository, flow_registry, scheduler_rx } = create_seeded_service().await;
            drop(scheduler_rx);

            let result = service.update_flow("flow", 0, flow_document()).await;

            assert!(matches!(result, Err(UpdateFlowError::SchedulerUnavailable)), "got {result:?}");
            assert_eq!(
                flow_repository.by_id("flow").await.unwrap().expect("flow stored").revision,
                0,
                "the repository must be unchanged"
            );
            assert_eq!(
                flow_registry.by_id("flow").expect("flow in registry").revision,
                0,
                "registry must be unchanged"
            );
        }

        #[tokio::test]
        async fn returns_not_found_for_an_unknown_flow() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } =
                create_service(FlowRegistry::new(vec![]));

            let result = service.update_flow("flow", 0, flow_document()).await;

            assert!(matches!(result, Err(UpdateFlowError::NotFound)), "got {result:?}");
            assert!(flow_repository.list().await.unwrap().is_empty(), "nothing must be stored");
            assert!(flow_registry.by_id("flow").is_none(), "nothing must be registered");
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        }

        #[tokio::test]
        async fn returns_revision_conflict_for_a_stale_base_revision() {
            let mut fixture = create_seeded_service().await;
            // Someone else updated the flow first
            fixture.flow_repository.update("flow", 0, flow_document()).await.expect("concurrent update");

            let result = fixture.service.update_flow("flow", 0, flow_document()).await;

            assert!(
                matches!(result, Err(UpdateFlowError::RevisionConflict { base_revision: 0, current_revision: 1 })),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture, 1).await;
        }
    }

    mod delete_flow {
        use super::*;

        /// Seeds the repository and registry with the flow at revision 0, like at boot
        async fn create_seeded_service() -> Fixture {
            let fixture =
                create_service(FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(flow()), revision: 0 }]));
            fixture.flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            fixture
        }

        async fn assert_unchanged(fixture: &mut Fixture) {
            assert!(
                fixture.flow_repository.by_id("flow").await.unwrap().is_some(),
                "flow must not be deleted from the repository"
            );
            assert!(fixture.flow_registry.by_id("flow").is_some(), "flow must not be deleted from the registry");
            assert!(fixture.scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
        }

        #[tokio::test]
        async fn deletes_unregisters_and_reconciles_the_flow() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } = create_seeded_service().await;

            service.delete_flow("flow", 0).await.expect("delete_flow succeeded");

            assert!(flow_repository.by_id("flow").await.unwrap().is_none(), "flow must be deleted from the repository");
            assert!(flow_registry.by_id("flow").is_none(), "flow must be deleted from the registry");
            assert!(
                matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { flow_id, revision: 0 }) if flow_id == "flow")
            );
        }

        #[tokio::test]
        async fn deletes_a_registry_entry_that_lags_behind_the_repository() {
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } = create_seeded_service().await;
            flow_repository.update("flow", 0, flow_document()).await.expect("update the repository");

            service.delete_flow("flow", 1).await.expect("delete_flow succeeded");

            assert!(flow_repository.by_id("flow").await.unwrap().is_none(), "flow must be deleted from the repository");
            assert!(flow_registry.by_id("flow").is_none(), "flow must be deleted from the registry");
            assert!(
                matches!(scheduler_rx.try_recv(), Ok(SchedulerCommand::Reconcile { flow_id, revision: 1 }) if flow_id == "flow")
            );
        }

        #[tokio::test]
        async fn deletes_a_stored_flow_that_is_not_registered() {
            // E.g. a flow that failed to load at boot
            let Fixture { service, flow_repository, mut scheduler_rx, .. } = create_service(FlowRegistry::new(vec![]));
            flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            let capacity = service.scheduler_tx.capacity();

            service.delete_flow("flow", 0).await.expect("delete_flow succeeded");

            assert!(flow_repository.by_id("flow").await.unwrap().is_none(), "flow must be deleted from the repository");
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
            assert_eq!(service.scheduler_tx.capacity(), capacity, "permit must be released");
        }

        #[tokio::test]
        async fn deletes_but_does_not_unregister_when_the_registry_has_a_newer_revision() {
            // A concurrent update stored and registered revision 1 after this delete's repository write
            let Fixture { service, flow_repository, flow_registry, mut scheduler_rx } =
                create_service(FlowRegistry::new(vec![VersionedFlow { flow: Arc::new(flow()), revision: 1 }]));
            flow_repository.insert("flow", flow_document()).await.expect("seed repository");
            let capacity = service.scheduler_tx.capacity();

            service.delete_flow("flow", 0).await.expect("delete_flow succeeded");

            assert!(flow_repository.by_id("flow").await.unwrap().is_none(), "flow must be deleted from the repository");
            assert_eq!(
                flow_registry.by_id("flow").expect("flow in registry").revision,
                1,
                "registry must be unchanged"
            );
            assert!(scheduler_rx.try_recv().is_err(), "nothing must be reconciled");
            assert_eq!(service.scheduler_tx.capacity(), capacity, "permit must be released");
        }

        #[tokio::test]
        async fn returns_scheduler_unavailable_when_the_scheduler_is_gone() {
            let Fixture { service, flow_repository, flow_registry, scheduler_rx } = create_seeded_service().await;
            drop(scheduler_rx);

            let result = service.delete_flow("flow", 0).await;

            assert!(matches!(result, Err(DeleteFlowError::SchedulerUnavailable)), "got {result:?}");
            assert!(
                flow_repository.by_id("flow").await.unwrap().is_some(),
                "flow must not be deleted from the repository"
            );
            assert!(flow_registry.by_id("flow").is_some(), "flow must not be deleted from the registry");
        }

        #[tokio::test]
        async fn returns_not_found_for_an_unknown_flow() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.delete_flow("other", 0).await;

            assert!(matches!(result, Err(DeleteFlowError::NotFound)), "got {result:?}");
            assert_unchanged(&mut fixture).await;
        }

        #[tokio::test]
        async fn returns_revision_conflict_for_a_stale_base_revision() {
            let mut fixture = create_seeded_service().await;
            // Someone else updated the flow first
            fixture.flow_repository.update("flow", 0, flow_document()).await.expect("concurrent update");

            let result = fixture.service.delete_flow("flow", 0).await;

            assert!(
                matches!(result, Err(DeleteFlowError::RevisionConflict { base_revision: 0, current_revision: 1 })),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture).await;
        }

        #[tokio::test]
        async fn returns_revision_conflict_for_a_future_base_revision() {
            let mut fixture = create_seeded_service().await;

            let result = fixture.service.delete_flow("flow", 1).await;

            assert!(
                matches!(result, Err(DeleteFlowError::RevisionConflict { base_revision: 1, current_revision: 0 })),
                "got {result:?}"
            );
            assert_unchanged(&mut fixture).await;
        }
    }

    mod run_to_completion {
        use super::*;
        use tokio::sync::oneshot;

        fn panicking_commit() -> Result<(), CreateFlowError> {
            panic!("commit failed")
        }

        #[tokio::test]
        async fn returns_the_result_of_the_commit() {
            let ok = run_to_completion(async { Ok::<_, CreateFlowError>(1) }).await;
            let err = run_to_completion(async { Err::<(), _>(CreateFlowError::AlreadyExists) }).await;

            assert!(matches!(ok, Ok(1)), "got {ok:?}");
            assert!(matches!(err, Err(CreateFlowError::AlreadyExists)), "got {err:?}");
        }

        #[tokio::test]
        async fn returns_commit_failed_when_the_commit_panics() {
            let result = run_to_completion(async { panicking_commit() }).await;

            assert!(matches!(result, Err(CreateFlowError::CommitFailed(_))), "got {result:?}");
        }

        #[tokio::test]
        async fn finishes_the_commit_when_the_caller_is_cancelled() {
            let (started_tx, started_rx) = oneshot::channel();
            let (release_tx, release_rx) = oneshot::channel();
            let (done_tx, done_rx) = oneshot::channel();
            let caller = tokio::spawn(run_to_completion(async move {
                started_tx.send(()).unwrap();
                release_rx.await.unwrap();
                done_tx.send(()).unwrap();
                Ok::<_, CreateFlowError>(())
            }));
            started_rx.await.expect("commit started");

            // Simulates a client disconnect halfway through the commit
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            release_tx.send(()).unwrap();

            done_rx.await.expect("commit ran to completion");
        }
    }
}
