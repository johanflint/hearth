use crate::domain::Zone;
use crate::store::StoreSnapshot;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::watch::Receiver;

#[derive(Debug)]
pub struct ZoneService {
    snapshot_rx: Receiver<StoreSnapshot>,
}

impl ZoneService {
    pub fn new(snapshot_rx: Receiver<StoreSnapshot>) -> Self {
        ZoneService { snapshot_rx }
    }

    pub fn list(&self) -> Vec<Arc<Zone>> {
        let store_snapshot = self.snapshot_rx.borrow().clone(); // Cheap: clones the Arc<ZoneMap>
        let mut zones: Vec<_> = store_snapshot.zones.values().cloned().collect();
        // First sort by name, then by id
        zones.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
        zones
    }

    pub fn retrieve_zone(&self, id: &str) -> Result<Arc<Zone>, RetrieveZoneError> {
        self.snapshot_rx.borrow().zones.get(id).cloned().ok_or(RetrieveZoneError::NotFound)
    }
}

#[derive(Debug, Error)]
pub enum RetrieveZoneError {
    #[error("zone not found")]
    NotFound,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Owner;
    use std::collections::{BTreeSet, HashMap};
    use tokio::sync::watch;

    fn zone(id: &str, name: &str) -> Arc<Zone> {
        Arc::new(Zone {
            id: id.to_string(),
            name: name.to_string(),
            group_ids: BTreeSet::new(),
            device_ids: BTreeSet::new(),
            revision: 0,
            owner: Owner::Integration,
            external_id: None,
        })
    }

    fn snapshot(zones: Vec<Arc<Zone>>) -> StoreSnapshot {
        let zones: HashMap<_, _> = zones.into_iter().map(|zone| (zone.id.clone(), zone)).collect();
        StoreSnapshot { zones: Arc::new(zones), ..Default::default() }
    }

    fn names(zones: &[Arc<Zone>]) -> Vec<&str> {
        zones.iter().map(|zone| zone.name.as_str()).collect()
    }

    #[test]
    fn list_returns_nothing_for_an_empty_snapshot() {
        let (_tx, rx) = watch::channel(StoreSnapshot::default());

        assert!(ZoneService::new(rx).list().is_empty());
    }

    #[test]
    fn list_returns_all_zones_in_the_snapshot() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden"), zone("2", "Upstairs")]));

        let zones = ZoneService::new(rx).list();

        assert_eq!(zones.len(), 2);
    }

    #[test]
    fn list_orders_the_zones_by_name_instead_of_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![
            zone("1", "Upstairs"),
            zone("2", "Downstairs"),
            zone("3", "Garden"),
            zone("4", "Attic"),
        ]));

        let zones = ZoneService::new(rx).list();

        assert_eq!(names(&zones), vec!["Attic", "Downstairs", "Garden", "Upstairs"]);
    }

    #[test]
    fn list_orders_zones_with_the_same_name_by_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("c", "Zone"), zone("a", "Zone"), zone("b", "Zone")]));

        let zones = ZoneService::new(rx).list();

        let ids: Vec<_> = zones.iter().map(|zone| zone.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn list_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(snapshot(vec![zone("1", "Upstairs")]));
        let service = ZoneService::new(rx);

        tx.send(snapshot(vec![zone("1", "Upstairs"), zone("2", "Garden")])).unwrap();

        assert_eq!(names(&service.list()), vec!["Garden", "Upstairs"]);
    }

    #[test]
    fn retrieve_zone_returns_the_zone_with_the_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden"), zone("2", "Upstairs")]));

        let found = ZoneService::new(rx).retrieve_zone("2").unwrap();

        assert_eq!(found.name, "Upstairs");
    }

    #[test]
    fn retrieve_zone_returns_not_found_for_an_unknown_id() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden")]));

        let result = ZoneService::new(rx).retrieve_zone("unknown");

        assert!(matches!(result, Err(RetrieveZoneError::NotFound)));
    }

    #[test]
    fn retrieve_zone_does_not_match_on_the_name() {
        let (_tx, rx) = watch::channel(snapshot(vec![zone("1", "Garden")]));

        let result = ZoneService::new(rx).retrieve_zone("Garden");

        assert!(matches!(result, Err(RetrieveZoneError::NotFound)));
    }

    #[test]
    fn retrieve_zone_follows_the_latest_snapshot() {
        let (tx, rx) = watch::channel(StoreSnapshot::default());
        let service = ZoneService::new(rx);

        tx.send(snapshot(vec![zone("1", "Garden")])).unwrap();

        assert!(service.retrieve_zone("1").is_ok());
    }
}
