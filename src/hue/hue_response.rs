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

#[derive(Debug, Deserialize)]
pub struct Owner {
    pub rid: String,
    pub rtype: String,
}
