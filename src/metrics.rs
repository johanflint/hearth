use metrics::{Unit, describe_counter, describe_gauge};
use strum::EnumIter;
use strum::IntoEnumIterator;

pub fn describe() {
    for metric in Metric::iter() {
        match metric.kind() {
            MetricKind::Counter => describe_counter!(metric.name(), metric.unit(), metric.description()),
            MetricKind::Gauge => describe_gauge!(metric.name(), metric.unit(), metric.description()),
        }
    }
}

#[derive(EnumIter, Debug)]
pub enum Metric {
    StoreDiscoveredDevices,
    StoreNumberOfDevices,
    StorePropertyChanged,
}

impl Metric {
    pub const fn name(&self) -> &'static str {
        match self {
            Metric::StoreDiscoveredDevices => "hearth_store_discovered_devices_total",
            Metric::StoreNumberOfDevices => "hearth_store_number_of_devices",
            Metric::StorePropertyChanged => "hearth_store_property_changed_total",
        }
    }

    const fn kind(&self) -> MetricKind {
        match self {
            Metric::StoreNumberOfDevices => MetricKind::Gauge,
            _ => MetricKind::Counter,
        }
    }

    const fn unit(&self) -> Unit {
        Unit::Count
    }

    const fn description(&self) -> &'static str {
        match self {
            Metric::StoreDiscoveredDevices => "Newly discovered devices",
            Metric::StoreNumberOfDevices => "Total number of devices",
            Metric::StorePropertyChanged => "Number of times a property changed event is processed",
        }
    }
}

#[derive(Debug)]
enum MetricKind {
    Counter,
    Gauge,
}