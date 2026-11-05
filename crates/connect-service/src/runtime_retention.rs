//! Bounded diagnostic attribution of original blocking calls; no owned jobs.
//! Enabled only by the `connect_service::runtime_retention=debug` log target.
//! Completion uses atomics even when the returned value still owns readers.
use rvoip_core::runtime_diagnostics::{current_os_thread_identity, OsThreadIdentity};
use serde::Serialize;
use std::cell::OnceCell;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

const THREAD_LIMIT: usize = 128;
const KIND_COUNT: usize = 7;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Health,
    Application,
    ReconnectScan,
    ReconnectReconcile,
    LifecycleScan,
    CodecScan,
    NativeControlPoll,
}
impl Kind {
    const ALL: [Self; KIND_COUNT] = [
        Self::Health,
        Self::Application,
        Self::ReconnectScan,
        Self::ReconnectReconcile,
        Self::LifecycleScan,
        Self::CodecScan,
        Self::NativeControlPoll,
    ];
    fn callsite(self) -> &'static str {
        match self {
            Self::Health => "service.run:health",
            Self::Application => "service.run:application",
            Self::ReconnectScan => "session_reconnect:page",
            Self::ReconnectReconcile => "session_reconnect:reconcile",
            Self::LifecycleScan => "session_lifecycle:hint_page",
            Self::CodecScan => "session_codecs:reap_recover_reconcile_targets",
            Self::NativeControlPoll => "native_control:original_future_poll",
        }
    }
}
struct Counter {
    started: AtomicU64,
    returned: AtomicU64,
    unwound: AtomicU64,
    in_flight: AtomicU64,
    last_start_ms: AtomicU64,
    last_exit_ms: AtomicU64,
}
impl Counter {
    const fn new() -> Self {
        Self {
            started: AtomicU64::new(0),
            returned: AtomicU64::new(0),
            unwound: AtomicU64::new(0),
            in_flight: AtomicU64::new(0),
            last_start_ms: AtomicU64::new(0),
            last_exit_ms: AtomicU64::new(0),
        }
    }
}
struct ThreadSlot {
    identity: OnceLock<OsThreadIdentity>,
    exited: AtomicBool,
    created_ms: AtomicU64,
    exited_ms: AtomicU64,
    calls: [Counter; KIND_COUNT],
}
impl ThreadSlot {
    const fn new() -> Self {
        Self {
            identity: OnceLock::new(),
            exited: AtomicBool::new(false),
            created_ms: AtomicU64::new(0),
            exited_ms: AtomicU64::new(0),
            calls: [const { Counter::new() }; KIND_COUNT],
        }
    }
}
struct Registry<const N: usize> {
    next: AtomicUsize,
    omitted: AtomicU64,
    slots: [ThreadSlot; N],
}
impl<const N: usize> Registry<N> {
    const fn new() -> Self {
        Self {
            next: AtomicUsize::new(0),
            omitted: AtomicU64::new(0),
            slots: [const { ThreadSlot::new() }; N],
        }
    }
    fn register(&self, identity: Option<OsThreadIdentity>, now: u64) -> Option<usize> {
        let Some(identity) = identity else {
            increment(&self.omitted);
            return None;
        };
        let index = self
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                (value < N).then_some(value + 1)
            });
        let Ok(index) = index else {
            increment(&self.omitted);
            return None;
        };
        let slot = &self.slots[index];
        slot.created_ms.store(now, Ordering::Relaxed);
        if slot.identity.set(identity).is_err() {
            increment(&self.omitted);
            return None;
        }
        Some(index)
    }
    fn enter(&self, index: usize, kind: Kind, now: u64) -> Call<'_> {
        let counter = &self.slots[index].calls[kind as usize];
        increment(&counter.started);
        increment(&counter.in_flight);
        counter.last_start_ms.store(now, Ordering::Relaxed);
        Call {
            counter,
            returned: false,
        }
    }
}
static REGISTRY: Registry<THREAD_LIMIT> = Registry::new();
static START: OnceLock<Instant> = OnceLock::new();
fn elapsed_ms() -> u64 {
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn increment(value: &AtomicU64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(1))
    });
}
struct OriginalThread(Option<usize>);
impl Drop for OriginalThread {
    fn drop(&mut self) {
        if let Some(index) = self.0 {
            let slot = &REGISTRY.slots[index];
            slot.exited_ms.store(elapsed_ms(), Ordering::Relaxed);
            slot.exited.store(true, Ordering::Release);
        }
    }
}
thread_local! {
    static ORIGINAL_THREAD: OnceCell<OriginalThread> = const { OnceCell::new() };
}
struct Call<'a> {
    counter: &'a Counter,
    returned: bool,
}
impl Drop for Call<'_> {
    fn drop(&mut self) {
        increment(if self.returned {
            &self.counter.returned
        } else {
            &self.counter.unwound
        });
        self.counter
            .last_exit_ms
            .store(elapsed_ms(), Ordering::Relaxed);
        self.counter.in_flight.fetch_sub(1, Ordering::Release);
    }
}
pub(crate) fn enabled() -> bool {
    tracing::enabled!(target: "connect_service::runtime_retention", tracing::Level::DEBUG)
}
/// The returned JoinHandle is the original Tokio job; no supervisor, wrapper
/// task, timeout, queue or replacement work is introduced.
pub(crate) fn spawn_blocking<F, R>(kind: Kind, original: F) -> tokio::task::JoinHandle<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    if !enabled() {
        return tokio::task::spawn_blocking(original);
    }
    tokio::task::spawn_blocking(move || {
        let index = ORIGINAL_THREAD.with(|state| {
            state
                .get_or_init(|| {
                    OriginalThread(REGISTRY.register(current_os_thread_identity(), elapsed_ms()))
                })
                .0
        });
        let mut call = index.map(|index| REGISTRY.enter(index, kind, elapsed_ms()));
        if call.is_none() {
            increment(&REGISTRY.omitted);
        }
        let value = original();
        if let Some(call) = &mut call {
            call.returned = true;
        }
        // Only atomic completion accounting runs while `value` remains owned.
        drop(call);
        value
    })
}
#[derive(Serialize)]
struct CallSnapshot {
    source_callsite: &'static str,
    started: u64,
    returned: u64,
    unwound: u64,
    in_flight: u64,
    last_start_ms: u64,
    last_exit_ms: u64,
}
#[derive(Serialize)]
struct ThreadSnapshot {
    incarnation: usize,
    os_tid: u64,
    darwin_pthread_handle: Option<u64>,
    created_ms: u64,
    thread_exited: bool,
    exited_ms: Option<u64>,
    calls: Vec<CallSnapshot>,
}
fn thread_snapshots<const N: usize>(registry: &Registry<N>) -> Vec<ThreadSnapshot> {
    registry
        .slots
        .iter()
        .enumerate()
        .filter_map(|(index, slot)| {
            let identity = slot.identity.get()?;
            let exited = slot.exited.load(Ordering::Acquire);
            Some(ThreadSnapshot {
                incarnation: index + 1,
                os_tid: identity.tid,
                darwin_pthread_handle: identity.pthread_handle,
                created_ms: slot.created_ms.load(Ordering::Relaxed),
                thread_exited: exited,
                exited_ms: exited.then(|| slot.exited_ms.load(Ordering::Relaxed)),
                calls: Kind::ALL
                    .into_iter()
                    .filter_map(|kind| {
                        let c = &slot.calls[kind as usize];
                        let started = c.started.load(Ordering::Relaxed);
                        (started > 0).then(|| CallSnapshot {
                            source_callsite: kind.callsite(),
                            started,
                            returned: c.returned.load(Ordering::Relaxed),
                            unwound: c.unwound.load(Ordering::Relaxed),
                            in_flight: c.in_flight.load(Ordering::Acquire),
                            last_start_ms: c.last_start_ms.load(Ordering::Relaxed),
                            last_exit_ms: c.last_exit_ms.load(Ordering::Relaxed),
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}
/// Existing authenticated administrator health boundary only. Called after its
/// original Store closure has returned and after resource observer locks release.
/// The diagnostic result is not part of readiness or idle acceptance.
pub(crate) fn observe(runtime: Option<&crate::canonical_runtime::Runtime>) {
    if !enabled() {
        return;
    }
    let current = tokio::runtime::Handle::try_current().ok();
    let metrics = current.as_ref().map(|handle| handle.metrics());
    let threads = thread_snapshots(&REGISTRY);
    let registered = REGISTRY.next.load(Ordering::Relaxed);
    let sample = serde_json::json!({
        "format": "connect/server-retention-attribution/1",
        "pid": std::process::id(), "observed_ms": elapsed_ms(),
        "scope": "process counters; observational, not atomic or acceptance evidence",
        "thread_limit": THREAD_LIMIT, "callsite_limit": KIND_COUNT,
        "omitted_identity_or_call_observations": REGISTRY.omitted.load(Ordering::Relaxed),
        "tokio_async_worker_count": metrics.as_ref().map(|m| m.num_workers()),
        "tokio_alive_tasks": metrics.as_ref().map(|m| m.num_alive_tasks()),
        "registering_or_unavailable_slots": registered.saturating_sub(threads.len()),
        "threads": threads,
        "registries": runtime.map(|runtime| runtime.retention_counts()),
        "limitations": ["Only listed original callsites are attributed; an idle registered thread may execute other work",
            "Atomic fields are individually sampled, not a transaction; absent or contended registries are null",
            "Map entries and Weak owners are labeled separately; no heap or RSS ownership is inferred",
            "Thread exit is original TLS destruction, not a fabricated job join"]
    });
    tracing::debug!(target: "connect_service::runtime_retention", snapshot = %sample,
        "Original server retention attribution");
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(tid: u64) -> OsThreadIdentity {
        OsThreadIdentity {
            tid,
            pthread_handle: None,
        }
    }
    #[test]
    fn fixed_registry_never_reuses_old_os_identity_incarnations() {
        let registry = Registry::<2>::new();
        assert_eq!(registry.register(Some(identity(10)), 0), Some(0));
        registry.slots[0].exited.store(true, Ordering::Release);
        assert_eq!(registry.register(Some(identity(10)), 1), Some(1));
        assert_eq!(registry.register(Some(identity(11)), 2), None);
        assert_eq!(registry.omitted.load(Ordering::Relaxed), 1);
        let rows = thread_snapshots(&registry);
        assert!(rows[0].thread_exited);
        assert!(!rows[1].thread_exited);
        assert_ne!(rows[0].incarnation, rows[1].incarnation);
    }
    #[test]
    fn missing_os_identity_is_an_omission_not_a_thread_or_zero_pass() {
        let registry = Registry::<1>::new();
        assert_eq!(registry.register(None, 0), None);
        assert!(thread_snapshots(&registry).is_empty());
        assert_eq!(registry.omitted.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn original_return_and_unwind_both_release_inflight_without_false_success() {
        let registry = Registry::<1>::new();
        let slot = registry.register(Some(identity(20)), 0).unwrap();
        let mut original = registry.enter(slot, Kind::Health, 0);
        assert_eq!(thread_snapshots(&registry)[0].calls[0].in_flight, 1);
        original.returned = true;
        drop(original);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _original = registry.enter(slot, Kind::Health, 1);
            panic!("original blocking failure");
        }));
        assert!(failed.is_err());
        let row = &thread_snapshots(&registry)[0].calls[0];
        assert_eq!(
            (row.started, row.returned, row.unwound, row.in_flight),
            (2, 1, 1, 0)
        );
    }
}
