use crate::api::devices::color_view::{CartesianCoordinateView, GamutView};
use crate::api::devices::number_view::NumberView;
use crate::api::devices::property_type_view::PropertyTypeView;
use crate::api::devices::unit_view::UnitView;
use crate::domain::property::{
    BooleanProperty, ColorProperty, DateTimeProperty, EnumProperty, NumberProperty, Property, ValueKind,
};
use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PropertyView {
    pub(super) name: String,
    property_type: PropertyTypeView,
    readonly: bool,
    #[serde(flatten)]
    value: PropertyValueView,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum PropertyValueView {
    Boolean { value: bool },
    Color { xy: CartesianCoordinateView, gamut: Option<GamutView> },
    DateTime { value: Option<DateTime<Utc>> },
    Enum { value: Option<String>, allowed_values: Vec<String> },
    Number { value: Option<NumberView>, unit: UnitView, minimum: Option<NumberView>, maximum: Option<NumberView> },
}

impl From<&dyn Property> for PropertyView {
    fn from(p: &dyn Property) -> Self {
        let any = p.as_any();
        let value = match p.value_kind() {
            ValueKind::Boolean => {
                let property = any.downcast_ref::<BooleanProperty>().expect("value kind matches type");
                PropertyValueView::Boolean { value: property.value() }
            }
            ValueKind::Color => {
                let property = any.downcast_ref::<ColorProperty>().expect("value kind matches type");
                PropertyValueView::Color { xy: property.xy().into(), gamut: property.gamut().map(GamutView::from) }
            }
            ValueKind::DateTime => {
                let property = any.downcast_ref::<DateTimeProperty>().expect("value kind matches type");
                PropertyValueView::DateTime { value: property.value() }
            }
            ValueKind::Enum => {
                let property = any.downcast_ref::<EnumProperty>().expect("value kind matches type");
                PropertyValueView::Enum {
                    value: property.value().map(str::to_string),
                    allowed_values: property.allowed_values().to_vec(),
                }
            }
            ValueKind::Number => {
                let property = any.downcast_ref::<NumberProperty>().expect("value kind matches type");
                PropertyValueView::Number {
                    value: property.value().map(NumberView::from),
                    unit: property.unit().into(),
                    minimum: property.minimum().map(NumberView::from),
                    maximum: property.maximum().map(NumberView::from),
                }
            }
            ValueKind::None => unreachable!("no property has value kind None"),
        };
        PropertyView {
            name: p.name().to_string(),
            property_type: (&p.property_type()).into(),
            readonly: p.readonly(),
            value,
        }
    }
}
