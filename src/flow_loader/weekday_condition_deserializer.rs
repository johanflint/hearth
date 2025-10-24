use crate::domain::Weekday;
use crate::flow_engine::WeekdayCondition;
use serde::de::{SeqAccess, Unexpected, Visitor};
use serde::{Deserialize, Deserializer, de};
use std::fmt::Formatter;

impl<'de> Deserialize<'de> for WeekdayCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct WeekdayConditionVisitor;

        impl<'de> Visitor<'de> for WeekdayConditionVisitor {
            type Value = WeekdayCondition;

            fn expecting(&self, formatter: &mut Formatter) -> std::fmt::Result {
                write!(formatter, "a string like 'Monday', 'Mon-Fri', 'weekend', 'weekday' or an array like ['Mon', 'Wed']")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                let value = v.trim().to_lowercase();
                match value.as_str() {
                    "weekdays" => Ok(WeekdayCondition::Weekdays),
                    "weekend" => Ok(WeekdayCondition::Weekend),
                    _ => {
                        // Handle "Mon-Fri" or a single "Mon"
                        if let Some((start, end)) = value.split_once('-') {
                            let start_day: Weekday =
                                serde_json::from_str(&format!("\"{}\"", start.trim())).map_err(|_| de::Error::invalid_value(Unexpected::Str(start), &"valid weekday"))?;
                            let end_day: Weekday =
                                serde_json::from_str(&format!("\"{}\"", end.trim())).map_err(|_| de::Error::invalid_value(Unexpected::Str(end), &"valid weekday"))?;

                            if start_day == end_day {
                                return Ok(WeekdayCondition::Specific(start_day));
                            }

                            if start_day.as_index() < end_day.as_index() {
                                Ok(WeekdayCondition::Range { start: start_day, end: end_day })
                            } else {
                                Ok(WeekdayCondition::Range { start: end_day, end: start_day })
                            }
                        } else {
                            let day: Weekday =
                                serde_json::from_str(&format!("\"{}\"", value.trim())).map_err(|_| de::Error::invalid_value(Unexpected::Str(&value), &"valid weekday"))?;
                            Ok(WeekdayCondition::Specific(day))
                        }
                    }
                }
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut seen = [false; 7]; // Track seen weekdays efficiently
                while let Some(weekday) = seq.next_element::<Weekday>()? {
                    seen[weekday.as_index()] = true;
                }

                let days: Vec<Weekday> = Weekday::all().iter().filter(|weekday| seen[weekday.as_index()]).cloned().collect();
                Ok(WeekdayCondition::Set(days))
            }
        }

        deserializer.deserialize_any(WeekdayConditionVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Weekday::*;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    #[case::full_name("Monday", Monday)]
    #[case::abbreviated("Wed", Wednesday)]
    #[case::lowercase("friday", Friday)]
    #[case::uppercase("SATURDAY", Saturday)]
    fn deserialize_specific_weekday(#[case] json: &str, #[case] expected: Weekday) {
        let result = serde_json::from_value::<WeekdayCondition>(json!(json)).unwrap();
        assert_eq!(result, WeekdayCondition::Specific(expected));
    }

    #[rstest]
    #[case::full_names("Monday-Friday", WeekdayCondition::Range { start: Monday, end: Friday })]
    #[case::abbreviated("mon-fri", WeekdayCondition::Range { start: Monday, end: Friday })]
    #[case::full_and_abbreviated("Wednesday-fri", WeekdayCondition::Range { start: Wednesday, end: Friday })]
    #[case::extra_spaces("Wednesday - fri", WeekdayCondition::Range { start: Wednesday, end: Friday })]
    #[case::same_days("Wed-Wed", WeekdayCondition::Specific(Wednesday))]
    #[case::not_chronologically_ordered("Wed-Mon", WeekdayCondition::Range { start: Monday, end: Wednesday })]
    fn deserialize_range(#[case] json: &str, #[case] expected: WeekdayCondition) {
        let result = serde_json::from_value::<WeekdayCondition>(json!(json)).unwrap();
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::two_days(vec!["Mon", "Wed"], WeekdayCondition::Set(vec![Monday, Wednesday]))]
    #[case::duplicates(vec!["Mon", "Wed", "Wed"], WeekdayCondition::Set(vec![Monday, Wednesday]))]
    fn deserialize_set(#[case] json: Vec<&str>, #[case] expected: WeekdayCondition) {
        let result = serde_json::from_value::<WeekdayCondition>(json!(json)).unwrap();
        assert_eq!(result, expected);
    }

    #[test]
    fn deserialize_weekdays() {
        let result = serde_json::from_value::<WeekdayCondition>(json!("Weekdays")).unwrap();
        assert_eq!(result, WeekdayCondition::Weekdays);
    }

    #[test]
    fn deserialize_weekend() {
        let result = serde_json::from_value::<WeekdayCondition>(json!("Weekend")).unwrap();
        assert_eq!(result, WeekdayCondition::Weekend);
    }
}
