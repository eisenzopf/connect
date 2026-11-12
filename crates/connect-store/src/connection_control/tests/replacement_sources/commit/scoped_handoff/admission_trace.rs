//! Test-only bounded scalar view of the EXISTING authority diagnostic target.
//! No scope names, IDs, credentials, payload, IO grant or production hook.
use std::sync::{Arc, Mutex};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Event, Metadata, Subscriber,
};

#[derive(Default, Clone, Debug)]
pub(super) struct Summary {
    cause: Option<String>,
    stage: Option<String>,
    kind: Option<String>,
    write: Option<bool>,
    completed_scopes: Option<u64>,
    total_scopes: Option<u64>,
}
impl Visit for Summary {
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}
    fn record_str(&mut self, field: &Field, value: &str) {
        // Only fixed internal labels from Admission::drop are retained.
        match field.name() {
            "cause" => self.cause = Some(value.into()),
            "stage" => self.stage = Some(value.into()),
            "kind" => self.kind = Some(value.into()),
            _ => {}
        }
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "write" {
            self.write = Some(value);
        }
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "completed_scopes" => self.completed_scopes = Some(value),
            "total_scopes" => self.total_scopes = Some(value),
            _ => {}
        }
    }
}
pub(super) type Buffer = Arc<Mutex<Vec<Summary>>>;
pub(super) fn buffer() -> Buffer {
    Arc::new(Mutex::new(Vec::new()))
}
pub(super) fn snapshot(buffer: &Buffer) -> Vec<Summary> {
    buffer.lock().unwrap().clone()
}
struct Trace(Buffer);
impl Subscriber for Trace {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "connect_store::authority_summary"
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let mut buffer = self.0.lock().unwrap();
        if buffer.len() < 32 {
            let mut summary = Summary::default();
            event.record(&mut summary);
            buffer.push(summary);
        }
    }
}
pub(super) fn capture<T>(buffer: &Buffer, call: impl FnOnce() -> T) -> T {
    tracing::subscriber::with_default(Trace(buffer.clone()), call)
}
