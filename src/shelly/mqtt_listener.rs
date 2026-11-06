use crate::domain::property::PropertyLocator;
use crate::domain::{Event, InitiatedBy};
use crate::integration::IntegrationError;
use crate::mqtt::{MqttMessage, MqttService};
use crate::shelly::domain::SwitchStatus;
use crate::store::StoreSnapshot;
use thiserror::Error;
use tokio::sync::mpsc::Sender;
use tokio::sync::watch::Receiver as WatchReceiver;
use tracing::{debug, warn};

const AVAILABILITY_TOPIC: &str = "online";
const SWITCH_STATUS_PREFIX: &str = "status/switch:";

#[derive(Debug, PartialEq)]
enum ShellyMessage {
    Availability(bool),
    Switch(SwitchStatus),
}

pub fn mqtt_listen(
    mqtt_service: &MqttService,
    store_rx: WatchReceiver<StoreSnapshot>,
    tx: Sender<Event>,
) -> Result<(), IntegrationError> {
    debug!("Listening to MQTT messages...");
    let mut rx = mqtt_service.subscribe(&["+/status/+", "+/online"])?;
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            match parse(&msg) {
                Ok(Some((device_id, message))) => {
                    if !store_rx.borrow().devices.contains_key(device_id) {
                        debug!("⚠️ Ignoring message for unknown device '{device_id}'");
                        continue;
                    }
                    match &message {
                        ShellyMessage::Availability(status) => {
                            debug!("Availability for device '{device_id}' changed to '{status}'");
                        }
                        ShellyMessage::Switch(status) => {
                            let property_id = PropertyLocator::ExternalId(format!("switch:{}", status.id));
                            let event = Event::BooleanPropertyChanged {
                                device_id: device_id.to_string(),
                                property_id,
                                value: status.output,
                                initiated_by: InitiatedBy::External,
                            };
                            if tx.send(event).await.is_err() {
                                warn!("⚠️ Event channel closed, stopping MQTT listener");
                                break;
                            }
                        }
                    }
                }
                Ok(None) => {}
                Err(err) => warn!("⚠️ Unable to parse message on '{}': {err}", msg.topic),
            }
        }
    });
    Ok(())
}

fn parse(message: &MqttMessage) -> Result<Option<(&str, ShellyMessage)>, ParseError> {
    let Some((device_id, rest)) = message.topic.split_once('/') else { return Ok(None) };
    let parsed = match rest {
        AVAILABILITY_TOPIC => match message.payload.as_ref() {
            b"true" => ShellyMessage::Availability(true),
            b"false" => ShellyMessage::Availability(false),
            other => return Err(ParseError::Availability(other.to_vec())),
        },
        r if r.starts_with(SWITCH_STATUS_PREFIX) => ShellyMessage::Switch(serde_json::from_slice(&message.payload)?),
        _ => return Ok(None),
    };

    Ok(Some((device_id, parsed)))
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("unexpected availability payload: {0:?}")]
    Availability(Vec<u8>),
    #[error("invalid switch status: {0}")]
    Switch(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    fn message(topic: &str, payload: &'static str) -> MqttMessage {
        MqttMessage { topic: topic.to_string(), payload: Bytes::from_static(payload.as_bytes()) }
    }

    #[rstest]
    #[case("true", true)]
    #[case("false", false)]
    fn parse_reads_the_availability(#[case] payload: &'static str, #[case] expected: bool) {
        let message = message("shellyplus1-a8032abdaad0/online", payload);

        let parsed = parse(&message).unwrap();

        assert_eq!(parsed, Some(("shellyplus1-a8032abdaad0", ShellyMessage::Availability(expected))));
    }

    #[rstest]
    #[case("")]
    #[case("True")]
    #[case("1")]
    #[case(r#"{"online":true}"#)]
    fn parse_rejects_an_unexpected_availability_payload(#[case] payload: &'static str) {
        let message = message("shelly-1/online", payload);
        let result = parse(&message);

        assert!(matches!(result, Err(ParseError::Availability(_))));
    }

    #[test]
    fn parse_reads_a_switch_status() {
        let message = message("shelly-1/status/switch:0", r#"{"id":0,"source":"button","output":true,"apower":0.0}"#);

        let parsed = parse(&message).unwrap();

        assert_eq!(parsed, Some(("shelly-1", ShellyMessage::Switch(SwitchStatus { id: 0, output: true }))));
    }

    #[test]
    fn parse_reads_the_id_of_the_switch_from_the_payload() {
        let message = message("shelly-1/status/switch:1", r#"{"id":1,"output":false}"#);

        let parsed = parse(&message).unwrap();

        assert_eq!(parsed, Some(("shelly-1", ShellyMessage::Switch(SwitchStatus { id: 1, output: false }))));
    }

    #[rstest]
    #[case("{")]
    #[case("{}")]
    #[case(r#"{"id":0}"#)]
    #[case(r#"{"id":0,"output":"on"}"#)]
    fn parse_rejects_an_invalid_switch_status(#[case] payload: &'static str) {
        let message = message("shelly-1/status/switch:0", payload);
        let result = parse(&message);

        assert!(matches!(result, Err(ParseError::Switch(_))));
    }

    #[rstest]
    #[case("shelly-1/status/wifi")]
    #[case("shelly-1/status/input:0")]
    #[case("shelly-1/status/switchfoo")]
    #[case("shelly-1/events/rpc")]
    #[case("shelly-1/online/extra")]
    #[case("shelly-1")]
    #[case("")]
    fn parse_ignores_topics_it_does_not_handle(#[case] topic: &str) {
        let message = message(topic, "{}");
        let parsed = parse(&message).unwrap();

        assert_eq!(parsed, None);
    }
}
