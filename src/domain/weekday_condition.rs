use crate::domain::Weekday;
use crate::domain::Weekday::{Friday, Monday, Saturday, Sunday, Thursday, Tuesday, Wednesday};

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
