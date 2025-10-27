use crate::domain::Time;
use serde::de::{Error, Unexpected};
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for Time {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        let parts: Vec<&str> = value.split(':').collect();
        if parts.len() != 2 {
            return Err(Error::invalid_value(Unexpected::Str(&value), &"a time in HH:MM format"));
        }

        let hour: u8 = parts[0]
            .parse()
            .map_err(|_| Error::invalid_value(Unexpected::Str(parts[0]), &"a valid hour between 0 and 23"))?;

        if hour > 23 {
            return Err(Error::invalid_value(Unexpected::Str(parts[0]), &"a valid hour between 0 and 23"));
        }

        if parts[1].len() != 2 {
            return Err(Error::invalid_value(Unexpected::Str(parts[1]), &"a valid minute between 0 and 59"));
        }

        let minute: u8 = parts[1]
            .parse()
            .map_err(|_| Error::invalid_value(Unexpected::Str(parts[0]), &"a valid minute between 0 and 59"))?;

        if minute > 59 {
            return Err(Error::invalid_value(Unexpected::Str(parts[0]), &"a valid minute between 0 and 59"));
        }

        Ok(Time { hour, minute })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case("0:00", Time { hour: 0, minute: 0 })]
    #[case("00:00", Time { hour: 0, minute: 0 })]
    #[case("20:00", Time { hour: 20, minute:0 })]
    #[case("23:59", Time { hour: 23, minute: 59 })]
    fn deserializes_valid_time(#[case] time: &str, #[case] expected: Time) {
        let result = serde_json::from_value::<Time>(json!(time)).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::missing_colon("2200")]
    #[case::missing_colon_and_minutes("20")]
    #[case::missing_minutes("20:")]
    #[case::invalid_hour("a0:00")]
    #[case::hour_too_large("24:00")]
    #[case::invalid_minutes("0:a")]
    #[case::minutes_too_large("0:60")]
    #[case::minutes_too_large("23:59:00")]
    #[case::minutes_too_short("23:5")]
    fn fails_for_an_invalid_time(#[case] time: &str) {
        let result = serde_json::from_value::<Time>(json!(time));
        assert!(result.is_err());
    }
}
