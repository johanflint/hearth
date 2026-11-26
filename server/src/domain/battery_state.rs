use std::fmt;
use strum::{EnumIter, EnumString};

#[derive(EnumIter, EnumString, Debug, Clone, PartialEq)]
#[strum(serialize_all = "lowercase")]
pub enum BatteryState {
    Normal,
    Low,
    Critical,
}

impl BatteryState {
    pub const fn from_percent(percent: u64) -> BatteryState {
        match percent {
            0..=1 => BatteryState::Critical,
            2..=19 => BatteryState::Low,
            _ => BatteryState::Normal,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            BatteryState::Normal => "normal",
            BatteryState::Low => "low",
            BatteryState::Critical => "critical",
        }
    }
}

impl fmt::Display for BatteryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::str::FromStr;
    use strum::IntoEnumIterator;

    #[rstest]
    #[case::empty(0, BatteryState::Critical)]
    #[case::almost_empty(1, BatteryState::Critical)]
    #[case::critical_low(2, BatteryState::Low)]
    #[case::critical_high(19, BatteryState::Low)]
    #[case::normal_low(20, BatteryState::Normal)]
    #[case::full(100, BatteryState::Normal)]
    fn from_percent(#[case] percent: u64, #[case] expected: BatteryState) {
        assert_eq!(BatteryState::from_percent(percent), expected);
    }

    #[rstest]
    #[case::normal(BatteryState::Normal, "normal")]
    #[case::low(BatteryState::Low, "low")]
    #[case::critical(BatteryState::Critical, "critical")]
    fn display(#[case] state: BatteryState, #[case] expected: &str) {
        assert_eq!(state.to_string(), expected);
    }

    #[test]
    fn from_str_is_the_inverse_of_display() {
        for state in BatteryState::iter() {
            assert_eq!(BatteryState::from_str(&state.to_string()), Ok(state));
        }
    }

    #[test]
    fn from_str_rejects_unknown_values() {
        assert!(BatteryState::from_str("empty").is_err());
    }
}
