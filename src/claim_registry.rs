use crate::domain::ClaimMode;
use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};

static CLAIM_REGISTRY: LazyLock<ClaimRegistry> = LazyLock::new(ClaimRegistry::default);

#[derive(Default, Debug)]
struct ClaimRegistry {
    claimed: RwLock<HashSet<String>>,
}

impl ClaimRegistry {
    /// Applies the claim intent and returns whether any writes may be sent.
    fn authorize(&self, device_id: &str, claim: Option<ClaimMode>) -> bool {
        let mut registry = self.claimed.write().unwrap();
        match claim {
            Some(ClaimMode::Claim) => {
                registry.insert(device_id.to_string());
                true
            }
            Some(ClaimMode::Release) => {
                registry.remove(device_id);
                true
            }
            None => !registry.contains(device_id),
        }
    }
}

pub fn authorize(device_id: &str, claim: Option<ClaimMode>) -> bool {
    CLAIM_REGISTRY.authorize(device_id, claim)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const DEVICE_ID: &str = "device_id";
    const DEVICE2_ID: &str = "device2_id";

    fn is_claimed(registry: &ClaimRegistry, device_id: &str) -> bool {
        registry.claimed.read().unwrap().contains(device_id)
    }

    #[rstest]
    #[case::unclaimed_without_claim(false, None, true, false)]
    #[case::claimed_without_claim(true, None, false, true)]
    #[case::unclaimed_claim(false, Some(ClaimMode::Claim), true, true)]
    #[case::claimed_claim(true, Some(ClaimMode::Claim), true, true)]
    #[case::unclaimed_release(false, Some(ClaimMode::Release), true, false)]
    #[case::claimed_release(true, Some(ClaimMode::Release), true, false)]
    fn authorize(#[case] claimed: bool, #[case] claim: Option<ClaimMode>, #[case] expected_authorized: bool, #[case] expected_claimed: bool) {
        let registry = ClaimRegistry::default();
        if claimed {
            registry.authorize(DEVICE_ID, Some(ClaimMode::Claim));
        }
        assert_eq!(registry.authorize(DEVICE_ID, claim), expected_authorized);
        assert_eq!(is_claimed(&registry, DEVICE_ID), expected_claimed);
    }

    #[test]
    fn authorize_claims_per_device() {
        let registry = ClaimRegistry::default();
        registry.authorize(DEVICE_ID, Some(ClaimMode::Claim));

        assert!(registry.authorize(DEVICE2_ID, None));
        assert!(!registry.authorize(DEVICE_ID, None));
    }
}