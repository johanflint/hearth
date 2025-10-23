use crate::flow_engine::WeekdayCondition;

#[derive(Clone, PartialEq, Debug)]
pub enum Schedule {
    Cron(String),
    Sunrise { when: WeekdayCondition, offset: i64 },
    Sunset { when: WeekdayCondition, offset: i64 },
}
