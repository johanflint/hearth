use crate::domain::Owner;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum OwnerView {
    Hearth,
    Integration,
}

impl From<&Owner> for OwnerView {
    fn from(owner: &Owner) -> Self {
        match owner {
            Owner::Hearth => OwnerView::Hearth,
            Owner::Integration => OwnerView::Integration,
        }
    }
}
