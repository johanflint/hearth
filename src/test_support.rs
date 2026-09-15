use crate::domain::device::{Device, DeviceType};
use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, Gamut, Property, PropertyType};
use std::collections::HashMap;

pub(crate) struct DeviceBuilder {
    id: String,
    properties: HashMap<String, Box<dyn Property>>,
}

impl DeviceBuilder {
    pub(crate) fn new(id: &str) -> Self {
        DeviceBuilder { id: id.to_string(), properties: HashMap::new() }
    }

    pub(crate) fn with_boolean_property(mut self, name: &str, value: bool) -> Self {
        let property: Box<dyn Property> = Box::new(BooleanProperty::new(name.to_string(), PropertyType::On, false, None, value));
        self.properties.insert(name.to_string(), property);
        self
    }

    pub(crate) fn with_color_property(mut self, name: &str, xy: CartesianCoordinate, gamut: Option<Gamut>) -> Self {
        let property: Box<dyn Property> = Box::new(ColorProperty::new(name.to_string(), PropertyType::Color,false, None, xy, gamut));
        self.properties.insert(name.to_string(), property);
        self
    }

    pub(crate) fn with_properties(mut self, properties: Vec<Box<dyn Property>>) -> Self {
        self.properties.extend(properties.into_iter().map(|p| (p.name().to_string(), p)));
        self
    }

    pub(crate) fn build(self) -> Device {
        Device {
            id: self.id,
            r#type: DeviceType::Light,
            manufacturer: "Signify Netherlands B.V.".to_string(),
            model_id: "Test Model".to_string(),
            product_name: "Hue color lamp".to_string(),
            name: "Test Device".to_string(),
            properties: self.properties,
            external_id: None,
            address: None,
            controller_id: None,
        }
    }
}
