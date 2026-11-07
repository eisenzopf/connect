//! Observe the real immediate-admission diagnostic under independent file locks.
use super::*;
use std::sync::atomic::AtomicUsize;
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Event, Metadata, Subscriber,
};

#[derive(Default)]
struct Cause(Option<String>);
impl Visit for Cause {
    fn record_debug(&mut self, _: &Field, _: &dyn std::fmt::Debug) {}

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "cause" {
            self.0 = Some(value.to_owned());
        }
    }
}

struct Trace<const N: usize> {
    expected: &'static str,
    partial_locks: [File; N],
    observed: Arc<AtomicUsize>,
}
impl<const N: usize> Subscriber for Trace<N> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.target() == "connect_store::authority_summary"
    }

    fn new_span(&self, _: &Attributes<'_>) -> Id {
        unreachable!("authority diagnostics emit events without spans")
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut cause = Cause::default();
        event.record(&mut cause);
        assert_eq!(cause.0.as_deref(), Some(self.expected));
        for independent in &self.partial_locks {
            FileExt::try_lock_exclusive(independent)
                .expect("diagnostic ran before the attempted admission released its partial lock");
            FileExt::unlock(independent).unwrap();
        }
        self.observed.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn observe_refusal<T, const N: usize>(
    expected: &'static str,
    partial_locks: [File; N],
    attempt: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let observed = Arc::new(AtomicUsize::new(0));
    let result = tracing::subscriber::with_default(
        Trace {
            expected,
            partial_locks,
            observed: observed.clone(),
        },
        attempt,
    );
    assert_eq!(observed.load(Ordering::Relaxed), 1);
    let error = result.as_ref().err().expect("actual admission must refuse");
    assert_eq!(error.code, "authority-busy");
    assert_eq!(error.message, "Current authority is busy");
    result
}

#[test]
fn immediate_kernel_admission_and_resource_contention_emit_distinct_causes_after_release() {
    let f = Fixture::new();
    let a = AuthorityScope::identity("admission_trace_a", AuthorityMode::Write).unwrap();
    let b = AuthorityScope::identity("admission_trace_b", AuthorityMode::Read).unwrap();
    let file = |scope: &AuthorityScope, role: &str| {
        let name = f
            .store
            .authority
            .name(&(scope.kind, scope.id.clone()))
            .unwrap();
        f.store.authority.file(&format!("{name}.{role}")).unwrap()
    };

    let blocking_admission = file(&b, "admission");
    FileExt::try_lock_exclusive(&blocking_admission).unwrap();
    drop(
        observe_refusal(
            "kernel-admission-lock",
            [file(&a, "admission"), file(&a, "resource")],
            || f.store.try_acquire_authority(&[a.clone(), b.clone()]),
        )
        .unwrap_err(),
    );
    drop(blocking_admission);

    let blocking_reader = f.store.try_acquire_authority(&[b.clone()]).unwrap();
    let b_write = AuthorityScope::identity("admission_trace_b", AuthorityMode::Write).unwrap();
    drop(
        observe_refusal(
            "kernel-resource-lock",
            [
                file(&a, "admission"),
                file(&a, "resource"),
                file(&b, "admission"),
            ],
            || f.store.try_acquire_authority(&[a.clone(), b_write.clone()]),
        )
        .unwrap_err(),
    );
    drop(blocking_reader);
    drop(f.store.try_acquire_authority(&[a, b_write]).unwrap());
}
