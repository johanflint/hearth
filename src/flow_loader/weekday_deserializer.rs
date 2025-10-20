use crate::flow_engine::Weekday;
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for Weekday {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        match value.to_lowercase().as_str() {
            "mon" | "monday" => Ok(Weekday::Monday),
            "tue" | "tuesday" => Ok(Weekday::Tuesday),
            "wed" | "wednesday" => Ok(Weekday::Wednesday),
            "thu" | "thursday" => Ok(Weekday::Thursday),
            "fri" | "friday" => Ok(Weekday::Friday),
            "sat" | "saturday" => Ok(Weekday::Saturday),
            "sun" | "sunday" => Ok(Weekday::Sunday),
            _ => Err(serde::de::Error::custom(format!("invalid weekday: {}", value))),
        }
    }
}
