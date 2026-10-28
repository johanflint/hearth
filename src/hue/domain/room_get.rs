use crate::hue::domain::{Metadata, Owner};
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_room_get
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct RoomGet {
    pub id: String,
    pub children: Vec<Owner>, // devices in the room
    pub services: Vec<Owner>, // contains the grouped_light of the room
    pub metadata: Metadata,
}

#[derive(Debug, Deserialize)]
pub struct RoomChanged {
    pub id: String,
    pub children: Option<Vec<Owner>>,
    pub services: Option<Vec<Owner>>,
    pub metadata: Option<RoomMetadataChanged>,
}

// An update only carries the changed metadata fields, so unlike `Metadata` nothing is required
#[derive(Debug, Deserialize)]
pub struct RoomMetadataChanged {
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_room_changed_with_only_a_name() {
        let json = json!({
            "id": "room-id",
            "id_v1": "/groups/14",
            "metadata": { "name": "Study" },
            "type": "room"
        });

        let room: RoomChanged = serde_json::from_value(json).unwrap();

        assert_eq!(room.id, "room-id");
        assert_eq!(room.metadata.unwrap().name, Some("Study".to_string()));
        assert!(room.children.is_none());
        assert!(room.services.is_none());
    }

    #[test]
    fn deserialize_room_changed_with_only_an_archetype() {
        let json = json!({ "id": "room-id", "metadata": { "archetype": "office" }, "type": "room" });

        let room: RoomChanged = serde_json::from_value(json).unwrap();

        assert_eq!(room.metadata.unwrap().name, None);
    }

    #[test]
    fn deserialize_room_get() {
        let json = json!({
            "id": "064e61aa-b077-4665-9bea-0ca4c332a331",
            "id_v1": "/groups/1",
            "children": [
                { "rid": "d6082688-85d9-4f5a-a3f3-f36e47c7c432", "rtype": "device" }
            ],
            "services": [
                { "rid": "6c1524d8-1fd7-40fc-a6f4-77e5a4146d89", "rtype": "grouped_light" }
            ],
            "metadata": { "name": "Bureau", "archetype": "office" },
            "type": "room"
        });

        let room_get = serde_json::from_value::<RoomGet>(json).unwrap();

        assert_eq!(room_get.id, "064e61aa-b077-4665-9bea-0ca4c332a331");
        assert_eq!(room_get.children.len(), 1);
        assert_eq!(room_get.children[0].rid, "d6082688-85d9-4f5a-a3f3-f36e47c7c432");
        assert_eq!(room_get.children[0].rtype, "device");
        assert_eq!(room_get.services[0].rid, "6c1524d8-1fd7-40fc-a6f4-77e5a4146d89");
        assert_eq!(room_get.services[0].rtype, "grouped_light");
        assert_eq!(room_get.metadata.name, "Bureau");
    }

    #[test]
    fn deserialize_room_get_without_children() {
        let json = json!({
            "id": "064e61aa-b077-4665-9bea-0ca4c332a331",
            "children": [],
            "services": [],
            "metadata": { "name": "Empty room", "archetype": "other" },
            "type": "room"
        });

        let room_get = serde_json::from_value::<RoomGet>(json).unwrap();

        assert!(room_get.children.is_empty());
        assert!(room_get.services.is_empty());
    }
}
