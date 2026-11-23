use crate::domain::ClaimMode;
use serde::de::Error;
use serde::{Deserialize, Deserializer};

impl<'de> Deserialize<'de> for ClaimMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;

        match value.as_str() {
            "claim" => Ok(Self::Claim),
            "release" => Ok(Self::Release),
            _ => Err(D::Error::unknown_variant(&value, &["claim", "release"])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::{Value, json};

    #[rstest]
    #[case::claim(json!("claim"), ClaimMode::Claim)]
    #[case::release(json!("release"), ClaimMode::Release)]
    fn deserializes_a_valid_claim(#[case] json: Value, #[case] expected: ClaimMode) {
        let parsed: ClaimMode = serde_json::from_value(json!(json)).unwrap();
        assert_eq!(parsed, expected);
    }

    #[rstest]
    #[case::invalid_claim_value(
        json!("invalid"),
        "unknown variant `invalid`, expected `claim` or `release`"
    )]
    fn deserialize_fails_for_invalid_cases(#[case] json: Value, #[case] expected_message: &str) {
        let parsed: Result<ClaimMode, _> = serde_json::from_value(json);
        let err = parsed.expect_err("expected an error but got Ok");
        let msg = err.to_string();
        assert!(
            msg.contains(expected_message),
            "Expected error message to contain '{expected_message}', but got '{msg}'"
        );
    }
}
