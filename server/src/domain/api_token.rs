use sha2::{Digest, Sha256};

const PREFIX: &str = "hearth_";

pub struct Token(String);

impl Token {
    pub fn generate() -> Self {
        let secret: [u8; 32] = rand::random();
        Token(format!("{PREFIX}{}", hex::encode(secret)))
    }

    pub fn parse(raw: &str) -> Option<Self> {
        raw.starts_with(PREFIX).then(|| Token(raw.to_string()))
    }

    /// The hash is what gets stored and looked up. A plain SHA-256 is enough
    /// because the token is random, so there is nothing to brute force.
    pub fn hash(&self) -> String {
        hex::encode(Sha256::digest(self.0.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[test]
    fn parse_accepts_a_generated_token() {
        let token = Token::generate();

        let parsed = Token::parse(token.as_str()).expect("a generated token should parse");

        assert_eq!(parsed.hash(), token.hash());
    }

    #[rstest]
    #[case::empty("")]
    #[case::no_prefix("abc")]
    #[case::other_case_prefix("HEARTH_abc")]
    #[case::leading_space(" hearth_abc")]
    #[case::prefix_without_underscore("hearth")]
    fn parse_rejects_a_value_without_the_prefix(#[case] raw: &str) {
        assert!(Token::parse(raw).is_none());
    }

    #[test]
    fn generate_creates_a_prefixed_token_with_64_hex_characters() {
        let token = Token::generate();

        let secret = token.0.strip_prefix("hearth_").expect("token should have the hearth_ prefix");
        assert_eq!(secret.len(), 64);
        assert!(secret.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_creates_a_different_token_every_time() {
        assert_ne!(Token::generate().0, Token::generate().0);
    }

    #[test]
    fn hash_is_the_hex_sha256_of_the_token() {
        // SHA-256 of "abc", so a change of algorithm or encoding invalidates every stored token
        let token = Token("abc".to_string());

        assert_eq!(token.hash(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn as_str_returns_the_token_that_is_hashed() {
        let token = Token::generate();

        assert!(token.as_str().starts_with("hearth_"));
        assert_eq!(Token(token.as_str().to_string()).hash(), token.hash());
    }

    #[test]
    fn hash_is_stable_and_differs_from_the_token() {
        let token = Token::generate();

        assert_eq!(token.hash(), token.hash());
        assert_ne!(token.hash(), token.0);
    }
}
