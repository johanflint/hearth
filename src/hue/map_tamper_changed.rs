use crate::domain::property::PropertyLocator;
use crate::domain::{Event, InitiatedBy};
use crate::hue::domain::{TamperChanged, TamperState};

pub fn map_tamper_changed(property: TamperChanged) -> Vec<Event> {
    let Some(reports) = property.tamper_reports.filter(|reports| !reports.is_empty()) else {
        return Vec::new();
    };

    let tampered = reports.iter().any(|report| report.state == TamperState::Tampered);
    let changed = reports.iter().map(|report| report.changed).max();

    vec![
        Event::BooleanPropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("tampered".to_string()),
            value: tampered,
            initiated_by: InitiatedBy::External,
        },
        Event::DateTimePropertyChanged {
            device_id: property.owner.rid.to_string(),
            property_id: PropertyLocator::Name("tamperedLastChanged".to_string()),
            value: changed,
            initiated_by: InitiatedBy::External,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hue::domain::{HueOwner, TamperReport, TamperSource};
    use chrono::{TimeZone, Utc};
    use rstest::rstest;

    fn owner() -> HueOwner {
        HueOwner { rid: "c0a5a9b4-1b0a-4d6f-8d3a-2f4d6c1e9b77".to_string(), rtype: "device".to_string() }
    }

    fn tamper_changed(tamper_reports: Option<Vec<TamperReport>>) -> TamperChanged {
        TamperChanged { id: "3b7e1c94-2f6a-4d8e-9c15-7a0d4e8b6f32".to_string(), owner: owner(), tamper_reports }
    }

    fn report(second: u32, state: TamperState) -> TamperReport {
        TamperReport {
            changed: Utc.with_ymd_and_hms(2026, 9, 23, 15, 41, second).unwrap(),
            source: TamperSource::BatteryDoor,
            state,
        }
    }

    #[rstest]
    #[case::without_reports(None)]
    #[case::with_empty_reports(Some(vec![]))]
    fn returns_no_events_without_reports(#[case] reports: Option<Vec<TamperReport>>) {
        assert!(map_tamper_changed(tamper_changed(reports)).is_empty());
    }

    #[rstest]
    #[case::tampered(TamperState::Tampered, true)]
    #[case::not_tampered(TamperState::NotTampered, false)]
    fn returns_the_tampered_state_and_the_time_it_changed(#[case] state: TamperState, #[case] expected: bool) {
        let events = map_tamper_changed(tamper_changed(Some(vec![report(8, state)])));

        assert_eq!(
            events,
            vec![
                Event::BooleanPropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("tampered".to_string()),
                    value: expected,
                    initiated_by: InitiatedBy::External,
                },
                Event::DateTimePropertyChanged {
                    device_id: owner().rid,
                    property_id: PropertyLocator::Name("tamperedLastChanged".to_string()),
                    value: Some(Utc.with_ymd_and_hms(2026, 9, 23, 15, 41, 8).unwrap()),
                    initiated_by: InitiatedBy::External,
                },
            ]
        );
    }

    #[test]
    fn is_tampered_when_any_report_is_tampered_and_uses_the_latest_time() {
        let reports = vec![report(5, TamperState::Tampered), report(9, TamperState::NotTampered)];

        let events = map_tamper_changed(tamper_changed(Some(reports)));

        assert!(matches!(&events[0], Event::BooleanPropertyChanged { value: true, .. }));
        assert!(matches!(
            &events[1],
            Event::DateTimePropertyChanged { value: Some(time), .. }
                if *time == Utc.with_ymd_and_hms(2026, 9, 23, 15, 41, 9).unwrap()
        ));
    }
}
