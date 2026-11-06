use crate::app_config::AppConfig;
use bytes::Bytes;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, Publish, QoS};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::task;
use tracing::warn;

#[derive(Debug)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Bytes,
}

#[derive(Debug)]
pub struct MqttService {
    client: AsyncClient,
    registry: Arc<Mutex<Registry>>,
}

impl MqttService {
    pub fn spawn(config: &AppConfig) -> Self {
        let mqtt_config = config.mqtt();
        let mut options = MqttOptions::new("hearth", mqtt_config.host(), mqtt_config.port() as u16);
        options.set_credentials(mqtt_config.username(), mqtt_config.password());
        options.set_keep_alive(Duration::from_secs(30));
        options.set_clean_session(true);

        let (client, event_loop) = AsyncClient::new(options, 64);
        let registry = Arc::new(Mutex::new(Registry::default()));
        task::spawn(poll(client.clone(), event_loop, Arc::clone(&registry)));
        MqttService { client, registry }
    }

    pub fn subscribe(&self, filters: &[&str]) -> Result<mpsc::Receiver<MqttMessage>, MqttSubscribeError> {
        let (tx, rx) = mpsc::channel(64);
        let filters: Vec<String> = filters.iter().map(|s| s.to_string()).collect();

        let mut registry = lock(&self.registry);
        if registry.connected {
            for filter in &filters {
                self.client.try_subscribe(filter, QoS::AtLeastOnce)?;
            }
        }
        registry.subscriptions.push(Subscription { filters, tx });
        Ok(rx)
    }
}

