use crate::shelly::device_address::DeviceAddress;
use crate::shelly::domain::{Component, ComponentsResult, DeviceInfo};
use crate::shelly::prober::ProbedDevice;
use std::net::SocketAddr;

pub(super) const PLUS_1_DEVICE_INFO: &str =
    include_str!("../../tests/resources/shelly_device_info_plus1_response.json");
pub(super) const PLUS_1_COMPONENTS: &str = include_str!("../../tests/resources/shelly_components_plus1_response.json");
pub(super) const PLUS_1_ID: &str = "shellyplus1-aabbccddeeff";

/// A Plus 1 with all the components from the fixture
pub(super) fn probed_plus_1() -> ProbedDevice {
    let components: ComponentsResult = serde_json::from_str(PLUS_1_COMPONENTS).unwrap();
    ProbedDevice {
        address: DeviceAddress::new("192.168.1.35", 80),
        info: plus_1_device_info(),
        components: components.components,
    }
}

/// A Plus 1 with the given id and components
pub(super) fn probed_device(id: &str, components: &[String]) -> ProbedDevice {
    let mut info = plus_1_device_info();
    info.id = id.to_string();
    ProbedDevice { address: DeviceAddress::new("192.168.1.35", 80), info, components: parse_components(components) }
}

/// The address of a mock server, which is reached by IP instead of by an mDNS hostname
pub(super) fn device_address(server_address: SocketAddr) -> DeviceAddress {
    DeviceAddress::new(&server_address.ip().to_string(), server_address.port())
}

pub(super) fn switch_component(id: u32, output: bool) -> String {
    format!(r#"{{"key":"switch:{id}","status":{{"id":{id},"output":{output}}},"config":{{}}}}"#)
}

pub(super) fn ble_component() -> String {
    r#"{"key":"ble","status":{},"config":{"enable":false}}"#.to_string()
}

pub(super) fn mqtt_component(connected: bool, config: serde_json::Value) -> String {
    serde_json::json!({ "key": "mqtt", "status": { "connected": connected }, "config": config }).to_string()
}

fn plus_1_device_info() -> DeviceInfo {
    serde_json::from_str(PLUS_1_DEVICE_INFO).unwrap()
}

fn parse_components(components: &[String]) -> Vec<Component> {
    let result: ComponentsResult =
        serde_json::from_str(&format!(r#"{{"components":[{}]}}"#, components.join(","))).unwrap();
    result.components
}
