//! Bounded rendezvous at the actual Service-to-Core continuation invocation.
//! No fabricated grant, reader, completion, adapter effect or shipping state.
use super::*;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparationCustodyPhase {
    Attach,
    Signal,
    AllocationCapture,
    AllocationReply,
    Activate,
    ManagedFinalPrepared,
    ManagedPostPanic,
}
impl PreparationCustodyPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Signal => "signal",
            Self::AllocationCapture => "allocation-capture",
            Self::AllocationReply => "allocation-reply",
            Self::Activate => "activate",
            Self::ManagedFinalPrepared => "managed-final-prepared",
            Self::ManagedPostPanic => "managed-post-panic",
        }
    }
}
/// Immutable original identities only. NativeMediaOrigin retains no IO guard;
/// the test must independently obtain a real current typed snapshot lease.
#[derive(Clone)]
pub struct PreparationCustodyObservation {
    pub phase: PreparationCustodyPhase,
    pub operation_id: String,
    pub record_id: String,
    pub target: Arc<NativeMediaOrigin>,
    pub source: Arc<NativeMediaOrigin>,
    pub expires_at: i64,
    pub cutoff: tokio::time::Instant,
}
pub(crate) struct Probe {
    entered: CancellationToken,
    release: CancellationToken,
    left: CancellationToken,
    observation: Mutex<Option<PreparationCustodyObservation>>,
    original_route: Mutex<Option<(Arc<NativeMediaOrigin>, Arc<NativeMediaOrigin>)>>,
    cutoff: Mutex<Option<tokio::time::Instant>>,
    actual_unpublished: std::sync::atomic::AtomicUsize,
    begin_submissions: std::sync::atomic::AtomicUsize,
    begin_unconfirmed: std::sync::atomic::AtomicBool,
    final_read: arc_swap::ArcSwapOption<FinalRead>,
    turn_resource: arc_swap::ArcSwapOption<Weak<()>>,
    post_starts: std::sync::atomic::AtomicUsize,
    post_accepted: std::sync::atomic::AtomicBool,
    aliases_gone_at_start: std::sync::atomic::AtomicBool,
    resource_alive_at_start: std::sync::atomic::AtomicBool,
    resources_gone_before_wait: std::sync::atomic::AtomicBool,
    panic_after_publication: bool,
}
pub(crate) enum FinalRead {
    Native(Weak<connect_store::NativeReplacementSceneRead>),
    Telephone(Weak<connect_store::TelephoneReplacementSceneRead>),
    Handoff(Weak<connect_store::NativeHandoffRead>),
}
impl FinalRead {
    fn released(&self) -> bool {
        match self {
            Self::Native(w) => w.strong_count() == 0,
            Self::Telephone(w) => w.strong_count() == 0,
            Self::Handoff(w) => w.strong_count() == 0,
        }
    }
}
pub struct PreparationCustodyProbe {
    probe: Arc<Probe>,
    owner: Weak<Policy>,
    key: String,
}
impl PreparationCustodyProbe {
    pub async fn entered(&self) {
        self.probe.entered.cancelled().await;
    }
    pub fn actual_begin_submissions(&self) -> usize {
        self.probe
            .begin_submissions
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn actual_begin_unconfirmed(&self) -> bool {
        self.probe
            .begin_unconfirmed
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn actual_unpublished_returns(&self) -> usize {
        self.probe
            .actual_unpublished
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn original_cutoff(&self) -> connect_protocol::Result<tokio::time::Instant> {
        self.probe
            .cutoff
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .as_ref()
            .copied()
            .ok_or_else(connect_protocol::Error::forbidden)
    }
    pub fn post_lifetime_observation(&self) -> (usize, bool, bool, bool, bool, bool) {
        (
            self.probe
                .post_starts
                .load(std::sync::atomic::Ordering::Acquire),
            self.probe
                .aliases_gone_at_start
                .load(std::sync::atomic::Ordering::Acquire),
            self.probe
                .resource_alive_at_start
                .load(std::sync::atomic::Ordering::Acquire),
            self.probe
                .resources_gone_before_wait
                .load(std::sync::atomic::Ordering::Acquire),
            self.probe
                .turn_resource
                .load_full()
                .is_some_and(|w| w.strong_count() == 0),
            self.probe
                .post_accepted
                .load(std::sync::atomic::Ordering::Acquire),
        )
    }
    pub fn final_read_released(&self) -> bool {
        self.probe
            .final_read
            .load_full()
            .is_some_and(|r| r.released())
    }
    pub fn release(&self) {
        self.probe.release.cancel();
    }
    pub async fn left(&self) {
        self.probe.left.cancelled().await;
    }
    pub fn original_route(
        &self,
    ) -> connect_protocol::Result<(Arc<NativeMediaOrigin>, Arc<NativeMediaOrigin>)> {
        self.probe
            .original_route
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .clone()
            .ok_or_else(connect_protocol::Error::forbidden)
    }
    pub fn observation(&self) -> connect_protocol::Result<PreparationCustodyObservation> {
        self.probe
            .observation
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?
            .clone()
            .ok_or_else(connect_protocol::Error::forbidden)
    }
}
impl Drop for PreparationCustodyProbe {
    fn drop(&mut self) {
        self.release();
        if let Some(owner) = self.owner.upgrade() {
            if let Ok(mut probes) = owner.preparation_custody_probes.lock() {
                if probes
                    .get(&self.key)
                    .is_some_and(|old| Weak::ptr_eq(old, &Arc::downgrade(&self.probe)))
                {
                    probes.remove(&self.key);
                }
            }
        }
    }
}
struct Leaving(Arc<Probe>);
impl Drop for Leaving {
    fn drop(&mut self) {
        self.0.left.cancel();
    }
}
impl Probe {
    pub(crate) fn note_begin_submission(&self) {
        self.begin_submissions
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    pub(crate) fn note_begin_unconfirmed(&self) {
        self.begin_unconfirmed
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn record_final_read(&self, read: FinalRead) {
        self.final_read.store(Some(Arc::new(read)));
    }
    pub(crate) fn record_turn_resource(&self, resource: Weak<()>) {
        self.turn_resource.store(Some(Arc::new(resource)));
    }
    pub(crate) fn post_start(&self) {
        self.aliases_gone_at_start.store(
            self.final_read.load_full().is_some_and(|r| r.released()),
            std::sync::atomic::Ordering::Release,
        );
        self.resource_alive_at_start.store(
            self.turn_resource
                .load_full()
                .is_some_and(|w| w.strong_count() > 0),
            std::sync::atomic::Ordering::Release,
        );
        self.post_starts
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    pub(crate) fn post_accepted(&self) {
        self.post_accepted
            .store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn post_observer_wait(&self) {
        self.resources_gone_before_wait.store(
            self.turn_resource
                .load_full()
                .is_some_and(|w| w.strong_count() == 0),
            std::sync::atomic::Ordering::Release,
        );
    }
    pub(crate) fn panic_after_publication(&self) {
        if self.panic_after_publication {
            panic!("controlled Service panic after actual managed publication/post custody");
        }
    }
    /// Only the real opaque Core StillUnpublished return records this witness.
    pub(crate) fn note_actual_unpublished(&self) {
        self.actual_unpublished
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
    pub(crate) fn set_original_route(
        &self,
        target: Arc<NativeMediaOrigin>,
        source: Arc<NativeMediaOrigin>,
    ) {
        if let Ok(mut route) = self.original_route.lock() {
            *route = Some((target, source));
        }
    }
    /// The original production Invocation remains on its owner's stack here.
    /// On cancellation/expiry return to the ORIGINAL Core call, whose normal
    /// currentness refusal is captured by that owner; never bypass its capture.
    pub(crate) async fn pause(
        self: &Arc<Self>,
        stop: &CancellationToken,
        cancel: &CancellationToken,
        cutoff: tokio::time::Instant,
        observation: Option<PreparationCustodyObservation>,
    ) {
        let _left = Leaving(self.clone());
        if let Ok(mut slot) = self.observation.lock() {
            *slot = observation;
        }
        if let Ok(mut original) = self.cutoff.lock() {
            *original = Some(cutoff);
        }
        self.entered.cancel();
        tokio::select! { biased;
            _ = stop.cancelled() => {},
            _ = cancel.cancelled() => {},
            _ = tokio::time::sleep_until(cutoff) => {},
            _ = self.release.cancelled() => {},
        }
    }
}
impl Policy {
    pub(crate) fn arm_preparation_custody_probe_for_test(
        self: &Arc<Self>,
        operation: &str,
        phase: PreparationCustodyPhase,
    ) -> connect_protocol::Result<PreparationCustodyProbe> {
        if operation.is_empty() || operation.len() > 256 {
            return Err(connect_protocol::Error::forbidden());
        }
        let key = format!("{}:{operation}", phase.name());
        let mut probes = self
            .preparation_custody_probes
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        probes.retain(|_, value| value.strong_count() != 0);
        if probes.len() >= self.capacity || probes.contains_key(&key) {
            return Err(connect_protocol::Error::forbidden());
        }
        let probe = Arc::new(Probe {
            entered: CancellationToken::new(),
            release: CancellationToken::new(),
            left: CancellationToken::new(),
            observation: Mutex::new(None),
            original_route: Mutex::new(None),
            cutoff: Mutex::new(None),
            actual_unpublished: std::sync::atomic::AtomicUsize::new(0),
            begin_submissions: std::sync::atomic::AtomicUsize::new(0),
            begin_unconfirmed: std::sync::atomic::AtomicBool::new(false),
            final_read: arc_swap::ArcSwapOption::empty(),
            turn_resource: arc_swap::ArcSwapOption::empty(),
            post_starts: std::sync::atomic::AtomicUsize::new(0),
            post_accepted: std::sync::atomic::AtomicBool::new(false),
            aliases_gone_at_start: std::sync::atomic::AtomicBool::new(false),
            resource_alive_at_start: std::sync::atomic::AtomicBool::new(false),
            resources_gone_before_wait: std::sync::atomic::AtomicBool::new(false),
            panic_after_publication: phase == PreparationCustodyPhase::ManagedPostPanic,
        });
        probes.insert(key.clone(), Arc::downgrade(&probe));
        Ok(PreparationCustodyProbe {
            probe,
            owner: Arc::downgrade(self),
            key,
        })
    }
    pub(crate) fn take_preparation_custody_probe_for_test(
        &self,
        operation: &str,
        phase: PreparationCustodyPhase,
    ) -> Option<Arc<Probe>> {
        self.preparation_custody_probes
            .lock()
            .ok()?
            .remove(&format!("{}:{operation}", phase.name()))?
            .upgrade()
    }
}
