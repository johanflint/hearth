use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DeviceGet {
    pub id: String,
}
