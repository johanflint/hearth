use crate::domain::Weekday;
use crate::domain::Weekday::{Friday, Monday, Saturday, Sunday, Thursday, Tuesday, Wednesday};
use std::fmt::{Display, Formatter};

#[derive(Clone, PartialEq, Debug)]
pub enum WeekdayCondition {
    Specific(Weekday),
    Range { start: Weekday, end: Weekday },
    Set(Vec<Weekday>),
    Weekdays,
    Weekend,
}

impl WeekdayCondition {
    pub fn included_days(&self) -> Vec<Weekday> {
        match self {
            WeekdayCondition::Specific(day) => vec![day.clone()],
            WeekdayCondition::Range { start, end } => {
                let all = Weekday::all();
                let start_index = start.as_index();
                let end_index = end.as_index();

                all[start_index..=end_index].to_vec()
            }
            WeekdayCondition::Set(days) => days.clone(),
            WeekdayCondition::Weekdays => vec![Monday, Tuesday, Wednesday, Thursday, Friday],
            WeekdayCondition::Weekend => vec![Saturday, Sunday],
        }
    }
}

impl Display for WeekdayCondition {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            WeekdayCondition::Specific(day) => {
                write!(f, "{}", day)
            }
            WeekdayCondition::Range { start, end } => {
                write!(f, "{}-{}", start, end)
            }
            WeekdayCondition::Set(days) => {
                let s = days.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
                write!(f, "{}", s)
            }
            WeekdayCondition::Weekdays => {
                write!(f, "weekdays")
            }
            WeekdayCondition::Weekend => {
                write!(f, "weekend")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    #[rstest]
    #[case::monday(WeekdayCondition::Specific(Monday), "Monday")]
    #[case::tuesday(WeekdayCondition::Specific(Tuesday), "Tuesday")]
    #[case::wednesday(WeekdayCondition::Specific(Wednesday), "Wednesday")]
    #[case::thursday(WeekdayCondition::Specific(Thursday), "Thursday")]
    #[case::friday(WeekdayCondition::Specific(Friday), "Friday")]
    #[case::saturday(WeekdayCondition::Specific(Saturday), "Saturday")]
    #[case::sunday(WeekdayCondition::Specific(Sunday), "Sunday")]
    #[case::range(WeekdayCondition::Range { start: Monday, end: Wednesday }, "Monday-Wednesday")]
    #[case::set(WeekdayCondition::Set(vec![Tuesday, Wednesday, Friday]), "Tuesday, Wednesday, Friday")]
    #[case::weekdays(WeekdayCondition::Weekdays, "weekdays")]
    #[case::weekend(WeekdayCondition::Weekend, "weekend")]
    fn test_display(#[case] condition: WeekdayCondition, #[case] expected: &str) {
        assert_eq!(format!("{}", condition), expected);
    }
}
