use crate::hue::domain::{LightChanged, LightRequest};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ECHO_TIMEOUT: Duration = Duration::from_secs(5);
const BRIGHTNESS_TOLERANCE: f64 = 0.5;
const MIREK_TOLERANCE: u64 = 1;
const XY_TOLERANCE: f64 = 0.002;

#[derive(Default, Debug)]
pub struct PendingWrites {
    writes: Mutex<HashMap<String, Vec<PendingWrite>>>,
}

impl PendingWrites {
    pub fn record(&self, light_id: &str, request: &LightRequest, now: Instant) {
        let write = PendingWrite::from_request(request, now + ECHO_TIMEOUT);
        if write.is_empty() {
            return;
        }

        let mut writes = self.writes.lock().unwrap();
        let pending = writes.entry(light_id.to_string()).or_default();
        // Hue sends no event for a no-op write, prune here too so its expectation doesn't linger
        pending.retain(|w| w.expires_at > now);
        pending.push(write);
    }

    pub fn confirm(&self, change: &LightChanged, now: Instant) -> bool {
        let mut writes = self.writes.lock().unwrap();
        let Some(pending) = writes.get_mut(&change.id) else {
            return false;
        };

        pending.retain(|w| w.expires_at > now);
        let confirmed = pending.iter_mut().any(|w| w.confirm(change));
        pending.retain(|w| !w.is_empty());
        if pending.is_empty() {
            writes.remove(&change.id);
        }

        confirmed
    }
}

// Values a request sent in Hue units, after clamping, conversion and gamut clipping
// Lights are the only ones that can be claimed
#[derive(Debug)]
struct PendingWrite {
    on: Option<bool>,
    brightness: Option<f64>,
    mirek: Option<u64>,
    xy: Option<(f64, f64)>,
    expires_at: Instant,
}

impl PendingWrite {
    fn from_request(request: &LightRequest, expires_at: Instant) -> Self {
        PendingWrite {
            on: request.on.as_ref().map(|on| on.on),
            brightness: request.dimming.as_ref().map(|dimming| dimming.brightness),
            mirek: request.color_temperature.as_ref().map(|color_temperature| color_temperature.mirek),
            xy: request.color.as_ref().map(|color| (color.xy.x, color.xy.y)),
            expires_at,
        }
    }

    fn is_empty(&self) -> bool {
        self.on.is_none() && self.brightness.is_none() && self.mirek.is_none() && self.xy.is_none()
    }

    fn confirm(&mut self, change: &LightChanged) -> bool {
        let on =
            take_if_matches(&mut self.on, change.on.as_ref().map(|on| on.on), |expected, actual| expected == actual);
        let brightness = take_if_matches(
            &mut self.brightness,
            change.dimming.as_ref().map(|dimming| dimming.brightness),
            |expected, actual| (expected - actual).abs() <= BRIGHTNESS_TOLERANCE,
        );
        let mirek = take_if_matches(
            &mut self.mirek,
            change.color_temperature.as_ref().and_then(|color_temperature| color_temperature.mirek),
            |expected, actual| expected.abs_diff(actual) <= MIREK_TOLERANCE,
        );
        let xy = take_if_matches(
            &mut self.xy,
            change.color.as_ref().map(|color| (color.xy.x, color.xy.y)),
            |expected, actual| {
                (expected.0 - actual.0).abs() <= XY_TOLERANCE && (expected.1 - actual.1).abs() <= XY_TOLERANCE
            },
        );

        on || brightness || mirek || xy
    }
}

// Clears the expected value if the actual value matches it
fn take_if_matches<T: Copy>(expected: &mut Option<T>, actual: Option<T>, matches: impl Fn(T, T) -> bool) -> bool {
    let matched = matches!((*expected, actual), (Some(expected), Some(actual)) if matches(expected, actual));
    if matched {
        *expected = None;
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::On;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    const LIGHT_ID: &str = "light_id";

    fn request(on: Option<bool>, brightness: Option<f64>, mirek: Option<u64>) -> LightRequest {
        LightRequest::new(on.map(|on| On { on }), brightness, mirek, None, None)
    }

    fn change(data: serde_json::Value) -> LightChanged {
        let mut value = json!({ "id": LIGHT_ID, "owner": { "rid": "device_id", "rtype": "device" } });
        value.as_object_mut().unwrap().extend(data.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn confirm_matches_a_brightness_within_the_tolerance() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);

        assert!(pending_writes.confirm(&change(json!({ "dimming": { "brightness": 39.92 } })), now));
    }

    #[test]
    fn confirm_rejects_a_brightness_outside_the_tolerance() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);

        assert!(!pending_writes.confirm(&change(json!({ "dimming": { "brightness": 80.0 } })), now));
    }

    #[test]
    fn confirm_rejects_an_unknown_light() {
        let pending_writes = PendingWrites::default();

        assert!(!pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), Instant::now()));
    }

    #[test]
    fn confirm_rejects_an_expired_write() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);

        assert!(!pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), now + ECHO_TIMEOUT));
    }

    #[test]
    fn confirm_matches_a_mirek_with_its_linked_color() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, None, Some(366)), now);

        let change = change(
            json!({ "color": { "xy": { "x": 0.4584, "y": 0.41 } }, "color_temperature": { "mirek": 367, "mirek_valid": true } }),
        );
        assert!(pending_writes.confirm(&change, now));
    }

    #[test]
    fn confirm_matches_fields_reported_in_separate_updates() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(Some(true), Some(40.0), None), now);

        assert!(pending_writes.confirm(&change(json!({ "on": { "on": true } })), now));
        assert!(pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), now));
    }

    #[test]
    fn confirm_consumes_the_matched_field() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);
        pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), now);

        assert!(!pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), now));
    }

    #[test]
    fn confirm_matches_the_echoes_of_consecutive_writes() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);
        pending_writes.record(LIGHT_ID, &request(None, Some(60.0), None), now);

        assert!(pending_writes.confirm(&change(json!({ "dimming": { "brightness": 40.0 } })), now));
        assert!(pending_writes.confirm(&change(json!({ "dimming": { "brightness": 60.0 } })), now));
    }

    #[test]
    fn record_prunes_expired_writes() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(LIGHT_ID, &request(None, Some(40.0), None), now);
        pending_writes.record(LIGHT_ID, &request(None, Some(60.0), None), now + ECHO_TIMEOUT);

        assert_eq!(pending_writes.writes.lock().unwrap()[LIGHT_ID].len(), 1);
    }
}
