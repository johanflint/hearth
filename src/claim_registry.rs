use crate::domain::ClaimMode;
use crate::domain::device::Device;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Formatter;
use std::sync::{LazyLock, RwLock};
use thiserror::Error;

static CLAIM_REGISTRY: LazyLock<ClaimRegistry> = LazyLock::new(ClaimRegistry::default);

#[derive(PartialEq, Debug)]
pub struct Claim {
    pub mode: ClaimMode,
    pub origin: ClaimOrigin,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ClaimOrigin {
    // Sorted ids of the flows that agreed on the claim
    Flows(Vec<String>),
    // A change made outside Hearth
    External,
}

#[derive(Debug, PartialEq, Error)]
pub enum AuthorizeError {
    #[error("claimed by {0}")]
    Claimed(ClaimOrigin),
    #[error(transparent)]
    NotClaimable(#[from] NotClaimable),
}

#[derive(Debug, PartialEq, Error)]
#[error("device is not claimable")]
pub struct NotClaimable;

#[derive(Default, Debug)]
struct ClaimRegistry {
    claims: RwLock<HashMap<String, ClaimOrigin>>,
}

impl ClaimRegistry {
    fn claim(&self, device: &Device, origin: ClaimOrigin) -> Result<(), NotClaimable> {
        ensure_claimable(device)?;
        self.claims.write().unwrap().insert(device.id.clone(), origin);
        Ok(())
    }

    fn release(&self, device: &Device) -> Result<(), NotClaimable> {
        ensure_claimable(device)?;
        self.claims.write().unwrap().remove(&device.id);
        Ok(())
    }

    fn claimed_by(&self, device_id: &str) -> Option<ClaimOrigin> {
        self.claims.read().unwrap().get(device_id).cloned()
    }

