//! Optional failure-only logging avoids changing timing with successful writes.
use tracing::{field::Visit, Event, Metadata, Subscriber};
use tracing_subscriber::{
    layer::{Context, Filter},
    prelude::*,
};

struct Rejections;
struct Failure(String);
impl Visit for Failure {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if matches!(field.name(), "error" | "e") {
            self.0 = format!("{value:?}");
        }
    }
}
impl<S: Subscriber> Filter<S> for Rejections {
    fn enabled(&self, meta: &Metadata<'_>, _: &Context<'_, S>) -> bool {
        meta.target() == "connect_service::canonical_native_dtmf"
            && (meta.fields().field("error").is_some() || meta.fields().field("e").is_some())
    }
    fn event_enabled(&self, event: &Event<'_>, _: &Context<'_, S>) -> bool {
        let mut failure = Failure(String::new());
        event.record(&mut failure);
        !failure.0.contains("OutcomeUnknown") && !failure.0.contains("StorageUnavailable")
    }
}
pub(super) fn install() -> bool {
    if std::env::var_os("CONNECT_G6_DTMF_REJECTIONS_ONLY").is_none() {
        return false;
    }
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_test_writer()
                .with_filter(Rejections),
        )
        .try_init()
        .unwrap();
    true
}
