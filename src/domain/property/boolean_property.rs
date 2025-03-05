use crate::domain::property::{Property, PropertyType};
use std::any::Any;

#[derive(PartialEq, Debug)]
pub struct BooleanProperty {
    name: String,
    property_type: PropertyType,
    readonly: bool,
    external_id: Option<String>,
    value: bool,
}

impl BooleanProperty {
    pub fn new(name: String, property_type: PropertyType, readonly: bool, external_id: Option<String>, value: bool) -> Self {
        BooleanProperty {
            name,
            property_type,
            readonly,
            external_id,
            value,
        }
    }
}

impl Property for BooleanProperty {
    fn name(&self) -> &str {
        &self.name
    }

    fn property_type(&self) -> PropertyType {
        self.property_type
    }

    fn readonly(&self) -> bool {
        self.readonly
    }

    fn external_id(&self) -> Option<&str> {
        self.external_id.as_deref()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn eq_dyn(&self, other: &dyn Property) -> bool {
        other.as_any().downcast_ref::<BooleanProperty>().map_or(false, |o| self == o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_boolean_property() {
        let property = BooleanProperty {
            name: "on".to_string(),
            property_type: PropertyType::On,
            readonly: false,
            external_id: Some("lol".to_string()),
            value: true,
        };

        let mut properties: HashMap<String, Box<dyn Property>> = HashMap::new();
        properties.insert(property.name().to_string(), Box::new(property));

        if let Some(property) = properties.get_mut("on") {
            if let Some(boolean_property) = property.as_any_mut().downcast_mut::<BooleanProperty>() {
                boolean_property.value = false;
            }
        }
    }
}
