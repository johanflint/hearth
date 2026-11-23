use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type SwitchId = u32;

const ECHO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default, Debug)]
pub struct PendingWrites {
    writes: Mutex<HashMap<(String, SwitchId), Vec<PendingWrite>>>,
}

impl PendingWrites {
    pub fn record(&self, device_id: &str, switch_id: SwitchId, on: bool, now: Instant) {
        let write = PendingWrite { on, expires_at: now + ECHO_TIMEOUT };

        let mut writes = self.writes.lock().unwrap();
        let pending = writes.entry((device_id.to_string(), switch_id)).or_default();
        pending.retain(|w| w.expires_at > now);
        pending.push(write);
    }

    pub fn confirm(&self, device_id: &str, switch_id: SwitchId, on: bool, now: Instant) -> bool {
        let mut writes = self.writes.lock().unwrap();
        let key = &(device_id.to_string(), switch_id);
        let Some(pending) = writes.get_mut(key) else { return false };

        pending.retain(|w| w.expires_at > now);
        let confirmed = pending.iter_mut().position(|w| w.on == on).map(|i| pending.remove(i)).is_some();
        if pending.is_empty() {
            writes.remove(key);
        }

        confirmed
    }
}

#[derive(Debug)]
struct PendingWrite {
    on: bool,
    expires_at: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWITCH_ID: SwitchId = 0;

    const DEVICE_ID: &str = "shellyplus1-a8032abdaad0";

    #[test]
    fn confirm_matches_a_recorded_write() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
    }

    #[test]
    fn confirm_rejects_the_opposite_value() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(!pending_writes.confirm(DEVICE_ID, SWITCH_ID, false, now));
    }

    #[test]
    fn confirm_rejects_an_unknown_device() {
        let pending_writes = PendingWrites::default();

        assert!(!pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, Instant::now()));
    }

    #[test]
    fn confirm_rejects_a_different_device() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(!pending_writes.confirm("other_device", SWITCH_ID, true, now));
    }

    #[test]
    fn confirm_rejects_a_different_switch() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, 0, true, now);

        assert!(!pending_writes.confirm(DEVICE_ID, 1, true, now));
    }

    #[test]
    fn confirm_rejects_an_expired_write() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(!pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now + ECHO_TIMEOUT));
    }

    #[test]
    fn confirm_matches_a_write_until_it_expires() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now + ECHO_TIMEOUT - Duration::from_millis(1)));
    }

    #[test]
    fn confirm_consumes_the_matched_write() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);
        pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now);

        assert!(!pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
    }

    #[test]
    fn confirm_consumes_only_one_of_two_identical_writes() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);

        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
        assert!(!pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
    }

    #[test]
    fn confirm_matches_the_echoes_of_consecutive_writes() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);
        pending_writes.record(DEVICE_ID, SWITCH_ID, false, now);

        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now));
        assert!(pending_writes.confirm(DEVICE_ID, SWITCH_ID, false, now));
    }

    #[test]
    fn confirm_keeps_the_writes_of_other_switches() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, 0, true, now);
        pending_writes.record(DEVICE_ID, 1, true, now);
        pending_writes.confirm(DEVICE_ID, 0, true, now);

        assert!(pending_writes.confirm(DEVICE_ID, 1, true, now));
    }

    #[test]
    fn confirm_removes_the_entry_when_no_writes_are_left() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);
        pending_writes.confirm(DEVICE_ID, SWITCH_ID, true, now);

        assert!(pending_writes.writes.lock().unwrap().is_empty());
    }

    #[test]
    fn record_prunes_expired_writes() {
        let pending_writes = PendingWrites::default();
        let now = Instant::now();
        pending_writes.record(DEVICE_ID, SWITCH_ID, true, now);
        pending_writes.record(DEVICE_ID, SWITCH_ID, false, now + ECHO_TIMEOUT);

        assert_eq!(pending_writes.writes.lock().unwrap()[&(DEVICE_ID.to_string(), SWITCH_ID)].len(), 1);
    }
}
