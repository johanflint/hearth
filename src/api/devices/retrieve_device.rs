use crate::api::ApiState;
use crate::api::devices::device_view::DeviceView;
use crate::services::RetrieveDeviceError;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub(super) async fn retrieve_device(
    Path(id): Path<String>,
    State(state): State<ApiState>,
) -> Result<Response, Response> {
    let device = state.device_service.retrieve_device(&id).map_err(retrieve_error_response)?;
    let response = DeviceView::from(device);
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn retrieve_error_response(err: RetrieveDeviceError) -> Response {
    match err {
        RetrieveDeviceError::NotFound => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::devices::router;
    use crate::api::devices::test_support::{device, state_with_devices};
    use crate::api::test_support::body_json;
    use crate::domain::property::{BooleanProperty, Property, PropertyType};
    use crate::domain::{Device, DeviceType};
    use crate::test_support::DeviceBuilder;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use rstest::rstest;
    use serde_json::json;
    use tower::ServiceExt;

    async fn call_retrieve_device(state: ApiState, id: &str) -> Response {
        let request = Request::builder().method("GET").uri(format!("/api/devices/{id}")).body(Body::empty()).unwrap();

        router().with_state(state).oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn retrieve_device_returns_the_device_with_camel_case_fields() {
        let state = state_with_devices(vec![device("lamp", "Lamp")]);

        let response = call_retrieve_device(state, "lamp").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            json!({
                "id": "lamp",
                "type": "light",
                "manufacturer": "Signify Netherlands B.V.",
                "modelId": "Test Model",
                "productName": "Hue color lamp",
                "name": "Lamp",
                "claimable": true,
                "properties": [],
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
    async fn retrieve_device_returns_the_type_of_the_device(#[case] r#type: DeviceType, #[case] expected: &str) {
        let state = state_with_devices(vec![Device { r#type, ..device("device", "Device") }]);

        let response = call_retrieve_device(state, "device").await;

        assert_eq!(body_json(response).await["type"], expected);
    }

    #[tokio::test]
    async fn retrieve_device_returns_the_properties_ordered_by_name() {
        let device = DeviceBuilder::new("device")
            .with_boolean_property("on", true)
            .with_boolean_property("enabled", false)
            .with_boolean_property("connected", true)
            .build();
        let state = state_with_devices(vec![device]);

        let response = call_retrieve_device(state, "device").await;

        let body = body_json(response).await;
        let names: Vec<_> =
            body["properties"].as_array().expect("properties array").iter().map(|p| p["name"].clone()).collect();
        assert_eq!(names, vec![json!("connected"), json!("enabled"), json!("on")]);
    }

    #[tokio::test]
    async fn retrieve_device_returns_the_properties_of_the_device() {
        let property: Box<dyn Property> =
            Box::new(BooleanProperty::new("on".to_string(), PropertyType::On, false, None, true));
        let state = state_with_devices(vec![DeviceBuilder::new("device").with_properties(vec![property]).build()]);

        let response = call_retrieve_device(state, "device").await;

        assert_eq!(
            body_json(response).await["properties"],
            json!([{ "name": "on", "propertyType": "on", "readonly": false, "kind": "boolean", "value": true }])
        );
    }

    #[tokio::test]
    async fn retrieve_device_finds_an_id_with_url_encoded_characters() {
        let state = state_with_devices(vec![device("a b", "Spaced")]);

        let response = call_retrieve_device(state, "a%20b").await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["name"], "Spaced");
    }

    #[tokio::test]
    async fn retrieve_device_returns_only_the_requested_device() {
        let state = state_with_devices(vec![device("a", "A"), device("b", "B")]);

        let response = call_retrieve_device(state, "b").await;

        assert_eq!(body_json(response).await["name"], "B");
    }

    #[tokio::test]
    async fn retrieve_device_returns_404_for_an_unknown_device() {
        let state = state_with_devices(vec![device("a", "A")]);

        let response = call_retrieve_device(state, "unknown").await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert!(body.is_empty(), "got {body:?}");
    }
}
