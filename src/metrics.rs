use metrics::{Unit, describe_counter, describe_gauge, describe_histogram};
use strum::EnumIter;
use strum::IntoEnumIterator;

pub fn describe() {
    for metric in Metric::iter() {
        match metric.kind() {
            MetricKind::Counter => describe_counter!(metric.name(), metric.unit(), metric.description()),
            MetricKind::Gauge => describe_gauge!(metric.name(), metric.unit(), metric.description()),
            MetricKind::Histogram => describe_histogram!(metric.name(), metric.unit(), metric.description()),
        }
    }
}

#[derive(EnumIter, Debug)]
pub enum Metric {
    StoreDiscoveredDevices,
    StoreNumberOfDevices,
    StorePropertyChanged,
    FlowExecutionDuration,
}

impl Metric {
    pub const fn name(&self) -> &'static str {
        match self {
            Metric::StoreDiscoveredDevices => "hearth_store_discovered_devices_total",
            Metric::StoreNumberOfDevices => "hearth_store_number_of_devices",
            Metric::StorePropertyChanged => "hearth_store_property_changed_total",
            Metric::FlowExecutionDuration => "hearth_flow_execution_duration_seconds",
        }
    }

    const fn kind(&self) -> MetricKind {
        match self {
            Metric::StoreNumberOfDevices => MetricKind::Gauge,
            Metric::FlowExecutionDuration => MetricKind::Histogram,
            _ => MetricKind::Counter,
        }
    }

    const fn unit(&self) -> Unit {
        match self {
            Metric::FlowExecutionDuration => Unit::Seconds,
            _ => Unit::Count
        }
    }

    const fn description(&self) -> &'static str {
        match self {
            Metric::StoreDiscoveredDevices => "Newly discovered devices",
            Metric::StoreNumberOfDevices => "Total number of devices",
            Metric::StorePropertyChanged => "Number of times a property changed event is processed",
            Metric::FlowExecutionDuration => "Flow execution duration",
        }
    }
}

#[derive(Debug)]
enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}