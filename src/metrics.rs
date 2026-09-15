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
    FlowExecutionDuration,
    FlowExecutionFailures,
    SseConnectionAttempts,
    StoreDeviceCount,
    StoreDeviceDiscoveries,
    StorePropertyChanges,
}

impl Metric {
    pub const fn name(&self) -> &'static str {
        match self {
            Metric::FlowExecutionDuration => "hearth_flow_execution_duration_seconds",
            Metric::FlowExecutionFailures => "hearth_flow_execution_failures_total",
            Metric::SseConnectionAttempts => "hearth_see_connection_attempts_total",
            Metric::StoreDeviceCount => "hearth_store_device_count",
            Metric::StoreDeviceDiscoveries => "hearth_store_device_discoveries_total",
            Metric::StorePropertyChanges => "hearth_store_property_changes_total",
        }
    }

    const fn kind(&self) -> MetricKind {
        match self {
            Metric::FlowExecutionDuration => MetricKind::Histogram,
            Metric::StoreDeviceCount => MetricKind::Gauge,
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
            Metric::FlowExecutionDuration => "Flow execution duration",
            Metric::FlowExecutionFailures => "Flow execution failures",
            Metric::SseConnectionAttempts => "Number of SSE connection attempts",
            Metric::StoreDeviceCount => "Total number of devices",
            Metric::StoreDeviceDiscoveries => "Newly discovered devices",
            Metric::StorePropertyChanges => "Number of times a property changed event is processed",
        }
    }
}

#[derive(Debug)]
enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

pub trait ResultOutcomeLabel {
    fn metric_label(&self) -> &'static str;
}

impl<T, E> ResultOutcomeLabel for Result<T, E> {
    fn metric_label(&self) -> &'static str {
        if self.is_ok() { "success" } else { "failure" }
    }
}

pub trait MetricReason {
    fn metric_reason(&self) -> &'static str;
}