async fn poll(client: AsyncClient, mut event_loop: EventLoop, registry: Arc<Mutex<Registry>>) {
    loop {
        match event_loop.poll().await {
            // Clean session: the broker forgets subscriptions, so subscribe after every (re)connect
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                let mut registry = lock(&registry);
                registry.connected = true;
                for filter in registry.subscriptions.iter().flat_map(|s| &s.filters) {
                    if let Err(err) = client.try_subscribe(filter, QoS::AtLeastOnce) {
                        warn!("⚠️ Failed to subscribe to '{filter}': {err}");
                    }
                }
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => dispatch(&registry, publish),
            Ok(_) => {}
            Err(err) => {
                lock(&registry).connected = false;
                warn!("⚠️ MQTT connection error: {err}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn dispatch(registry: &Mutex<Registry>, publish: Publish) {
    let registry = lock(registry);

    for subscription in
        registry.subscriptions.iter().filter(|s| s.filters.iter().any(|f| rumqttc::matches(&publish.topic, f)))
    {
        let message = MqttMessage { topic: publish.topic.clone(), payload: publish.payload.clone() };
        match subscription.tx.try_send(message) {
            Ok(()) | Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(_)) => warn!(
                "⚠️ MQTT subscriber {:?} too slow, dropped message on topic '{}'",
                subscription.filters, publish.topic
            ),
        }
    }
}

fn lock(r: &Mutex<Registry>) -> MutexGuard<'_, Registry> {
    r.lock().expect("mqtt registry lock poisoned")
}

#[derive(Debug)]
struct Subscription {
    filters: Vec<String>,
    tx: mpsc::Sender<MqttMessage>,
}

#[derive(Default, Debug)]
struct Registry {
    subscriptions: Vec<Subscription>,
    connected: bool,
}

#[derive(Debug, Error)]
pub enum MqttSubscribeError {
    #[error(transparent)]
    ClientError(#[from] rumqttc::ClientError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumqttc::MqttOptions;
    use tokio::sync::mpsc::error::TryRecvError;

    fn publish(topic: &str, payload: &'static str) -> Publish {
        Publish::new(topic, QoS::AtLeastOnce, payload)
    }

    fn subscription(filters: &[&str], capacity: usize) -> (Subscription, mpsc::Receiver<MqttMessage>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Subscription { filters: filters.iter().map(|f| f.to_string()).collect(), tx }, rx)
    }

    fn registry(subscriptions: Vec<Subscription>) -> Mutex<Registry> {
        Mutex::new(Registry { subscriptions, connected: false })
    }

    // The event loop is returned so the request channel stays open
    fn service(request_capacity: usize) -> (MqttService, EventLoop) {
        let (client, event_loop) = AsyncClient::new(MqttOptions::new("test", "localhost", 1883), request_capacity);
        (MqttService { client, registry: Arc::new(Mutex::new(Registry::default())) }, event_loop)
    }

    #[test]
    fn dispatch_delivers_the_message_to_a_matching_subscription() {
        let (sub, mut rx) = subscription(&["+/online"], 8);
        let registry = registry(vec![sub]);

        dispatch(&registry, publish("shelly-1/online", "true"));

        let message = rx.try_recv().unwrap();
        assert_eq!(message.topic, "shelly-1/online");
        assert_eq!(message.payload, Bytes::from_static(b"true"));
    }

    #[test]
    fn dispatch_skips_subscriptions_that_do_not_match() {
        let (sub, mut rx) = subscription(&["+/online"], 8);
        let registry = registry(vec![sub]);

        dispatch(&registry, publish("shelly-1/status/switch:0", "{}"));

        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn dispatch_delivers_to_every_matching_subscription() {
        let (first, mut first_rx) = subscription(&["+/online"], 8);
        let (second, mut second_rx) = subscription(&["shelly-1/#"], 8);
        let (other, mut other_rx) = subscription(&["hue/#"], 8);
        let registry = registry(vec![first, second, other]);

        dispatch(&registry, publish("shelly-1/online", "true"));

        assert!(first_rx.try_recv().is_ok());
        assert!(second_rx.try_recv().is_ok());
        assert!(matches!(other_rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn dispatch_delivers_once_when_several_filters_of_one_subscription_match() {
        let (sub, mut rx) = subscription(&["+/online", "shelly-1/#"], 8);
        let registry = registry(vec![sub]);

        dispatch(&registry, publish("shelly-1/online", "true"));

        assert!(rx.try_recv().is_ok());
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }

    #[test]
    fn dispatch_drops_the_message_when_the_subscriber_is_full_and_keeps_the_others_flowing() {
        let (slow, mut slow_rx) = subscription(&["#"], 1);
        let (fast, mut fast_rx) = subscription(&["#"], 8);
        let registry = registry(vec![slow, fast]);

        dispatch(&registry, publish("a", "first"));
        dispatch(&registry, publish("b", "second"));

        assert_eq!(slow_rx.try_recv().unwrap().topic, "a");
        assert!(matches!(slow_rx.try_recv(), Err(TryRecvError::Empty)));
        assert_eq!(fast_rx.try_recv().unwrap().topic, "a");
        assert_eq!(fast_rx.try_recv().unwrap().topic, "b");
    }

    #[test]
    fn dispatch_ignores_closed_subscribers() {
        let (closed, closed_rx) = subscription(&["#"], 8);
        let (open, mut open_rx) = subscription(&["#"], 8);
        drop(closed_rx);
        let registry = registry(vec![closed, open]);

        dispatch(&registry, publish("a", "payload"));

        assert!(open_rx.try_recv().is_ok());
    }

    #[test]
    fn subscribe_registers_the_subscription_so_it_receives_dispatched_messages() {
        let (service, _event_loop) = service(8);

        let mut rx = service.subscribe(&["+/online"]).unwrap();
        dispatch(&service.registry, publish("shelly-1/online", "true"));

        assert_eq!(rx.try_recv().unwrap().topic, "shelly-1/online");
    }

    #[test]
    fn subscribe_does_not_touch_the_client_before_the_connection_is_established() {
        // A full request queue would make try_subscribe fail, so success proves it wasn't called
        let (service, _event_loop) = service(1);

        assert!(service.subscribe(&["a", "b", "c"]).is_ok());
    }

    #[test]
    fn subscribe_fails_without_registering_when_connected_and_the_request_queue_is_full() {
        let (service, _event_loop) = service(1);
        lock(&service.registry).connected = true;

        let result = service.subscribe(&["a", "b"]);

        assert!(matches!(result, Err(MqttSubscribeError::ClientError(_))));
        assert!(lock(&service.registry).subscriptions.is_empty());
    }
}
