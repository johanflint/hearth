use crate::domain::GeoLocation;
use serde::de::Error;
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for GeoLocation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Debug, Deserialize)]
        pub struct Inner {
            latitude: f64,
            longitude: f64,
            altitude_m: f64,
        }

        let inner = Inner::deserialize(deserializer)?;
        if !(inner.latitude >= -90.0 && inner.latitude <= 90.0) {
            return Err(Error::custom(format!("invalid location latitude: {}, must be between -90 and 90", inner.latitude)));
        }

        if !(inner.longitude >= -180.0 && inner.longitude <= 180.0) {
            return Err(Error::custom(format!("invalid location longitude: {}, must be between -180 and 180", inner.latitude)));
        }

        Ok(GeoLocation {
            latitude: inner.latitude,
            longitude: inner.longitude,
            altitude: inner.altitude_m,
        })
    }
}
