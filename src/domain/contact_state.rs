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
}
