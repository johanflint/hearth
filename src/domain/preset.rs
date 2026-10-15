use crate::domain::Value;

#[derive(Debug)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub value: Value,
}
