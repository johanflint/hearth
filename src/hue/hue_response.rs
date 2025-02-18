use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct HueResponse<T> {
    pub data: Vec<T>,
    pub errors: Vec<HueError>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct HueError {
    pub description: String,
}