    /// Applies the claim intent. Ok means writes may be sent, Err means they may not.
    fn authorize(&self, device: &Device, claim: Option<Claim>) -> Result<(), AuthorizeError> {
        match claim {
            Some(Claim { mode: ClaimMode::Claim, origin }) => Ok(self.claim(device, origin)?),
            Some(Claim { mode: ClaimMode::Release, .. }) => Ok(self.release(device)?),
            None => match self.claimed_by(&device.id) {
                Some(origin) => Err(AuthorizeError::Claimed(origin)),
                None => Ok(())
            },
        }
    }
}

fn ensure_claimable(device: &Device) -> Result<(), NotClaimable> {
    device.claimable.then_some(()).ok_or(NotClaimable)
}

pub fn claim(device: &Device, origin: ClaimOrigin) -> Result<(), NotClaimable> {
    CLAIM_REGISTRY.claim(device, origin)
}

pub fn release(device: &Device) -> Result<(), NotClaimable> {
    CLAIM_REGISTRY.release(device)
}

pub fn authorize(device: &Device, claim: Option<Claim>) -> Result<(), AuthorizeError> {
    CLAIM_REGISTRY.authorize(device, claim)
}

impl fmt::Display for ClaimOrigin {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ClaimOrigin::Flows(flow_ids) => {
                let noun = if flow_ids.len() == 1 { "flow" } else { "flows" };
                let flow_ids: Vec<String> = flow_ids.iter().map(|id| format!("'{id}'")).collect();
                write!(f, "{} {}", noun, flow_ids.join(", "))
            }
            ClaimOrigin::External => write!(f, "an external change"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::DeviceBuilder;
    use rstest::rstest;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn device(device_id: &str) -> Device {
        DeviceBuilder::new(device_id).build()
    }

    fn unclaimable_device(device_id: &str) -> Device {
        Device { claimable: false, ..device(device_id) }
    }

    fn flows(flow_ids: &[&str]) -> ClaimOrigin {
        ClaimOrigin::Flows(flow_ids.iter().map(|id| id.to_string()).collect())
    }

    fn claim_intent(mode: ClaimMode, flow_ids: &[&str]) -> Claim {
        Claim { mode, origin: flows(flow_ids) }
    }

    #[rstest]
    #[case::unclaimed_without_claim(false, None, true, false)]
    #[case::claimed_without_claim(true, None, false, true)]
    #[case::unclaimed_claim(false, Some(ClaimMode::Claim), true, true)]
    #[case::claimed_claim(true, Some(ClaimMode::Claim), true, true)]
    #[case::unclaimed_release(false, Some(ClaimMode::Release), true, false)]
    #[case::claimed_release(true, Some(ClaimMode::Release), true, false)]
    fn authorize(#[case] claimed: bool, #[case] mode: Option<ClaimMode>, #[case] expected_authorized: bool, #[case] expected_claimed: bool) {
        let registry = ClaimRegistry::default();
        let device = device(DEVICE_ID);
        if claimed {
            registry.claim(&device, flows(&["flow"])).unwrap();
        }
        assert_eq!(registry.authorize(&device, mode.map(|mode| claim_intent(mode, &["flow"]))).is_ok(), expected_authorized);
        assert_eq!(registry.claimed_by(DEVICE_ID).is_some(), expected_claimed);
    }

    #[test]
    fn authorize_claims_per_device() {
        let registry = ClaimRegistry::default();
        registry.claim(&device(DEVICE_ID), flows(&["flow"])).unwrap();

        assert_eq!(registry.authorize(&device(DEVICE2_ID), None), Ok(()));
        assert_eq!(registry.authorize(&device(DEVICE_ID), None), Err(AuthorizeError::Claimed(flows(&["flow"]))));
    }

    #[test]
    fn authorize_returns_the_origin_of_the_blocking_claim() {
        let registry = ClaimRegistry::default();
        registry.claim(&device(DEVICE_ID), flows(&["dinner", "movie"])).unwrap();

        assert_eq!(registry.authorize(&device(DEVICE_ID), None), Err(AuthorizeError::Claimed(flows(&["dinner", "movie"]))));
    }

    #[test]
    fn authorize_a_new_claim_replaces_the_origin() {
        let registry = ClaimRegistry::default();
        let device = device(DEVICE_ID);
        registry.authorize(&device, Some(claim_intent(ClaimMode::Claim, &["dinner"]))).unwrap();
        registry.authorize(&device, Some(claim_intent(ClaimMode::Claim, &["movie"]))).unwrap();

        assert_eq!(registry.claimed_by(DEVICE_ID), Some(flows(&["movie"])));
    }

    #[rstest]
    #[case::claim(ClaimMode::Claim)]
    #[case::release(ClaimMode::Release)]
    fn authorize_rejects_a_claim_intent_for_a_device_that_is_not_claimable(#[case] mode: ClaimMode) {
        let registry = ClaimRegistry::default();

        assert_eq!(registry.authorize(&unclaimable_device(DEVICE_ID), Some(claim_intent(mode, &["flow"]))), Err(AuthorizeError::NotClaimable(NotClaimable)));
        assert_eq!(registry.claimed_by(DEVICE_ID), None);
    }

    #[test]
    fn authorize_without_claim_allows_writes_to_a_device_that_is_not_claimable() {
        let registry = ClaimRegistry::default();

        assert_eq!(registry.authorize(&unclaimable_device(DEVICE_ID), None), Ok(()));
    }

    #[test]
    fn claim_registers_the_origin_and_release_removes_it() {
        let registry = ClaimRegistry::default();
        let device = device(DEVICE_ID);

        registry.claim(&device, ClaimOrigin::External).unwrap();
        assert_eq!(registry.claimed_by(DEVICE_ID), Some(ClaimOrigin::External));

        registry.release(&device).unwrap();
        assert_eq!(registry.claimed_by(DEVICE_ID), None);
    }

    #[test]
    fn claim_and_release_reject_a_device_that_is_not_claimable() {
        let registry = ClaimRegistry::default();
        let device = unclaimable_device(DEVICE_ID);

        assert_eq!(registry.claim(&device, ClaimOrigin::External), Err(NotClaimable));
        assert_eq!(registry.release(&device), Err(NotClaimable));
        assert_eq!(registry.claimed_by(DEVICE_ID), None);
    }

    #[rstest]
    #[case::single_flow(flows(&["dinner"]), "flow 'dinner'")]
    #[case::multiple_flows(flows(&["dinner", "movie"]), "flows 'dinner', 'movie'")]
    #[case::external(ClaimOrigin::External, "an external change")]
    fn claim_origin_display(#[case] origin: ClaimOrigin, #[case] expected: &str) {
        assert_eq!(origin.to_string(), expected);
    }
}
