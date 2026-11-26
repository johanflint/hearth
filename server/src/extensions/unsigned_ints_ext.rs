/// A trait to convert values from Mirek to Kelvin and vice versa.
/// See https://en.wikipedia.org/wiki/Mired.
pub trait MirekConversions {
    /// Returns the value in Kelvin by treating `self` as the value in mirek.
    fn mirek_to_kelvin(self) -> Self;

    /// Returns the value in mirek by treating `self` as the value in Kelvin.
    fn kelvin_to_mirek(self) -> Self;
}

macro_rules! impl_mirek_conversions {
    ($($t:ty)*) => ($(
        impl MirekConversions for $t {
            fn mirek_to_kelvin(self) -> $t {
                1_000_000 / self
            }
            fn kelvin_to_mirek(self) -> $t {
                1_000_000 / self
            }
        }
    )*)
}

impl_mirek_conversions! { u32 u64 u128 usize }

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(153, 6535)]
    #[case(500, 2000)]
    fn mirek_to_kelvin_u32(#[case] input: u32, #[case] expected: u32) {
        assert_eq!(input.mirek_to_kelvin(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(153, 6535)]
    #[case(500, 2000)]
    fn mirek_to_kelvin_u64(#[case] input: u64, #[case] expected: u64) {
        assert_eq!(input.mirek_to_kelvin(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(153, 6535)]
    #[case(500, 2000)]
    fn mirek_to_kelvin_u128(#[case] input: u128, #[case] expected: u128) {
        assert_eq!(input.mirek_to_kelvin(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(153, 6535)]
    #[case(500, 2000)]
    fn mirek_to_kelvin_usize(#[case] input: usize, #[case] expected: usize) {
        assert_eq!(input.mirek_to_kelvin(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(6535, 153)]
    #[case(2000, 500)]
    fn kelvin_to_mirek_u32(#[case] input: u32, #[case] expected: u32) {
        assert_eq!(input.kelvin_to_mirek(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(6535, 153)]
    #[case(2000, 500)]
    fn kelvin_to_mirek_u64(#[case] input: u64, #[case] expected: u64) {
        assert_eq!(input.kelvin_to_mirek(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(6535, 153)]
    #[case(2000, 500)]
    fn kelvin_to_mirek_u128(#[case] input: u128, #[case] expected: u128) {
        assert_eq!(input.kelvin_to_mirek(), expected);
    }

    #[rstest]
    #[should_panic]
    #[case(0, 0)]
    #[case(6535, 153)]
    #[case(2000, 500)]
    fn kelvin_to_mirek_usize(#[case] input: usize, #[case] expected: usize) {
        assert_eq!(input.kelvin_to_mirek(), expected);
    }
}
