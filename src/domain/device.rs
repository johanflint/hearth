#[derive(Debug)]
pub struct Device {
    pub id: String,
}

#[derive(PartialEq, Debug)]
pub enum DeviceType {
    Light,
}
