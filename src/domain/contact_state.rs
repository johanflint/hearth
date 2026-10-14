use std::fmt;
use strum::EnumIter;

#[derive(EnumIter, Debug, PartialEq)]
pub enum ContactState {
    Open,
    Closed,
}

impl ContactState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            ContactState::Open => "open",
            ContactState::Closed => "closed",
        }
    }
}

impl fmt::Display for ContactState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::open(ContactState::Open, "open")]
    #[case::closed(ContactState::Closed, "closed")]
    fn as_str(#[case] state: ContactState, #[case] expected: &str) {
        assert_eq!(state.as_str(), expected);
    }

    #[rstest]
    #[case::open(ContactState::Open, "open")]
    #[case::closed(ContactState::Closed, "closed")]
    fn display(#[case] state: ContactState, #[case] expected: &str) {
        assert_eq!(state.to_string(), expected);
    }
}
