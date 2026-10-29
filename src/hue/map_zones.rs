use crate::domain::{GroupOwner, Zone};
use crate::hue::domain::{LightGet, ZoneGet};
use crate::hue::map_rooms::grouped_light_id;
use std::collections::BTreeSet;
use thiserror::Error;

pub fn map_zones(zones: Vec<ZoneGet>, lights: &[LightGet]) -> Result<Vec<Zone>, MapZonesError> {
    let zones = zones.into_iter().map(|zone| map_zone(zone, lights)).collect::<Result<Vec<_>, _>>()?;

    // A zone without lights can't be controlled
    Ok(zones.into_iter().filter(|zone| !zone.device_ids.is_empty()).collect())
}

fn map_zone(zone: ZoneGet, lights: &[LightGet]) -> Result<Zone, MapZonesError> {
    let device_ids = zone
        .children
        .into_iter()
        .map(|child| device_id_for_light(&child.rid, lights))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let external_id = grouped_light_id(&zone.services);

    Ok(Zone {
        id: zone.id,
        name: zone.metadata.name,
        group_ids: BTreeSet::new(), // Hue zones never contain rooms (groups)
        device_ids,
        revision: 0,
        owner: GroupOwner::Integration,
        external_id,
    })
}

fn device_id_for_light(light_id: &str, lights: &[LightGet]) -> Result<String, MapZonesError> {
    lights
        .iter()
        .find(|light| light.id == light_id)
        .map(|light| light.owner.rid.clone())
        .ok_or_else(|| MapZonesError::UnknownLight { light_id: light_id.to_string() })
}

#[derive(Error, Debug)]
pub enum MapZonesError {
    #[error("unknown light '{light_id}'")]
    UnknownLight { light_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{Metadata, On, Owner};
    use pretty_assertions::assert_eq;

    fn owner(rid: &str, rtype: &str) -> Owner {
        Owner { rid: rid.to_string(), rtype: rtype.to_string() }
    }

    fn light(id: &str, device_id: &str) -> LightGet {
        LightGet {
            id: id.to_string(),
            owner: owner(device_id, "device"),
            on: On { on: false },
            dimming: None,
            color_temperature: None,
            color: None,
        }
    }

    fn zone(id: &str, name: &str, light_ids: &[&str]) -> ZoneGet {
        ZoneGet {
            id: id.to_string(),
            children: light_ids.iter().map(|rid| owner(rid, "light")).collect(),
            services: vec![owner("grouped-light", "grouped_light")],
            metadata: Metadata { name: name.to_string(), archetype: "downstairs".to_string() },
        }
    }

    fn ids(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn map_zones_maps_a_zone_to_an_integration_owned_zone() {
        let lights = vec![light("light-1", "device-1"), light("light-2", "device-2")];

        let zones = map_zones(vec![zone("zone-1", "Downstairs", &["light-1", "light-2"])], &lights).unwrap();

        assert_eq!(
            zones,
            vec![Zone {
                id: "zone-1".to_string(),
                name: "Downstairs".to_string(),
                group_ids: BTreeSet::new(),
                device_ids: ids(&["device-1", "device-2"]),
                revision: 0,
                owner: GroupOwner::Integration,
                external_id: Some("grouped-light".to_string()),
            }]
        );
    }

    #[test]
    fn map_zones_maps_the_children_from_light_ids_to_device_ids() {
        let lights = vec![light("light-1", "device-1")];

        let zones = map_zones(vec![zone("zone-1", "Downstairs", &["light-1"])], &lights).unwrap();

        assert_eq!(zones[0].device_ids, ids(&["device-1"]));
    }

    #[test]
    fn map_zones_leaves_the_external_id_empty_for_a_zone_without_a_grouped_light() {
        let mut zone = zone("zone-1", "Downstairs", &["light-1"]);
        zone.services = vec![owner("motion-service", "motion")];

        let zones = map_zones(vec![zone], &[light("light-1", "device-1")]).unwrap();

        assert_eq!(zones[0].external_id, None);
    }

    #[test]
    fn map_zones_deduplicates_lights_of_the_same_device() {
        let lights = vec![light("light-1", "device"), light("light-2", "device")];

        let zones = map_zones(vec![zone("zone-1", "Downstairs", &["light-1", "light-2"])], &lights).unwrap();

        assert_eq!(zones[0].device_ids, ids(&["device"]));
    }

    #[test]
    fn map_zones_lets_zones_overlap() {
        let lights = vec![light("light-1", "device-1"), light("light-2", "device-2")];
        let zones = vec![zone("zone-1", "First", &["light-1", "light-2"]), zone("zone-2", "Second", &["light-2"])];

        let zones = map_zones(zones, &lights).unwrap();

        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].device_ids, ids(&["device-1", "device-2"]));
        assert_eq!(zones[1].device_ids, ids(&["device-2"]));
    }

    #[test]
    fn map_zones_skips_a_zone_without_children() {
        let lights = vec![light("light-1", "device-1")];
        let zones = vec![zone("empty", "Empty", &[]), zone("zone-1", "Downstairs", &["light-1"])];

        let zones = map_zones(zones, &lights).unwrap();

        assert_eq!(zones.iter().map(|zone| zone.id.as_str()).collect::<Vec<_>>(), vec!["zone-1"]);
    }

    #[test]
    fn map_zones_returns_an_error_for_an_unknown_light() {
        let lights = vec![light("light-1", "device-1")];

        let result = map_zones(vec![zone("zone-1", "Downstairs", &["light-1", "missing"])], &lights);

        assert!(
            matches!(&result, Err(MapZonesError::UnknownLight { light_id }) if light_id == "missing"),
            "got {result:?}"
        );
    }

    #[test]
    fn map_zones_returns_an_error_when_any_zone_has_an_unknown_light() {
        let lights = vec![light("light-1", "device-1")];
        let zones = vec![zone("zone-1", "Fine", &["light-1"]), zone("zone-2", "Broken", &["missing"])];

        assert!(map_zones(zones, &lights).is_err());
    }

    #[test]
    fn map_zones_returns_nothing_without_zones() {
        assert!(map_zones(vec![], &[light("light-1", "device-1")]).unwrap().is_empty());
    }

    #[test]
    fn unknown_light_error_names_the_light() {
        let error = MapZonesError::UnknownLight { light_id: "missing".to_string() };

        assert_eq!(error.to_string(), "unknown light 'missing'");
    }
}
