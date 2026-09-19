use metrics::histogram;
use std::fmt::Debug;
use std::time::Instant;
use tracing::field::{Field, Visit};
use tracing::span::Attributes;
use tracing::{Id, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

struct SpanStart(Instant);

struct MetricName(String);

#[derive(Default)]
struct MetricNameVisitor(Option<String>);

impl Visit for MetricNameVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "metric_name" {
            self.0 = Some(value.to_string());
        }
    }

    // Required by the trait but never hit by string fields, so it's a no-op
    fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
}

pub struct MetricsLayer;

impl<S> Layer<S> for MetricsLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return; };
        span.extensions_mut().insert(SpanStart(Instant::now()));

        let mut visitor = MetricNameVisitor::default();
        attrs.record(&mut visitor);
        if let Some(name) = visitor.0.take() {
            span.extensions_mut().insert(MetricName(name));
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return; };
        let extensions = span.extensions();
        let Some(start) = extensions.get::<SpanStart>() else { return; };
        let elapsed = start.0.elapsed();

        // Any span tagged with `fields(metric_name = "...")` is automatically recorded.
        // Spans without the field are ignored.
        if let Some(metric_name) = extensions.get::<MetricName>() {
            histogram!(metric_name.0.clone()).record(elapsed.as_secs_f64());
        }
    }
}
