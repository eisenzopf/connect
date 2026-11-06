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
const PRIVATE_RECEIVE_TARGET: &str = "connect_service::canonical_replacement";
const PRIVATE_RECEIVE_LEAVES: [&str; 14] = [
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
impl PrivateReceive {
    fn observe(&self, leaf: u32, emit: impl FnOnce(&'static str)) {
        if !tracing::enabled!(target:PRIVATE_RECEIVE_TARGET,tracing::Level::DEBUG)
            || leaf as usize >= PRIVATE_RECEIVE_LEAVES.len()
        {
            return;
        }
        let bit = 1u64 << leaf;
        if self.seen.fetch_or(bit, Ordering::Relaxed) & bit != 0 {
            return;
        }
        emit(PRIVATE_RECEIVE_LEAVES[leaf as usize]);
    }
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
        self.observe(leaf, |leaf| {
        let root = target.root.upgrade();
        let receiver = target
            .readiness
            .as_ref()
            .and_then(|gate| gate.diagnostic_video_receiver(source));
        tracing::debug!(target:PRIVATE_RECEIVE_TARGET,
            leaf, connection=%scope.connection,
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
        });
    }
}

#[cfg(test)]
mod private_receive_filter_tests {
    use super::*;
    use std::sync::{atomic::AtomicUsize, Arc};
    use tracing_subscriber::{
        layer::{Context, SubscriberExt},
        Layer,
    };
    struct CountEvents(Arc<AtomicUsize>);
    impl<S: tracing::Subscriber> Layer<S> for CountEvents {
        fn on_event(&self, _: &tracing::Event<'_>, _: Context<'_, S>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn required_runner_filters() -> [String; 2] {
        let browser = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/rehearse-gate6-browser-matrix.py"
        ));
        let native = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/check-transport-fixtures.py"
        ));
        let browser_filter = browser
            .lines()
            .find(|line| line.contains("env[\"RUST_LOG\"] = "))
            .unwrap()
            .split('"')
            .nth(3)
            .unwrap();
        let native_base = native
            .lines()
            .find(|line| line.contains("server_env = dict(env, RUST_LOG="))
            .unwrap()
            .split('"')
            .nth(1)
            .unwrap();
        let native_append = native
            .lines()
            .find(|line| {
                line.contains("server_env[\"RUST_LOG\"] += ")
                    && line.contains("canonical_replacement=debug")
            })
            .unwrap()
            .split('"')
            .nth(3)
            .unwrap();
        let native_filter = format!("{native_base}{native_append}");
        [browser_filter.to_owned(), native_filter]
    }
    #[test]
    fn required_native_and_browser_runner_filters_admit_each_private_receive_leaf_once() {
        for directives in required_runner_filters() {
            let diagnostic = PrivateReceive::default();
            let events = Arc::new(AtomicUsize::new(0));
            // Disabled collection must not consume an original owner's budget.
            tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
                diagnostic.observe(0,|leaf|
                tracing::debug!(target:PRIVATE_RECEIVE_TARGET,leaf,"Private receive test observation"))
            });
            let filter = tracing_subscriber::EnvFilter::try_new(&directives).unwrap();
            let subscriber = tracing_subscriber::registry()
                .with(filter)
                .with(CountEvents(events.clone()));
            tracing::subscriber::with_default(subscriber, || {
                for _ in 0..2 {
                    for leaf in 0..PRIVATE_RECEIVE_LEAVES.len() as u32 {
                        diagnostic.observe(leaf,|leaf|tracing::debug!(target:PRIVATE_RECEIVE_TARGET,leaf,"Private receive test observation"));
                    }
                }
                diagnostic.observe(PRIVATE_RECEIVE_LEAVES.len() as u32, |_| {
                    panic!("out-of-range diagnostic emitted")
                });
            });
            assert_eq!(
                events.load(Ordering::Relaxed),
                14,
                "actual runner log filter omitted or flooded bounded leaves: {directives}"
            );
            assert_eq!(diagnostic.seen.load(Ordering::Relaxed).count_ones(), 14);
        }
    }
    #[test]
    fn required_native_and_browser_filters_admit_original_six_terminal_scene_stages() {
        const TARGET: &str = "rvoip_core::audio_summary";
        // The prepared upstream source is part of the locked dependency graph.
        // Pair this exact production topic check with Core's actual emitter
        // regression; neither merely accepting a stale topic nor a parallel
        // test-only logger can qualify the combined contract.
        let production=include_str!(concat!(env!("CARGO_MANIFEST_DIR"),
            "/../../.deps/rvoip/crates/foundation/rvoip-core/src/staged_streams/media_scene/tick_summary.rs"));
        assert_eq!(
            production
                .matches("target: \"rvoip_core::audio_summary\"")
                .count(),
            1
        );
        assert!(!production.contains("target: \"rvoip_core::scene_summary\""));
        for directives in required_runner_filters() {
            let events = Arc::new(AtomicUsize::new(0));
            let subscriber = tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::try_new(&directives).unwrap())
                .with(CountEvents(events.clone()));
            tracing::subscriber::with_default(subscriber, || {
                for stage in [
                    "wake-gap",
                    "members-lock-wait",
                    "members-lock-held",
                    "adopt-metadata",
                    "audio",
                    "direct",
                ] {
                    tracing::debug!(target:TARGET,stage,"Original terminal scene filter observation");
                }
                tracing::debug!(target:"rvoip_core::scene_summary","Old disabled topic must remain excluded");
            });
            assert_eq!(
                events.load(Ordering::Relaxed),
                6,
                "required actual runner filter omits terminal scene records: {directives}"
            );
        }
    }
}
