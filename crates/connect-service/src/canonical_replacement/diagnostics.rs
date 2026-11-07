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

/// Closed observational codes only. Exact message matching prevents arbitrary
/// identity/path/provider details from crossing the existing bounded event.
/// These codes never alter the original packet error, grant or retry policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum PrivateSceneBusy {
    NotAuthorityBusy,
    UnknownAuthorityBusy,
    ObserverOwnership,
    ObserverPreparingOrRetiring,
    ObserverScopeUnavailable,
    ObserverGenerationChanged,
    ObserverSlotsOccupied,
    ObserverResealOccupied,
    ReaderRegistry,
    AuthorityCurrentBusy,
    AuthorityOriginalWaitCutoff,
    RefusedSnapshotChanged,
    ObservationMetadataChanged,
    BeforeProjectionChanged,
    DuringProjectionChanged,
    ProjectionClockChanged,
    DuringAdmissionChanged,
    ClockRevalidationChanged,
    ActivationProjectionPending,
}
const PRIVATE_SCENE_BUSY_DETAILS: &[(&str, PrivateSceneBusy)] = &[
    (
        "Packet observer ownership is busy",
        PrivateSceneBusy::ObserverOwnership,
    ),
    (
        "Packet observers are preparing or retiring",
        PrivateSceneBusy::ObserverPreparingOrRetiring,
    ),
    (
        "No current physical packet observer scope",
        PrivateSceneBusy::ObserverScopeUnavailable,
    ),
    (
        "Original packet observer generation changed",
        PrivateSceneBusy::ObserverGenerationChanged,
    ),
    (
        "All current packet observers are busy",
        PrivateSceneBusy::ObserverSlotsOccupied,
    ),
    (
        "Original packet observer is busy during snapshot reseal",
        PrivateSceneBusy::ObserverResealOccupied,
    ),
    (
        "Current authority reader registry is busy",
        PrivateSceneBusy::ReaderRegistry,
    ),
    // Existing Store text is shared by file-admission and file-identity-map
    // exclusion. Do not invent a distinction the original error does not make.
    (
        "Current authority is busy",
        PrivateSceneBusy::AuthorityCurrentBusy,
    ),
    (
        "Current authority is busy; retry the same logical operation",
        PrivateSceneBusy::AuthorityOriginalWaitCutoff,
    ),
    (
        "Current packet state changed during refused resolution",
        PrivateSceneBusy::RefusedSnapshotChanged,
    ),
    (
        "Observation metadata changed",
        PrivateSceneBusy::ObservationMetadataChanged,
    ),
    (
        "Current packet state changed before projection",
        PrivateSceneBusy::BeforeProjectionChanged,
    ),
    (
        "Current packet state changed during projection",
        PrivateSceneBusy::DuringProjectionChanged,
    ),
    (
        "Current packet clock kept changing during projection",
        PrivateSceneBusy::ProjectionClockChanged,
    ),
    (
        "Current packet state changed during admission",
        PrivateSceneBusy::DuringAdmissionChanged,
    ),
    (
        "Current packet state changed during clock revalidation",
        PrivateSceneBusy::ClockRevalidationChanged,
    ),
    (
        "Exact replacement activation is awaiting durable projection",
        PrivateSceneBusy::ActivationProjectionPending,
    ),
];
impl PrivateSceneBusy {
    pub(super) fn from_error(error: &connect_protocol::Error) -> Self {
        if error.code != "authority-busy" {
            return Self::NotAuthorityBusy;
        }
        PRIVATE_SCENE_BUSY_DETAILS
            .iter()
            .find_map(|(message, kind)| (*message == error.message).then_some(*kind))
            .unwrap_or(Self::UnknownAuthorityBusy)
    }
    fn name(self) -> &'static str {
        match self {
            Self::NotAuthorityBusy => "not-authority-busy",
            Self::UnknownAuthorityBusy => "unknown-authority-busy",
            Self::ObserverOwnership => "observer-ownership",
            Self::ObserverPreparingOrRetiring => "observer-preparing-or-retiring",
            Self::ObserverScopeUnavailable => "observer-scope-unavailable",
            Self::ObserverGenerationChanged => "observer-generation-changed",
            Self::ObserverSlotsOccupied => "observer-slots-occupied",
            Self::ObserverResealOccupied => "observer-original-slot-reseal-occupied",
            Self::ReaderRegistry => "authority-reader-registry",
            Self::AuthorityCurrentBusy => "authority-current-busy-file-or-identity-registry",
            Self::AuthorityOriginalWaitCutoff => "authority-original-wait-cutoff",
            Self::RefusedSnapshotChanged => "refused-snapshot-changed",
            Self::ObservationMetadataChanged => "observation-metadata-changed",
            Self::BeforeProjectionChanged => "snapshot-before-projection-changed",
            Self::DuringProjectionChanged => "snapshot-during-projection-changed",
            Self::ProjectionClockChanged => "snapshot-projection-clock-changed",
            Self::DuringAdmissionChanged => "snapshot-during-admission-changed",
            Self::ClockRevalidationChanged => "snapshot-clock-revalidation-changed",
            Self::ActivationProjectionPending => "activation-projection-pending",
        }
    }
}
/// The Store call has returned and dropped its owned guards before this
/// classifier sees Err. Only this Copy enum escapes, never the raw error text.
/// Positive reads and the preexisting packet error mapping remain unchanged.
#[track_caller]
pub(super) fn private_scene_result<T>(
    result: connect_protocol::Result<T>,
    detail: &mut Option<PrivateSceneBusy>,
) -> rvoip_core::staged_streams::ControlResult<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            *detail = Some(PrivateSceneBusy::from_error(&error));
            Err(crate::canonical_media::packet_authority_error(error))
        }
    }
}
struct PrivateReceiveFields<'a> {
    connection: &'a str,
    target_generation: u64,
    operation_id: Option<&'a dyn std::fmt::Debug>,
    source_connection: &'a str,
    source_generation: u64,
    source_logical: &'a str,
    source_transport: &'a str,
    source_stream_generation: u64,
    medium: Option<rvoip_core::stream_lifecycle::Medium>,
    target_logical: Option<&'a str>,
    preroll_current: bool,
    core_committed: bool,
    root_cancelled: bool,
}
/// One production emitter shared with the actual runner-filter tests. Caller
/// must first pass the existing fixed leaf budget, after admission guards drop.
fn emit_private_receive(
    leaf: &'static str,
    fields: &PrivateReceiveFields<'_>,
    error: &rvoip_core::staged_streams::StreamControlError,
    store_busy: Option<PrivateSceneBusy>,
) {
    tracing::debug!(target:PRIVATE_RECEIVE_TARGET,
        leaf, connection=%fields.connection, target_generation=fields.target_generation,
        operation_id=?fields.operation_id,
        source_connection=%fields.source_connection, source_generation=fields.source_generation,
        source_logical=%fields.source_logical, source_transport=%fields.source_transport,
        source_stream_generation=fields.source_stream_generation,
        medium=?fields.medium, target_logical=?fields.target_logical,
        preroll_current=fields.preroll_current, core_committed=fields.core_committed,
        root_cancelled=fields.root_cancelled,
        store_busy=store_busy.map(PrivateSceneBusy::name).unwrap_or("not-observed"),
        ?error,"Original private receive authority leaf refused");
}

