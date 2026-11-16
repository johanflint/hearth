use crate::api::ApiState;
use crate::api::devices::device_view::DeviceView;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

pub(super) async fn list_devices(State(state): State<ApiState>) -> Result<Response, Response> {
    let devices: Vec<_> = state.device_service.list().into_iter().map(DeviceView::from).collect();

    Ok((StatusCode::OK, Json(ListDevicesResponse { devices })).into_response())
}

#[derive(Debug, Serialize)]
struct ListDevicesResponse {
    devices: Vec<DeviceView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::devices::router;
    use crate::api::devices::test_support::{device, state_with_devices};
    use crate::api::test_support::{body_json, create_state};
    use crate::domain::property::{
        BooleanProperty, CartesianCoordinate, DateTimeProperty, EnumProperty, Gamut, NumberProperty, Property,
        PropertyType, Unit,
    };
    use crate::domain::{Device, DeviceType};
    use crate::flow_registry::FlowRegistry;
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::{TimeZone, Utc};
    use rstest::rstest;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    async fn call_list_devices(state: ApiState) -> Response {
        let request = Request::builder().method("GET").uri("/api/devices").body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    fn device_with_property(property: impl Property + 'static) -> Device {
        DeviceBuilder::new("device").with_properties(vec![Box::new(property)]).build()
    }

    async fn list_devices_json(devices: Vec<Device>) -> Value {
        let response = call_list_devices(state_with_devices(devices)).await;

        assert_eq!(response.status(), StatusCode::OK);
        body_json(response).await
    }

    async fn first_property(device: Device) -> Value {
        list_devices_json(vec![device]).await["devices"][0]["properties"][0].clone()
    }

    #[tokio::test]
    async fn list_devices_returns_an_empty_list_when_there_are_no_devices() {
        let state = create_state(FlowRegistry::new(vec![])).state;

        let response = call_list_devices(state).await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await, json!({ "devices": [] }));
    }

    #[tokio::test]
    async fn list_devices_returns_the_device_with_camel_case_fields() {
        let body = list_devices_json(vec![device("lamp", "Lamp")]).await;

        assert_eq!(
            body,
            json!({
                "devices": [{
                    "id": "lamp",
                    "type": "light",
                    "manufacturer": "Signify Netherlands B.V.",
                    "modelId": "Test Model",
                    "productName": "Hue color lamp",
                    "name": "Lamp",
                    "claimable": true,
                    "properties": [],
                }]
            })
        );
    }

