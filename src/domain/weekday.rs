use crate::domain::Weekday::*;
use std::fmt::{Display, Formatter};

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

impl Display for Weekday {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Monday => "Monday",
            Tuesday => "Tuesday",
            Wednesday => "Wednesday",
            Thursday => "Thursday",
            Friday => "Friday",
            Saturday => "Saturday",
            Sunday => "Sunday",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    #[rstest]
    #[case(Monday, "Monday")]
    #[case(Tuesday, "Tuesday")]
    #[case(Wednesday, "Wednesday")]
    #[case(Thursday, "Thursday")]
    #[case(Friday, "Friday")]
    #[case(Saturday, "Saturday")]
    #[case(Sunday, "Sunday")]
    fn test_display(#[case] condition: Weekday, #[case] expected: &str) {
        assert_eq!(format!("{}", condition), expected);
    }
}