/// Numeric attribution owned by the existing original target, not a worker or
/// a reader. A fixed leaf bitset bounds output to fourteen lines per target.
#[derive(Default)]
pub(super) struct PrivateReceive {
    seen: AtomicU64,
    started: [AtomicU64; 2],
    receive: [ReceiveCounters; 4],
    preparation_started: [AtomicU64; 2],
    preparation: [PreparationCounters; 2],
    io: std::sync::OnceLock<Arc<rvoip_uctp::staged_media::StagedRtpCounters>>,
    summary_emitted: AtomicBool,
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
        store_busy: Option<PrivateSceneBusy>,
    ) {
        self.observe(leaf, |leaf| {
            let root = target.root.upgrade();
            let receiver = target
                .readiness
                .as_ref()
                .and_then(|gate| gate.diagnostic_video_receiver(source));
            let fields = PrivateReceiveFields {
                connection: scope.connection.as_str(),
                target_generation: target.pending.generation(),
                operation_id: root
                    .as_ref()
                    .map(|root| &root.handle.intent().operation_id as &dyn std::fmt::Debug),
                source_connection: source.connection.as_str(),
                source_generation: source.connection_generation,
                source_logical: source.logical_id.as_str(),
                source_transport: source.transport_id.as_str(),
                source_stream_generation: source.stream_generation,
                medium: medium.or(receiver.map(|_| rvoip_core::stream_lifecycle::Medium::Video)),
                target_logical: receiver,
                preroll_current: target
                    .readiness
                    .as_ref()
                    .and_then(|gate| gate.preroll.get())
                    .is_some_and(|p| p.is_current()),
                core_committed: root.as_ref().is_some_and(|root| root.handle.is_committed()),
                root_cancelled: root
                    .as_ref()
                    .is_none_or(|root| root.handle.cancellation().is_cancelled()),
            };
            emit_private_receive(leaf, &fields, error, store_busy);
        });
    }
}

