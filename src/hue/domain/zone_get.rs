use crate::hue::domain::{Metadata, Owner};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_zone_get
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ZoneGet {
    pub id: String,
    pub children: Vec<Owner>, // lights in the zone, unlike a room, which contains devices
    pub services: Vec<Owner>, // contains the grouped_light of the zone
    pub metadata: Metadata,
}

#[derive(Debug, Deserialize)]
pub struct ZoneChanged {
    pub id: String,
    pub children: Option<Vec<Owner>>,
    pub services: Option<Vec<Owner>>,
    pub metadata: Option<ZoneMetadataChanged>,
}

// An update only carries the changed metadata fields, so unlike `Metadata` nothing is required
#[derive(Debug, Deserialize)]
pub struct ZoneMetadataChanged {
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_zone_get() {
        let json = json!({
            "id": "55657751-766f-47b2-9506-e3595ae0a422",
            "id_v1": "/groups/22",
            "children": [
                { "rid": "47b4d15d-fdb9-479b-a505-88ab12d560c8", "rtype": "light" },
                { "rid": "52e19969-8f64-4e87-ba87-e60838ab989b", "rtype": "light" }
            ],
            "services": [
                { "rid": "718c837f-8691-4d40-9c58-816854268c49", "rtype": "grouped_light" }
            ],
            "metadata": { "name": "Buiten", "archetype": "garden" },
            "type": "zone"
        });

        let zone_get = serde_json::from_value::<ZoneGet>(json).unwrap();

        assert_eq!(zone_get.id, "55657751-766f-47b2-9506-e3595ae0a422");
        assert_eq!(zone_get.children.len(), 2);
        assert_eq!(zone_get.children[0].rid, "47b4d15d-fdb9-479b-a505-88ab12d560c8");
        assert_eq!(zone_get.children[0].rtype, "light");
        assert_eq!(zone_get.services[0].rid, "718c837f-8691-4d40-9c58-816854268c49");
        assert_eq!(zone_get.services[0].rtype, "grouped_light");
        assert_eq!(zone_get.metadata.name, "Buiten");
    }

    #[test]
    fn deserialize_zone_get_without_children() {
        let json = json!({
            "id": "55657751-766f-47b2-9506-e3595ae0a422",
            "children": [],
            "services": [],
            "metadata": { "name": "Empty zone", "archetype": "other" },
            "type": "zone"
        });

        let zone_get = serde_json::from_value::<ZoneGet>(json).unwrap();

        assert!(zone_get.children.is_empty());
        assert!(zone_get.services.is_empty());
    }

    #[test]
    fn deserialize_zone_changed_with_only_a_name() {
        let json = json!({
            "id": "zone-id",
            "id_v1": "/groups/22",
            "metadata": { "name": "Upstairs" },
            "type": "zone"
        });

        let zone: ZoneChanged = serde_json::from_value(json).unwrap();

        assert_eq!(zone.id, "zone-id");
        assert_eq!(zone.metadata.unwrap().name, Some("Upstairs".to_string()));
        assert!(zone.children.is_none());
        assert!(zone.services.is_none());
    }

    #[test]
    fn deserialize_zone_changed_with_only_an_archetype() {
        let json = json!({ "id": "zone-id", "metadata": { "archetype": "upstairs" }, "type": "zone" });

        let zone: ZoneChanged = serde_json::from_value(json).unwrap();

        assert_eq!(zone.metadata.unwrap().name, None);
    }

    #[test]
    fn deserialize_zone_changed_with_only_children() {
        let json = json!({
            "id": "zone-id",
            "children": [{ "rid": "light-1", "rtype": "light" }],
            "type": "zone"
        });

        let zone: ZoneChanged = serde_json::from_value(json).unwrap();

        assert_eq!(zone.children.unwrap().len(), 1);
        assert!(zone.metadata.is_none());
    }
}
