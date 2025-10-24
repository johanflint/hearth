use crate::domain::Weekday::*;

#[derive(PartialEq, Eq, Hash, Debug, Clone)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub fn as_index(&self) -> usize {
        match self {
            Monday => 0,
            Tuesday => 1,
            Wednesday => 2,
            Thursday => 3,
            Friday => 4,
            Saturday => 5,
            Sunday => 6,
        }
    }

    pub fn all() -> [Weekday; 7] {
        [Monday, Tuesday, Wednesday, Thursday, Friday, Saturday, Sunday]
    }
}
