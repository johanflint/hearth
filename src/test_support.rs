use crate::domain::property::{BooleanProperty, CartesianCoordinate, ColorProperty, Gamut, Property, PropertyType};
use crate::domain::{Device, DeviceType, Value};
use crate::flow_engine::Expression::{self, Literal};
use crate::flow_engine::flow::{ActionFlowNode, Flow, FlowLink, FlowNode, FlowNodeKind};
use crate::flow_engine::property_command::{PropertyCommand, ResolvedPropertyCommand};
use crate::flow_engine::{ControlGroupAction, VersionedFlow};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct DeviceBuilder {
    id: String,
    properties: HashMap<String, Box<dyn Property>>,
}

impl DeviceBuilder {
    pub(crate) fn new(id: &str) -> Self {
        DeviceBuilder { id: id.to_string(), properties: HashMap::new() }
    }

    pub(crate) fn with_boolean_property(mut self, name: &str, value: bool) -> Self {
        let property: Box<dyn Property> =
            Box::new(BooleanProperty::new(name.to_string(), PropertyType::On, false, None, value));
        self.properties.insert(name.to_string(), property);
        self
    }

    pub(crate) fn with_color_property(mut self, name: &str, xy: CartesianCoordinate, gamut: Option<Gamut>) -> Self {
        let property: Box<dyn Property> =
            Box::new(ColorProperty::new(name.to_string(), PropertyType::Color, false, None, xy, gamut));
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
            claimable: true,
            properties: self.properties,
            external_id: None,
            address: None,
            controller_id: None,
        }
    }
}

pub(crate) fn property_command(value: Value) -> PropertyCommand {
    PropertyCommand { value: Literal { value }, transition: None }
}

pub(crate) fn resolved_property_command(value: Value) -> ResolvedPropertyCommand {
    ResolvedPropertyCommand { value, transition: None }
}

/// A flow that only references the groups, through one control group action per group id, in a chain
pub(crate) fn flow_using_groups(id: &str, name: &str, revision: u64, group_ids: &[&str]) -> VersionedFlow {
    let end_node = FlowNode::new(format!("{id}_end"), vec![], FlowNodeKind::End);
    let first_node = group_ids.iter().enumerate().rev().fold(end_node, |next, (index, group_id)| {
        let action = ControlGroupAction::new(group_id.to_string(), None, HashMap::new());
        let kind = FlowNodeKind::Action(ActionFlowNode::new(Box::new(action)));
        FlowNode::new(format!("{id}_action_{index}"), vec![FlowLink::new(Arc::new(next), Value::None)], kind)
    });
    let start_node = FlowNode::new(
        format!("{id}_start"),
        vec![FlowLink::new(Arc::new(first_node), Value::None)],
        FlowNodeKind::Start,
    );
    let flow = Flow::new(id.to_string(), name.to_string(), None, None, Arc::new(start_node), HashMap::new())
        .expect("valid flow");
    VersionedFlow { flow: Arc::new(flow), revision }
}

/// A flow that only references the preset, through its trigger
pub(crate) fn flow_using_preset(id: &str, name: &str, revision: u64, preset_id: &str) -> VersionedFlow {
    let start_node = FlowNode::new(format!("{id}_start"), vec![], FlowNodeKind::Start);
    let trigger = Expression::PresetValue { id: preset_id.to_string() };
    let flow = Flow::new(id.to_string(), name.to_string(), None, Some(trigger), Arc::new(start_node), HashMap::new())
        .expect("valid flow");
    VersionedFlow { flow: Arc::new(flow), revision }
}