const PRIVATE_BUSY_CLASSES: [PrivateSceneBusy; 19] = [
    PrivateSceneBusy::NotAuthorityBusy,
    PrivateSceneBusy::UnknownAuthorityBusy,
    PrivateSceneBusy::ObserverOwnership,
    PrivateSceneBusy::ObserverPreparingOrRetiring,
    PrivateSceneBusy::ObserverScopeUnavailable,
    PrivateSceneBusy::ObserverGenerationChanged,
    PrivateSceneBusy::ObserverSlotsOccupied,
    PrivateSceneBusy::ObserverResealOccupied,
    PrivateSceneBusy::ReaderRegistry,
    PrivateSceneBusy::AuthorityCurrentBusy,
    PrivateSceneBusy::AuthorityOriginalWaitCutoff,
    PrivateSceneBusy::RefusedSnapshotChanged,
    PrivateSceneBusy::ObservationMetadataChanged,
    PrivateSceneBusy::BeforeProjectionChanged,
    PrivateSceneBusy::DuringProjectionChanged,
    PrivateSceneBusy::ProjectionClockChanged,
    PrivateSceneBusy::DuringAdmissionChanged,
    PrivateSceneBusy::ClockRevalidationChanged,
    PrivateSceneBusy::ActivationProjectionPending,
];
const POLICY_ERROR_CODES: [&str; 10] = [
    "model",
    "unsupported",
    "authority",
    "not-recipient",
    "cancelled",
    "deadline",
    "not-ready",
    "metadata-busy",
    "backpressure",
    "cleanup-unconfirmed",
];
#[derive(Default)]
struct ReceiveCounters {
    completed_by_entry: [AtomicU64; 2],
    authorized: AtomicU64,
    refused_by_leaf: [AtomicU64; 14],
    store_busy: [AtomicU64; 19],
}
#[derive(Default)]
struct PreparationCounters {
    completed: AtomicU64,
    authorized: AtomicU64,
    errors: [AtomicU64; 10],
}
#[derive(Debug)]
struct ReceiveObservation {
    medium: &'static str,
    completed_by_entry: [u64; 2],
    authorized: u64,
    refused_by_leaf: [u64; 14],
    store_busy: [u64; 19],
}
#[derive(Debug)]
struct PreparationObservation {
    output: bool,
    started: u64,
    completed: u64,
    in_flight_sample: u64,
    authorized: u64,
    errors: [u64; 10],
}
#[derive(Debug)]
struct DeliveryObservation {
    entry_names: [&'static str; 2],
    started_by_entry: [u64; 2],
    in_flight_sample: u64,
    leaf_names: [&'static str; 14],
    busy_names: [&'static str; 19],
    receive: [ReceiveObservation; 4],
    preparation_error_codes: [&'static str; 10],
    preparation: [PreparationObservation; 2],
    native_rtp_io: Option<[rvoip_uctp::staged_media::RtpIoObservation; 3]>,
    private_video_route_phases: Option<
        Vec<rvoip_core::staged_streams::direct_subscriptions::PrivateVideoDeliveryObservation>,
    >,
    saturated: bool,
}
fn policy_error(error: &rvoip_core::staged_streams::StreamControlError) -> usize {
    use rvoip_core::staged_streams::StreamControlError as E;
    match error {
        E::Model(_) => 0,
        E::Unsupported => 1,
        E::Authority => 2,
        E::NotRecipient => 3,
        E::Cancelled => 4,
        E::Deadline => 5,
        E::NotReady => 6,
        E::MetadataBusy => 7,
        E::Backpressure => 8,
        E::CleanupUnconfirmed => 9,
    }
}
impl PrivateReceive {
    /// Capture once after the original installation. This Arc retains numeric
    /// metadata alone, never authority, a port, a transport or a worker.
    pub(super) fn install_io(
        &self,
        counters: Option<Arc<rvoip_uctp::staged_media::StagedRtpCounters>>,
    ) {
        if let Some(counters) = counters {
            let _ = self.io.set(counters);
        }
    }
    pub(super) fn begin_receive(&self, held: bool) {
        add(&self.started[usize::from(held)], 1);
    }
    pub(super) fn receive_result<T>(
        &self,
        held: bool,
        medium: Option<rvoip_core::stream_lifecycle::Medium>,
        leaf: u32,
        result: &rvoip_core::staged_streams::ControlResult<T>,
        busy: Option<PrivateSceneBusy>,
    ) {
        use rvoip_core::stream_lifecycle::Medium;
        // Do not look up a kind earlier than the existing policy does. Early
        // refusals remain unknown; they cannot be mislabeled as video writes.
        let index = match medium {
            Some(Medium::Audio) => 1,
            Some(Medium::Video) => 2,
            Some(Medium::Screen) => 3,
            _ => 0,
        };
        let counters = &self.receive[index];
        if result.is_ok() {
            add(&counters.authorized, 1);
        } else if let Some(counter) = counters.refused_by_leaf.get(leaf as usize) {
            add(counter, 1);
        }
        if let Some(busy) = busy {
            add(&counters.store_busy[busy as usize], 1);
        }
        add(&counters.completed_by_entry[usize::from(held)], 1);
    }
    pub(super) fn begin_preparation(&self, output: bool) {
        add(&self.preparation_started[usize::from(output)], 1);
    }
    pub(super) fn preparation_result<T>(
        &self,
        output: bool,
        result: &rvoip_core::staged_streams::ControlResult<T>,
    ) {
        let counters = &self.preparation[usize::from(output)];
        match result {
            Ok(_) => add(&counters.authorized, 1),
            Err(error) => add(&counters.errors[policy_error(error)], 1),
        }
        add(&counters.completed, 1);
    }
    fn snapshot(&self) -> DeliveryObservation {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        let started_by_entry = self.started.each_ref().map(load);
        let receive = std::array::from_fn(|index| {
            let counters = &self.receive[index];
            ReceiveObservation {
                medium: ["unknown", "audio", "video", "screen"][index],
                completed_by_entry: counters.completed_by_entry.each_ref().map(load),
                authorized: load(&counters.authorized),
                refused_by_leaf: counters.refused_by_leaf.each_ref().map(load),
                store_busy: counters.store_busy.each_ref().map(load),
            }
        });
        let preparation = std::array::from_fn(|index| {
            let counters = &self.preparation[index];
            let started = load(&self.preparation_started[index]);
            let completed = load(&counters.completed);
            PreparationObservation {
                output: index == 1,
                started,
                completed,
                in_flight_sample: started.saturating_sub(completed),
                authorized: load(&counters.authorized),
                errors: counters.errors.each_ref().map(load),
            }
        });
        let started = started_by_entry
            .iter()
            .copied()
            .fold(0u64, u64::saturating_add);
        let completed = receive
            .iter()
            .flat_map(|entry| entry.completed_by_entry)
            .fold(0u64, u64::saturating_add);
        let saturated = started_by_entry.contains(&u64::MAX)
            || receive.iter().any(|entry| {
                entry.completed_by_entry.contains(&u64::MAX)
                    || entry.authorized == u64::MAX
                    || entry.refused_by_leaf.contains(&u64::MAX)
                    || entry.store_busy.contains(&u64::MAX)
            })
            || preparation.iter().any(|entry| {
                [entry.started, entry.completed, entry.authorized].contains(&u64::MAX)
                    || entry.errors.contains(&u64::MAX)
            });
        DeliveryObservation {
            entry_names: ["fresh-source-context", "held-source-context"],
            started_by_entry,
            in_flight_sample: started.saturating_sub(completed),
            leaf_names: PRIVATE_RECEIVE_LEAVES,
            busy_names: PRIVATE_BUSY_CLASSES.map(PrivateSceneBusy::name),
            receive,
            preparation_error_codes: POLICY_ERROR_CODES,
            preparation,
            native_rtp_io: self.io.get().map(|counters| counters.io_snapshot()),
            private_video_route_phases: None,
            saturated,
        }
    }
    fn observe_summary(&self, emit: impl FnOnce(DeliveryObservation)) {
        if !tracing::enabled!(target: PRIVATE_RECEIVE_TARGET, tracing::Level::DEBUG)
            || self.summary_emitted.swap(true, Ordering::Relaxed)
        {
            return;
        }
        emit(self.snapshot());
    }
    /// Called only after the awaited preparation future has returned and its
    /// guards have dropped. No destructor emits while a reader might be held.
    pub(super) fn terminal(&self, target: &super::media::TargetPolicy, succeeded: bool) {
        self.observe_summary(|mut observation| {
            observation.private_video_route_phases = target.readiness.as_ref()
                .and_then(|gate| gate.preroll.get())
                .map(|preroll| preroll.delivery_observations());
            let root = target.root.upgrade();
            tracing::debug!(target: PRIVATE_RECEIVE_TARGET,
                connection=%target.pending.intent().replacement,
                target_generation=target.pending.generation(),
                operation_id=?root.as_ref().map(|root| &root.handle.intent().operation_id),
                preparation_result=if succeeded { "ok" } else { "error" },
                core_committed=root.as_ref().is_some_and(|root| root.handle.is_committed()),
                root_cancelled=root.as_ref().is_none_or(|root| root.handle.cancellation().is_cancelled()),
                concurrent_snapshot=true,
                counter_scope="installed-target-through-observation-may-include-public-writes",
                policy_unit="callbacks-not-unique-packets-or-writes",
                transport_unit="local-primary-rtp-datagrams-not-remote-receipt-or-decode",
                counts=?observation,
                "Original replacement delivery counters observed");
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
    pub(super) fn required_runner_filters() -> [String; 2] {
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
        let production = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../.deps/rvoip/crates/foundation/rvoip-core/src/staged_streams/media_scene/tick_summary.rs"
        ));
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

#[cfg(test)]
mod private_scene_busy_tests;

#[cfg(test)]
mod delivery_counter_tests {
    use super::*;
    use rvoip_core::{staged_streams::StreamControlError, stream_lifecycle::Medium};
    #[test]
    fn private_receive_counts_actual_entry_outcomes_and_unknown_before_kind_lookup() {
        let counters = PrivateReceive::default();
        counters.begin_receive(false);
        let refusal: rvoip_core::staged_streams::ControlResult<()> =
            Err(StreamControlError::Authority);
        counters.receive_result(false, None, 0, &refusal, None);
        counters.begin_receive(true);
        counters.receive_result(true, Some(Medium::Video), 13, &Ok(()), None);
        counters.begin_receive(false);
        counters.receive_result(
            false,
            Some(Medium::Audio),
            5,
            &Err::<(), _>(StreamControlError::MetadataBusy),
            Some(PrivateSceneBusy::ObserverSlotsOccupied),
        );
        counters.begin_receive(true);
        counters.receive_result(true, Some(Medium::Screen), 13, &Ok(()), None);
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.started_by_entry, [2, 2]);
        assert_eq!(snapshot.in_flight_sample, 0);
        assert_eq!(snapshot.receive[0].medium, "unknown");
        assert_eq!(snapshot.receive[0].refused_by_leaf[0], 1);
        assert_eq!(snapshot.receive[2].completed_by_entry, [0, 1]);
        assert_eq!(snapshot.receive[2].authorized, 1);
        assert_eq!(snapshot.receive[3].authorized, 1);
        assert_eq!(
            snapshot.receive[1].store_busy[PrivateSceneBusy::ObserverSlotsOccupied as usize],
            1
        );
        assert!(
            snapshot.native_rtp_io.is_none(),
            "policy authority is not a writer receipt"
        );
        assert_eq!(snapshot.preparation[0].started, 0);
        assert_eq!(snapshot.preparation[1].started, 0);
    }
    #[test]
    fn private_receive_preserves_every_original_leaf_and_closed_busy_class() {
        let counters = PrivateReceive::default();
        for leaf in 0..PRIVATE_RECEIVE_LEAVES.len() as u32 {
            counters.begin_receive(false);
            counters.receive_result(
                false,
                Some(Medium::Video),
                leaf,
                &Err::<(), _>(StreamControlError::NotReady),
                None,
            );
        }
        for kind in PRIVATE_BUSY_CLASSES {
            counters.begin_receive(true);
            counters.receive_result(
                true,
                Some(Medium::Audio),
                5,
                &Err::<(), _>(StreamControlError::MetadataBusy),
                Some(kind),
            );
        }
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.leaf_names, PRIVATE_RECEIVE_LEAVES);
        assert_eq!(snapshot.receive[2].refused_by_leaf, [1; 14]);
        assert_eq!(snapshot.receive[1].store_busy, [1; 19]);
        assert_eq!(
            snapshot.busy_names,
            PRIVATE_BUSY_CLASSES.map(PrivateSceneBusy::name)
        );
        assert_eq!(snapshot.receive[1].authorized, 0);
        assert_eq!(snapshot.receive[2].authorized, 0);
    }
    #[test]
    fn preparation_audio_is_separate_from_video_callbacks_and_transport_acceptance() {
        let counters = PrivateReceive::default();
        counters.begin_preparation(false);
        counters.preparation_result(false, &Ok(()));
        counters.begin_preparation(true);
        counters.preparation_result(true, &Err::<(), _>(StreamControlError::NotReady));
        counters.begin_preparation(true);
        counters.preparation_result(true, &Ok(()));
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.preparation[0].authorized, 1);
        assert!(!snapshot.preparation[0].output);
        assert_eq!(
            (
                snapshot.preparation[1].started,
                snapshot.preparation[1].completed,
                snapshot.preparation[1].authorized
            ),
            (2, 2, 1)
        );
        assert_eq!(snapshot.preparation[1].errors[6], 1);
        assert!(snapshot.preparation[1].output);
        assert_eq!(snapshot.preparation_error_codes[6], "not-ready");
        assert_eq!(snapshot.started_by_entry, [0, 0]);
        assert_eq!(snapshot.receive[2].authorized, 0);
        assert!(snapshot.native_rtp_io.is_none());
    }
    #[test]
    fn policy_saturation_and_overtaking_samples_never_underflow() {
        let counters = PrivateReceive::default();
        counters.started[0].store(u64::MAX, Ordering::Relaxed);
        counters.begin_receive(false);
        counters.receive[2]
            .authorized
            .store(u64::MAX, Ordering::Relaxed);
        counters.receive_result(false, Some(Medium::Video), 13, &Ok(()), None);
        let snapshot = counters.snapshot();
        assert_eq!(snapshot.started_by_entry[0], u64::MAX);
        assert_eq!(snapshot.receive[2].authorized, u64::MAX);
        assert!(snapshot.saturated);
        counters.started[0].store(0, Ordering::Relaxed);
        counters.preparation[0]
            .completed
            .store(u64::MAX, Ordering::Relaxed);
        assert_eq!(counters.snapshot().in_flight_sample, 0);
        assert_eq!(counters.snapshot().preparation[0].in_flight_sample, 0);
    }
    #[test]
    fn original_counter_handle_is_captured_once_and_owns_only_metadata() {
        let counters = PrivateReceive::default();
        let first = Arc::new(rvoip_uctp::staged_media::StagedRtpCounters::default());
        let second = Arc::new(rvoip_uctp::staged_media::StagedRtpCounters::default());
        counters.install_io(None);
        counters.install_io(Some(first.clone()));
        counters.install_io(Some(second.clone()));
        assert!(Arc::ptr_eq(counters.io.get().unwrap(), &first));
        assert_eq!(Arc::strong_count(&second), 1);
        drop(first);
        drop(second);
        let snapshot = counters.snapshot();
        assert!(snapshot.native_rtp_io.is_some());
        assert!(snapshot
            .native_rtp_io
            .unwrap()
            .iter()
            .all(|entry| entry.inputs == 0));
    }
    #[test]
    fn summary_emits_once_outside_original_guards_and_disabled_output_keeps_budget() {
        for directives in private_receive_filter_tests::required_runner_filters() {
            use tracing_subscriber::layer::SubscriberExt;
            let counters = PrivateReceive::default();
            tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), || {
                counters.observe_summary(|_| panic!("disabled summary emitted"));
            });
            assert!(!counters.summary_emitted.load(Ordering::Relaxed));
            let lock = std::sync::Mutex::new(());
            let held = lock.lock().unwrap();
            counters.begin_receive(false);
            let result: rvoip_core::staged_streams::ControlResult<()> =
                Err(StreamControlError::Authority);
            counters.receive_result(false, None, 0, &result, None);
            assert!(
                matches!(result, Err(StreamControlError::Authority)),
                "observation cannot substitute the original result"
            );
            drop(held);
            let subscriber = tracing_subscriber::registry()
                .with(tracing_subscriber::EnvFilter::try_new(&directives).unwrap());
            let calls = std::sync::atomic::AtomicUsize::new(0);
            tracing::subscriber::with_default(subscriber, || {
                counters.observe_summary(|snapshot| {
                    assert!(lock.try_lock().is_ok());
                    assert_eq!(snapshot.receive[0].refused_by_leaf[0], 1);
                    calls.fetch_add(1, Ordering::Relaxed);
                });
                counters.observe_summary(|_| panic!("terminal summary repeated"));
            });
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        }
    }
}
