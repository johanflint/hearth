use strum::EnumIter;

#[derive(EnumIter, Debug, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
}
