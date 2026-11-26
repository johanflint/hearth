#[derive(Clone, Default, Debug)]
pub struct GeoLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: f64, // In meters
}
