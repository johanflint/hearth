use crate::domain::property::Unit;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum UnitView {
    Percentage,
    Lux,
    DegreesCelsius,
    Kelvin,
    None,
}

impl From<&Unit> for UnitView {
    fn from(unit: &Unit) -> Self {
        match unit {
            Unit::Percentage => Self::Percentage,
            Unit::Lux => Self::Lux,
            Unit::DegreesCelsius => Self::DegreesCelsius,
            Unit::Kelvin => Self::Kelvin,
            Unit::None => Self::None,
        }
    }
}
