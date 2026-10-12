use crate::domain::ClaimMode;
use std::collections::HashMap;
use std::fmt;
use std::fmt::Formatter;
use std::sync::{LazyLock, RwLock};

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
    #[allow(dead_code)]
    External,
}

#[derive(Default, Debug)]
struct ClaimRegistry {
    claims: RwLock<HashMap<String, ClaimOrigin>>,
}

impl ClaimRegistry {
    /// Applies the claim intent and returns whether any writes may be sent.
    fn authorize(&self, device_id: &str, claim: Option<Claim>) -> Result<(), ClaimOrigin> {
        let mut claims = self.claims.write().unwrap();
        match claim {
            Some(Claim { mode: ClaimMode::Claim, origin }) => {
                claims.insert(device_id.to_string(), origin);
                Ok(())
            }
            Some(Claim { mode: ClaimMode::Release, .. }) => {
                claims.remove(device_id);
                Ok(())
            }
            None => match claims.get(device_id) {
                Some(origin) => Err(origin.clone()),
                None => Ok(())
            },
        }
    }
}

pub fn authorize(device_id: &str, claim: Option<Claim>) -> Result<(), ClaimOrigin> {
    CLAIM_REGISTRY.authorize(device_id, claim)
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
    use rstest::rstest;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn flows(flow_ids: &[&str]) -> ClaimOrigin {
        ClaimOrigin::Flows(flow_ids.iter().map(|id| id.to_string()).collect())
    }

    fn claim(mode: ClaimMode, flow_ids: &[&str]) -> Claim {
        Claim { mode, origin: flows(flow_ids) }
    }

    fn is_claimed(registry: &ClaimRegistry, device_id: &str) -> bool {
        registry.claims.read().unwrap().contains_key(device_id)
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
        if claimed {
            registry.authorize(DEVICE_ID, Some(claim(ClaimMode::Claim, &["flow"]))).unwrap();
        }
        assert_eq!(registry.authorize(DEVICE_ID, mode.map(|mode| claim(mode, &["flow"]))).is_ok(), expected_authorized);
        assert_eq!(is_claimed(&registry, DEVICE_ID), expected_claimed);
    }

    #[test]
    fn authorize_claims_per_device() {
        let registry = ClaimRegistry::default();
        registry.authorize(DEVICE_ID, Some(claim(ClaimMode::Claim, &["flow"]))).unwrap();

        assert_eq!(registry.authorize(DEVICE2_ID, None), Ok(()));
        assert_eq!(registry.authorize(DEVICE_ID, None), Err(flows(&["flow"])));
    }

    #[test]
    fn authorize_returns_the_origin_of_the_blocking_claim() {
        let registry = ClaimRegistry::default();
        registry.authorize(DEVICE_ID, Some(claim(ClaimMode::Claim, &["dinner", "movie"]))).unwrap();

        assert_eq!(registry.authorize(DEVICE_ID, None), Err(flows(&["dinner", "movie"])));
    }

    #[test]
    fn authorize_a_new_claim_replaces_the_origin() {
        let registry = ClaimRegistry::default();
        registry.authorize(DEVICE_ID, Some(claim(ClaimMode::Claim, &["dinner"]))).unwrap();
        registry.authorize(DEVICE_ID, Some(claim(ClaimMode::Claim, &["movie"]))).unwrap();

        assert_eq!(registry.authorize(DEVICE_ID, None), Err(flows(&["movie"])));
    }

    #[rstest]
    #[case::single_flow(flows(&["dinner"]), "flow 'dinner'")]
    #[case::multiple_flows(flows(&["dinner", "movie"]), "flows 'dinner', 'movie'")]
    #[case::external(ClaimOrigin::External, "an external change")]
    fn claim_origin_display(#[case] origin: ClaimOrigin, #[case] expected: &str) {
        assert_eq!(origin.to_string(), expected);
    }
}
