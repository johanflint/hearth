/// Converts the logarithmic light level of Hue (`10000 * log10(lux)  1`) to lux.
/// A light level of 0 means no light at all, as the formula would otherwise give 0.79 lux.
pub fn light_level_to_lux(light_level: u64) -> f64 {
    match light_level {
        0 => 0.0,
        level => 10f64.powf((level as f64 - 1.0) / 10_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::no_light(0, 0.0)]
    #[case::one_lux(1, 1.0)]
    #[case::ten_lux(10_001, 10.0)]
    #[case::thousand_lux(30_001, 1_000.0)]
    fn light_level_to_lux_converts_the_logarithmic_scale(#[case] light_level: u64, #[case] expected: f64) {
        assert!((light_level_to_lux(light_level) - expected).abs() < 1e-9);
    }
}
