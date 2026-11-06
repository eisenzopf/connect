//! Bounded timing for immutable metadata retries, never physical submission.
use super::*;
use connect_store::NativeReplacementCommitted;
use std::{sync::atomic::AtomicU64, time::Instant};

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Capture,
    Activation,
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Capture => "capture",
            Self::Activation => "activation",
        }
    }
}
#[derive(Default)]
struct Counters {
    attempts: AtomicU64,
    completed: AtomicU64,
    dispatch_us: AtomicU64,
    execution_us: AtomicU64,
    authority_busy: AtomicU64,
    storage_unavailable: AtomicU64,
    permanent_refusal: AtomicU64,
}
fn add(counter: &AtomicU64, value: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(value))
    });
}
fn micros(start: Instant) -> u64 {
    start.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}

/// Preserve the original five-second metadata retry policy exactly. The body
/// factory may only clone immutable metadata for the next Store call. It must
/// never reserve an attempt or submit a Core effect.
pub(super) async fn metadata<T, F, G>(
    stop: &tokio_util::sync::CancellationToken,
    capture: &Arc<NativeReplacementCommitted>,
    phase: Phase,
    mut body: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnMut() -> G,
    G: FnOnce() -> Result<T> + Send + 'static,
{
    let counters = Arc::new(Counters::default());
    let began = Instant::now();
    let result = retry_metadata(stop, || {
        let call = body();
        let counters = counters.clone();
        async move {
            add(&counters.attempts, 1);
            let queued = Instant::now();
            store_call(move || {
                add(&counters.dispatch_us, micros(queued));
                let execution = Instant::now();
                let result = call();
                add(&counters.execution_us, micros(execution));
                if let Err(error) = &result {
                    add(
                        match error.code.as_str() {
                            "authority-busy" => &counters.authority_busy,
                            "storage-unavailable" => &counters.storage_unavailable,
                            _ => &counters.permanent_refusal,
                        },
                        1,
                    );
                }
                add(&counters.completed, 1);
                result
            })
            .await
        }
    })
    .await;
    let elapsed = micros(began);
    let dispatch = counters.dispatch_us.load(Ordering::Relaxed);
    let execution = counters.execution_us.load(Ordering::Relaxed);
    // All Store guards are gone here. No trace output or JSON serialization
    // occurs under the SQL mutex, authority custody or native registry locks.
    // Outside time includes retry sleep, async wakeup and local accounting;
    // it must not be mislabeled as only authority wait or only retry sleep.
    tracing::debug!(target: "connect_service::canonical_replacement::projection",
        operation_record = capture.activation().origin().offer_record_id(),
        phase = phase.name(), elapsed_us = elapsed, dispatch_us = dispatch,
        store_execution_us = execution,
        outside_store_us = elapsed.saturating_sub(dispatch).saturating_sub(execution),
        attempts = counters.attempts.load(Ordering::Relaxed),
        completed_calls = counters.completed.load(Ordering::Relaxed),
        authority_busy = counters.authority_busy.load(Ordering::Relaxed),
        storage_unavailable = counters.storage_unavailable.load(Ordering::Relaxed),
        permanent_refusal = counters.permanent_refusal.load(Ordering::Relaxed),
        result = result.as_ref().map(|_| "ok").unwrap_or_else(|error| error.code.as_str()),
        store_admission = %capture.projection_diagnostics(),
        "Replacement metadata retry phase observed");
    result
}

/// Numeric attribution owned by the existing original target, not a worker or
/// a reader. A fixed leaf bitset bounds output to fourteen lines per target.
#[derive(Default)]
pub(super) struct PrivateReceive {
    seen: AtomicU64,
}
impl PrivateReceive {
    pub(super) fn refused(
        &self,
        leaf: u32,
        target: &super::media::TargetPolicy,
        scope: &rvoip_core::staged_streams::MediaScope<'_>,
        source: &rvoip_core::staged_streams::MediaSource,
        medium: Option<rvoip_core::stream_lifecycle::Medium>,
        error: &rvoip_core::staged_streams::StreamControlError,
    ) {
        const LEAVES: [&str; 14] = [
            "scope",
            "readiness-gate",
            "current-preroll-source",
            "scene-registry",
            "original-media-origin",
            "current-scene-read",
            "scene-read-current",
            "selected-source-permission",
            "source-entry",
            "source-projection",
            "source-descriptor",
            "current-replacement-owners",
            "original-activation",
            "current-replacement-activation",
        ];
        if !tracing::enabled!(target:"connect_service::private_receive_refusal",tracing::Level::DEBUG)
            || leaf as usize >= LEAVES.len()
        {
            return;
        }
        let bit = 1u64 << leaf;
        if self.seen.fetch_or(bit, Ordering::Relaxed) & bit != 0 {
            return;
        }
        let root = target.root.upgrade();
        let receiver = target
            .readiness
            .as_ref()
            .and_then(|gate| gate.diagnostic_video_receiver(source));
        tracing::debug!(target:"connect_service::private_receive_refusal",
            leaf=LEAVES[leaf as usize], connection=%scope.connection,
            target_generation=target.pending.generation(),
            operation_id=?root.as_ref().map(|root|&root.handle.intent().operation_id),
            source_connection=%source.connection, source_generation=source.connection_generation,
            source_logical=%source.logical_id, source_transport=%source.transport_id,
            source_stream_generation=source.stream_generation,
            medium=?medium.or(receiver.map(|_|rvoip_core::stream_lifecycle::Medium::Video)),
            target_logical=?receiver,
            preroll_current=target.readiness.as_ref().and_then(|gate|gate.preroll.get()).is_some_and(|p|p.is_current()),
            core_committed=root.as_ref().is_some_and(|root|root.handle.is_committed()),
            root_cancelled=root.as_ref().is_none_or(|root|root.handle.cancellation().is_cancelled()),
            ?error,"Original private receive authority leaf refused");
    }
}