    #[rstest]
    #[case::light(DeviceType::Light, "light")]
    #[case::motion_sensor(DeviceType::MotionSensor, "motionSensor")]
    #[case::remote(DeviceType::Remote, "remote")]
    #[case::contact_sensor(DeviceType::ContactSensor, "contactSensor")]
    #[case::switch(DeviceType::Switch, "switch")]
    #[case::presence_sensor(DeviceType::PresenceSensor, "presenceSensor")]
    #[tokio::test]
    async fn list_devices_returns_the_type_of_the_device(#[case] r#type: DeviceType, #[case] expected: &str) {
        let body = list_devices_json(vec![Device { r#type, ..device("device", "Device") }]).await;

        assert_eq!(body["devices"][0]["type"], expected);
    }

    #[tokio::test]
    async fn list_devices_returns_the_devices_ordered_by_name() {
        let body = list_devices_json(vec![device("1", "Remote"), device("2", "Ceiling"), device("3", "Lamp")]).await;

        let ids: Vec<_> = body["devices"].as_array().expect("devices array").iter().map(|d| d["id"].clone()).collect();
        assert_eq!(ids, vec![json!("2"), json!("3"), json!("1")]);
    }

    #[tokio::test]
    async fn list_devices_returns_the_properties_ordered_by_name() {
        let device = DeviceBuilder::new("device")
            .with_boolean_property("on", true)
            .with_boolean_property("enabled", false)
            .with_boolean_property("connected", true)
            .build();

        let body = list_devices_json(vec![device]).await;

        let names: Vec<_> = body["devices"][0]["properties"]
            .as_array()
            .expect("properties array")
            .iter()
            .map(|p| p["name"].clone())
            .collect();
        assert_eq!(names, vec![json!("connected"), json!("enabled"), json!("on")]);
    }

    #[tokio::test]
    async fn list_devices_returns_a_boolean_property() {
        let property = BooleanProperty::new("on".to_string(), PropertyType::On, false, None, true);

        assert_eq!(
            first_property(device_with_property(property)).await,
            json!({ "name": "on", "propertyType": "on", "readonly": false, "kind": "boolean", "value": true })
        );
    }

    #[tokio::test]
    async fn list_devices_returns_a_number_property() {
        let property = NumberProperty::builder("brightness".to_string(), PropertyType::Brightness, false)
            .unit(Unit::Percentage)
            .positive_int(Some(80), Some(0), Some(100))
            .build();

        assert_eq!(
            first_property(device_with_property(property)).await,
            json!({
                "name": "brightness",
                "propertyType": "brightness",
                "readonly": false,
                "kind": "number",
                "value": 80,
                "unit": "percentage",
                "minimum": 0,
                "maximum": 100,
            })
        );
    }

    #[tokio::test]
    async fn list_devices_returns_a_number_property_without_a_value_or_limits() {
        let property = NumberProperty::builder("temperature".to_string(), PropertyType::Temperature, true)
            .unit(Unit::DegreesCelsius)
            .positive_int(None, None, None)
            .build();

        assert_eq!(
            first_property(device_with_property(property)).await,
            json!({
                "name": "temperature",
                "propertyType": "temperature",
                "readonly": true,
                "kind": "number",
                "value": null,
                "unit": "degreesCelsius",
                "minimum": null,
                "maximum": null,
            })
        );
    }

    #[rstest]
    #[case::positive_int(NumberProperty::builder("n".to_string(), PropertyType::Brightness, false).positive_int(Some(5), None, None).build(), json!(5))]
    #[case::negative_int(NumberProperty::builder("n".to_string(), PropertyType::Brightness, false).negative_int(Some(-5), None, None).build(), json!(-5))]
    #[case::float(NumberProperty::builder("n".to_string(), PropertyType::Brightness, false).float(Some(2.5), None, None).build(), json!(2.5))]
    #[tokio::test]
    async fn list_devices_returns_the_number_value_as_a_plain_json_number(
        #[case] property: NumberProperty,
        #[case] expected: Value,
    ) {
        let property = first_property(device_with_property(property)).await;

        assert_eq!(property["value"], expected);
    }

    #[rstest]
    #[case::percentage(Unit::Percentage, "percentage")]
    #[case::lux(Unit::Lux, "lux")]
    #[case::degrees_celsius(Unit::DegreesCelsius, "degreesCelsius")]
    #[case::kelvin(Unit::Kelvin, "kelvin")]
    #[case::none(Unit::None, "none")]
    #[tokio::test]
    async fn list_devices_returns_the_unit_of_a_number_property(#[case] unit: Unit, #[case] expected: &str) {
        let property = NumberProperty::builder("n".to_string(), PropertyType::Brightness, false).unit(unit).build();

        let property = first_property(device_with_property(property)).await;

        assert_eq!(property["unit"], expected);
    }

    #[tokio::test]
    async fn list_devices_returns_an_enum_property() {
        let property = EnumProperty::new(
            "button".to_string(),
            PropertyType::Button,
            true,
            None,
            Some("pressed".to_string()),
            vec!["pressed".to_string(), "released".to_string()],
        )
        .unwrap();

        assert_eq!(
            first_property(device_with_property(property)).await,
            json!({
                "name": "button",
                "propertyType": "button",
                "readonly": true,
                "kind": "enum",
                "value": "pressed",
                "allowedValues": ["pressed", "released"],
            })
        );
    }

    #[tokio::test]
    async fn list_devices_returns_an_enum_property_without_a_value() {
        let property = EnumProperty::new(
            "button".to_string(),
            PropertyType::Button,
            true,
            None,
            None,
            vec!["pressed".to_string()],
        )
        .unwrap();

        let property = first_property(device_with_property(property)).await;

        assert_eq!(property["value"], Value::Null);
        assert_eq!(property["allowedValues"], json!(["pressed"]));
    }

    #[rstest]
    #[case::without_gamut(None, Value::Null)]
    #[case::with_gamut(
        Some(Gamut::new(
            CartesianCoordinate::new(0.7, 0.3),
            CartesianCoordinate::new(0.2, 0.7),
            CartesianCoordinate::new(0.1, 0.05),
        )),
        json!({
            "red": { "x": 0.7, "y": 0.3 },
            "green": { "x": 0.2, "y": 0.7 },
            "blue": { "x": 0.1, "y": 0.05 },
        })
    )]
    #[tokio::test]
    async fn list_devices_returns_a_color_property(#[case] gamut: Option<Gamut>, #[case] expected_gamut: Value) {
        let device = DeviceBuilder::new("device")
            .with_color_property("color", CartesianCoordinate::new(0.3, 0.4), gamut)
            .build();

        assert_eq!(
            first_property(device).await,
            json!({
                "name": "color",
                "propertyType": "color",
                "readonly": false,
                "kind": "color",
                "xy": { "x": 0.3, "y": 0.4 },
                "gamut": expected_gamut,
            })
        );
    }

    #[rstest]
    #[case::without_value(None, Value::Null)]
    #[case::with_value(Some(Utc.with_ymd_and_hms(2026, 11, 16, 9, 30, 0).unwrap()), json!("2026-11-16T09:30:00Z"))]
    #[tokio::test]
    async fn list_devices_returns_a_datetime_property(
        #[case] value: Option<chrono::DateTime<Utc>>,
        #[case] expected: Value,
    ) {
        let property = DateTimeProperty::new(
            "motion_last_changed".to_string(),
            PropertyType::MotionLastChanged,
            true,
            None,
            value,
        );

        assert_eq!(
            first_property(device_with_property(property)).await,
            json!({
                "name": "motion_last_changed",
                "propertyType": "motionLastChanged",
                "readonly": true,
                "kind": "dateTime",
                "value": expected,
            })
        );
    }
}
