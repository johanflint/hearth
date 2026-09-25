use crate::hue::domain::Owner;
use serde::Deserialize;

// API: https://developers.meethue.com/develop/hue-api-v2/api-reference/#resource_zigbee_connectivity_get
#[derive(Debug, Deserialize)]
pub struct ZigbeeConnectivityGet {
    pub id: String,
    pub owner: Owner,
    pub status: String, // One of connected, disconnected, connectivity_issue, unidirectional_incoming
}
