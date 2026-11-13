//! State-change publication. SQL and file admission belong only to the owner worker.
//!
//! `SnapshotFacts::current` must use only the captured runtime/physical atomics
//! and original expiry. Publication is disabled until the coordinated-writer
//! boundary has been installed. Neither a generic metadata memo nor a hint can
//! enable this module. The runtime must retain and join `OwnedWatcher`.
use crate::{AuthorityGuardSet, AuthorityScope};
use arc_swap::ArcSwapOption;
use connect_protocol::{Error, Result};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering},
    Arc, Condvar, Mutex, Weak,
};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_PUBLICATIONS: usize = 4096;
const MAX_WRITERS: usize = 1024;
const MAX_REFRESH_PER_TURN: usize = 8;
static NEXT_SLOT: AtomicU64 = AtomicU64::new(1);
/// A metadata-map miss is not evidence of a pending kernel Writer. Deferred
/// observation preserves existing knowledge and cached, fenced authority only;
/// it never authorizes a new publication or extends an original lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeObservation {
    Clear,
    Blocked,
    Deferred,
}
type Probe = dyn Fn(&[AuthorityScope]) -> Result<ProbeObservation> + Send + Sync;
type Refresh<T> = dyn Fn() -> Result<Prepared<T>> + Send + Sync;

/// This callback is on media delivery. It must perform no SQL, filesystem,
/// registry lookup, contended mutex acquisition, or mutable authorization work.
pub(crate) trait SnapshotFacts: Send + Sync + 'static {
    fn current(&self) -> bool;
}

pub(crate) struct Prepared<T: SnapshotFacts> {
    facts: Arc<T>,
    scopes: Arc<[AuthorityScope]>,
    _guards: Arc<AuthorityGuardSet>,
    until: Instant,
}
impl<T: SnapshotFacts> Prepared<T> {
    pub(crate) fn new(
        facts: Arc<T>,
        guards: Arc<AuthorityGuardSet>,
        until: Instant,
    ) -> Result<Self> {
        let scopes = guards.snapshot_read_scopes()?;
        if scopes.is_empty() || Instant::now() >= until || !facts.current() {
            return Err(Error::forbidden());
        }
        Ok(Self {
            facts,
            scopes: scopes.into(),
            _guards: guards,
            until,
        })
    }
}
struct Generation<T: SnapshotFacts> {
    number: u64,
    prepared: Prepared<T>,
    epoch: Epoch,
}
impl<T: SnapshotFacts> Generation<T> {
    fn current(&self) -> bool {
        self.epoch.current() && self.prepared.facts.current()
    }
}
// Shared off-media lifetime flag only; it owns no Registry, file, or grant.
// One atomic read replaces packet-side Weak upgrades and two flag reads.
const DISABLED: u8 = 0;
const AVAILABLE: u8 = 1;
const STOPPED: u8 = 2;
const DROPPED: u8 = 3;
struct Availability(AtomicU8);
impl Availability {
    fn current(&self) -> bool {
        self.0.load(Ordering::Acquire) == AVAILABLE
    }
    fn check(&self) -> Result<()> {
        match self.0.load(Ordering::Acquire) {
            AVAILABLE => Ok(()),
            DROPPED => Err(Error::forbidden()),
            _ => Err(busy()),
        }
    }
    fn enable(&self) -> Result<()> {
        match self
            .0
            .compare_exchange(DISABLED, AVAILABLE, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(AVAILABLE) => Ok(()),
            Err(_) => Err(Error::forbidden()),
        }
    }
    fn disable(&self) {
        // A late invalidation must never undo permanent stop/drop.
        let _ = self
            .0
            .compare_exchange(AVAILABLE, DISABLED, Ordering::AcqRel, Ordering::Acquire);
    }
    fn stop(&self) {
        let _ = self
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state != DROPPED).then_some(STOPPED)
            });
    }
    fn stopped(&self) -> bool {
        self.0.load(Ordering::Acquire) >= STOPPED
    }
    fn dropped(&self) {
        self.0.store(DROPPED, Ordering::Release);
    }
}

/// Packet metadata only: retaining this cannot pin any authority Read fence.
#[derive(Clone)]
pub(crate) struct Epoch {
    revoked: Arc<AtomicBool>,
    availability: Arc<Availability>,
    until: Instant,
}
impl Epoch {
    pub(crate) fn current(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && Instant::now() < self.until
            && self.availability.current()
    }
    fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
}
impl<T: SnapshotFacts> Drop for Generation<T> {
    fn drop(&mut self) {
        self.epoch.revoke();
    }
}
/// Private stamp prevents queued work from borrowing a replacement publication,
/// even when that later publication happens to authorize the same logical pair.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    slot: u64,
    generation: u64,
}
pub(crate) struct Lease<T: SnapshotFacts> {
    slot: u64,
    generation: Arc<Generation<T>>,
}
impl<T: SnapshotFacts> Clone for Lease<T> {
    fn clone(&self) -> Self {
        Self {
            slot: self.slot,
            generation: self.generation.clone(),
        }
    }
}
impl<T: SnapshotFacts> Lease<T> {
    pub(crate) fn facts(&self) -> &T {
        &self.generation.prepared.facts
    }
    pub(crate) fn stamp(&self) -> Stamp {
        Stamp {
            slot: self.slot,
            generation: self.generation.number,
        }
    }
    pub(crate) fn current(&self) -> bool {
        self.generation.current()
    }
    pub(crate) fn packet_epoch(&self) -> Epoch {
        self.generation.epoch.clone()
    }
}
// A materialized NativeMediaRead retains IO custody, never the publication's
// typed facts. Erasing only this back-edge prevents NativeMediaRead -> Lease ->
// Generation -> NativeMediaRead from expanding its Send/Sync proof recursively.
// The trait object owns the SAME existing Generation allocation and Read fences.
trait GenerationCustody: Send + Sync {
    fn current(&self) -> bool;
    fn packet_epoch(&self) -> Epoch;
}
impl<T: SnapshotFacts> GenerationCustody for Generation<T> {
    fn current(&self) -> bool {
        Generation::current(self)
    }
    fn packet_epoch(&self) -> Epoch {
        self.epoch.clone()
    }
}
#[derive(Clone)]
pub(crate) struct CustodyLease {
    stamp: Stamp,
    generation: Arc<dyn GenerationCustody>,
}
impl CustodyLease {
    pub(crate) fn stamp(&self) -> Stamp {
        self.stamp
    }
    pub(crate) fn current(&self) -> bool {
        self.generation.current()
    }
    pub(crate) fn packet_epoch(&self) -> Epoch {
        self.generation.packet_epoch()
    }
}
impl<T: SnapshotFacts> Lease<T> {
    pub(crate) fn into_custody(self) -> CustodyLease {
        CustodyLease {
            stamp: self.stamp(),
            // Safe Arc unsizing: no new allocation, lock, or extra reader.
            generation: self.generation,
        }
    }
}

struct Refusal {
    error: Error,
    active: AtomicBool,
}
struct RefreshAttempt {
    sequence: Option<u64>,
    transient: bool,
    definitive: bool,
    retry_at: Instant,
    backoff: Duration,
}
struct Slot<T: SnapshotFacts> {
    id: u64,
    handles: AtomicUsize,
    generation: AtomicU64,
    publication: ArcSwapOption<Generation<T>>,
    refusal: ArcSwapOption<Refusal>,
    scope_hint: ArcSwapOption<Vec<AuthorityScope>>,
    attempt: Mutex<RefreshAttempt>,
    availability: Arc<Availability>,
    refresh: Box<Refresh<T>>,
}
pub(crate) struct Handle<T: SnapshotFacts> {
    slot: Arc<Slot<T>>,
}
impl<T: SnapshotFacts> Clone for Handle<T> {
    fn clone(&self) -> Self {
        self.slot.handles.fetch_add(1, Ordering::Relaxed);
        Self {
            slot: self.slot.clone(),
        }
    }
}
impl<T: SnapshotFacts> Drop for Handle<T> {
    fn drop(&mut self) {
        // The control worker may temporarily own an erased Slot Arc. The last
        // application handle must still revoke independently retained packet
        // metadata now, before that temporary worker ownership disappears.
        if self.slot.handles.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.slot.revoke();
        }
    }
}
impl<T: SnapshotFacts> Handle<T> {
    /// Fresh admission linearizes before a later invalidation. The retained
    /// snapshot's resource fences prevent that writer committing until IO joins.
    pub(crate) fn admit(&self) -> Result<Lease<T>> {
        self.admit_at(None)
    }
    pub(crate) fn admit_original(&self, stamp: Stamp) -> Result<Lease<T>> {
        self.admit_at(Some(stamp))
    }
    /// Lifetime observers need a definitive original refusal, not a fabricated
    /// retirement on refresh contention. This returns no new IO capability.
    pub(crate) fn observe(&self) -> Result<Lease<T>> {
        match self.admit() {
            Ok(lease) => Ok(lease),
            Err(error) => {
                if !self.slot.availability.current() {
                    return Err(error);
                }
                if let Some(refusal) = self.slot.refusal.load_full() {
                    if refusal.active.load(Ordering::Acquire) {
                        return Err(refusal.error.clone());
                    }
                }
                Err(error)
            }
        }
    }
    fn admit_at(&self, stamp: Option<Stamp>) -> Result<Lease<T>> {
        self.slot.availability.check()?;
        let generation = self.slot.publication.load_full().ok_or_else(busy)?;
        if stamp.is_some_and(|s| s.slot != self.slot.id || s.generation != generation.number)
            || !generation.current()
        {
            return Err(busy());
        }
        // Invalidation first revokes this exact object. No check-then-clone of
        // an unowned grant: the Arc already retains its complete Read interval.
        let lease = Lease {
            slot: self.slot.id,
            generation,
        };
        if !lease.current() {
            return Err(busy());
        }
        Ok(lease)
    }
}
fn busy() -> Error {
    Error::new(
        "authority-busy",
        "Current media authorization is being refreshed",
    )
}
fn intersects(left: &[AuthorityScope], right: &[AuthorityScope]) -> bool {
    // All stored hints and notification/probe inputs are normalized once.
    // Compare names without cloning their Strings for every slot/pair.
    left.iter().any(|a| right.contains(a))
}
trait ErasedSlot: Send + Sync {
    fn revoke(&self);
    fn revoke_refusal(&self);
    fn revoke_for(&self, scopes: &[AuthorityScope]);
    fn published_scopes(&self) -> Option<Arc<Vec<AuthorityScope>>>;
    fn refresh_if_missing(&self, registry: &Registry);
    // Off-media scheduling metadata only. The caller owns registry.control;
    // this never admits media, acquires SQL/files or retains a positive grant.
    fn refresh_ready(&self, state: &Control) -> bool;
}
impl<T: SnapshotFacts> ErasedSlot for Slot<T> {
    fn revoke(&self) {
        let refused = self.refusal.swap(None);
        if let Some(refused) = &refused {
            refused.active.store(false, Ordering::Release);
        }
        if let Some(current) = self.publication.load_full() {
            current.epoch.revoke();
        }
        // Only control-plane callers remove publication. A media miss must not
        // unexpectedly become the last cached fence destructor on the hot path.
        if self.publication.swap(None).is_some() || refused.is_some() {
            if let Ok(mut attempt) = self.attempt.lock() {
                attempt.sequence = None;
            }
        }
    }
    fn revoke_refusal(&self) {
        // Failed projections did not prove a complete fenced footprint. A
        // state change anywhere invalidates this denial, not healthy disjoint
        // publications. Its next attempt must not inherit a cached sequence.
        if let Some(refused) = self.refusal.swap(None) {
            refused.active.store(false, Ordering::Release);
            if let Ok(mut attempt) = self.attempt.lock() {
                attempt.sequence = None;
            }
        }
    }
    fn revoke_for(&self, scopes: &[AuthorityScope]) {
        let affected = self
            .scope_hint
            .load_full()
            .is_some_and(|hint| intersects(&hint, scopes));
        if affected {
            self.revoke();
        }
    }
    fn published_scopes(&self) -> Option<Arc<Vec<AuthorityScope>>> {
        if let Some(current) = self.publication.load_full() {
            let valid = current.current();
            // Do not pin the resource while revoking publication.
            drop(current);
            if !valid {
                self.revoke();
            }
        }
        // Names only, with no file/SQL/authority owner: keep watching an old
        // scope after withdrawal so a cross-process writer's release can wake
        // exactly one fresh projection of an otherwise unchanged route.
        self.scope_hint.load_full()
    }
    fn refresh_ready(&self, state: &Control) -> bool {
        if self.handles.load(Ordering::Acquire) == 0
            || self.publication.load().is_some()
            || !self.availability.current()
            || state.external_failed
            || self
                .scope_hint
                .load_full()
                .is_some_and(|hint| state.blocks(&hint))
        {
            return false;
        }
        // Same control -> attempt ordering and eligibility as the factory
        // path below. No eligible work means the original idle wait is kept.
        let Ok(attempt) = self.attempt.lock() else {
            return false;
        };
        if attempt.definitive && (!state.writers.is_empty() || state.external_busy()) {
            return false;
        }
        attempt.sequence != Some(state.sequence)
            || attempt.transient && Instant::now() >= attempt.retry_at
    }
    fn refresh_if_missing(&self, registry: &Registry) {
        if self.handles.load(Ordering::Acquire) == 0 || self.publication.load().is_some() {
            return;
        }
        if !registry.availability.current() {
            return;
        }
        let sequence = {
            let Ok(state) = registry.control.lock() else {
                return;
            };
            if state.external_failed
                || self
                    .scope_hint
                    .load_full()
                    .is_some_and(|hint| state.blocks(&hint))
            {
                return;
            }
            // Keep the same control -> attempt order as writer invalidation;
            // never carry an attempt guard into a later control acquisition.
            let Ok(mut attempt) = self.attempt.lock() else {
                return;
            };
            // A previous definitive error proved no complete footprint. Wait
            // for ALL announced mutations to settle before redoing that failed
            // projection. A genuinely new route can still prove exact success.
            if attempt.definitive && (!state.writers.is_empty() || state.external_busy()) {
                return;
            }
            let sequence = state.sequence;
            if attempt.sequence == Some(sequence)
                && (!attempt.transient || Instant::now() < attempt.retry_at)
            {
                return;
            }
            if attempt.sequence != Some(sequence) {
                attempt.backoff = Duration::from_millis(1);
            }
            attempt.sequence = Some(sequence);
            attempt.transient = false;
            sequence
        };
        // This closure must capture Weak<Store>, use the existing slow guarded
        // projection, and be bounded. Never capture its OwnedWatcher/runtime.
        let prepared = match (self.refresh)() {
            Ok(prepared) => prepared,
            Err(error) => {
                let transient = matches!(
                    error.code.as_str(),
                    "authority-busy" | "storage-unavailable"
                );
                if let Ok(mut attempt) = self.attempt.lock() {
                    attempt.definitive = !transient;
                    if transient {
                        attempt.transient = true;
                        attempt.retry_at = Instant::now() + attempt.backoff;
                        attempt.backoff = (attempt.backoff * 2).min(Duration::from_millis(250));
                    }
                }
                if !transient {
                    if let Ok(state) = registry.control.lock() {
                        if self.handles.load(Ordering::Acquire) > 0
                            && state.sequence == sequence
                            && state.writers.is_empty()
                            && !state.external_busy()
                            && registry.availability.current()
                        {
                            self.refusal.store(Some(Arc::new(Refusal {
                                error,
                                active: AtomicBool::new(true),
                            })));
                        }
                    }
                }
                return;
            }
        };
        if Instant::now() >= prepared.until || !prepared.facts.current() {
            return;
        }
        let Ok(state) = registry.control.lock() else {
            return;
        };
        if self.handles.load(Ordering::Acquire) == 0
            || state.blocks(&prepared.scopes)
            || !registry.availability.current()
        {
            return;
        }
        let Ok(previous) = self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        else {
            return;
        };
        if let Ok(mut attempt) = self.attempt.lock() {
            attempt.definitive = false;
        }
        self.scope_hint
            .store(Some(Arc::new(prepared.scopes.to_vec())));
        let epoch = Epoch {
            availability: self.availability.clone(),
            revoked: Arc::new(AtomicBool::new(false)),
            until: prepared.until,
        };
        let next = Arc::new(Generation {
            number: previous + 1,
            prepared,
            epoch,
        });
        // Prepared retains its complete actual Read fences through publication.
        // The same control mutex orders this exact-scope check against local
        // writer notification. A disjoint writer/sequence change cannot make
        // these fenced facts stale; unknown-footprint failures stay global.
        self.publication.store(Some(next));
        drop(state);
    }
}
impl<T: SnapshotFacts> Drop for Slot<T> {
    fn drop(&mut self) {
        self.revoke();
    }
}
struct Pending {
    id: u64,
    scopes: Vec<AuthorityScope>,
}
struct Control {
    slots: Vec<Weak<dyn ErasedSlot>>,
    writers: Vec<Pending>,
    next_writer: u64,
    sequence: u64,
    refresh_cursor: usize,
    external_scopes: Vec<AuthorityScope>,
    external_failed: bool,
}
impl Control {
    fn external_busy(&self) -> bool {
        self.external_failed || !self.external_scopes.is_empty()
    }
    fn blocks(&self, scopes: &[AuthorityScope]) -> bool {
        self.external_failed
            || intersects(&self.external_scopes, scopes)
            || self
                .writers
                .iter()
                .any(|writer| intersects(&writer.scopes, scopes))
    }
}
pub(crate) struct Registry {
    control: Mutex<Control>,
    changed: Condvar,
    probe: Box<Probe>,
    availability: Arc<Availability>,
    watcher_started: AtomicBool,
    #[cfg(test)]
    original_idle_waits: AtomicUsize,
}
impl Registry {
    // Existing synthetic kernel-probe controls retain their explicit Busy
    // meaning. Production Manager composition uses the typed observer below.
    #[cfg(test)]
    pub(crate) fn new(
        probe: impl Fn(&[AuthorityScope]) -> Result<()> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Self::new_with_probe(move |scopes| match probe(scopes) {
            Ok(()) => Ok(ProbeObservation::Clear),
            Err(error) if error.code == "authority-busy" => Ok(ProbeObservation::Blocked),
            Err(error) => Err(error),
        })
    }
    pub(crate) fn new_with_probe(
        probe: impl Fn(&[AuthorityScope]) -> Result<ProbeObservation> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            control: Mutex::new(Control {
                slots: Vec::new(),
                writers: Vec::new(),
                next_writer: 0,
                sequence: 0,
                refresh_cursor: 0,
                external_scopes: Vec::new(),
                external_failed: false,
            }),
            changed: Condvar::new(),
            probe: Box::new(probe),
            availability: Arc::new(Availability(AtomicU8::new(DISABLED))),
            watcher_started: AtomicBool::new(false),
            #[cfg(test)]
            original_idle_waits: AtomicUsize::new(0),
        })
    }
    /// Call only after the persistent coordinated-writer boundary is verified.
    /// Merely registering SQLite functions is not that proof.
    pub(crate) fn enable_after_writer_boundary(&self) -> Result<()> {
        if self.availability.stopped() || !self.watcher_started.load(Ordering::Acquire) {
            return Err(Error::forbidden());
        }
        self.availability.enable()?;
        self.changed.notify_all();
        Ok(())
    }
    pub(crate) fn stopped(&self) -> bool {
        self.availability.stopped()
    }
    pub(crate) fn register<T: SnapshotFacts>(
        self: &Arc<Self>,
        refresh: impl Fn() -> Result<Prepared<T>> + Send + Sync + 'static,
    ) -> Result<Handle<T>> {
        self.register_with_hints(&[], refresh)
    }
    pub(crate) fn register_with_hints<T: SnapshotFacts>(
        self: &Arc<Self>,
        hints: &[AuthorityScope],
        refresh: impl Fn() -> Result<Prepared<T>> + Send + Sync + 'static,
    ) -> Result<Handle<T>> {
        self.register_impl(hints, refresh, false)
    }
    pub(crate) fn try_register_with_hints<T: SnapshotFacts>(
        self: &Arc<Self>,
        hints: &[AuthorityScope],
        refresh: impl Fn() -> Result<Prepared<T>> + Send + Sync + 'static,
    ) -> Result<Handle<T>> {
        self.register_impl(hints, refresh, true)
    }
    fn register_impl<T: SnapshotFacts>(
        self: &Arc<Self>,
        hints: &[AuthorityScope],
        refresh: impl Fn() -> Result<Prepared<T>> + Send + Sync + 'static,
        immediate: bool,
    ) -> Result<Handle<T>> {
        if self.stopped() {
            return Err(Error::forbidden());
        }
        let hints = AuthorityScope::merged(
            &hints
                .iter()
                .map(AuthorityScope::read_scope)
                .collect::<Vec<_>>(),
        )?;
        let id = NEXT_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::storage())?;
        let slot = Arc::new(Slot {
            id,
            handles: AtomicUsize::new(1),
            generation: AtomicU64::new(0),
            publication: ArcSwapOption::empty(),
            refusal: ArcSwapOption::empty(),
            scope_hint: ArcSwapOption::new((!hints.is_empty()).then(|| Arc::new(hints))),
            attempt: Mutex::new(RefreshAttempt {
                sequence: None,
                transient: false,
                definitive: false,
                retry_at: Instant::now(),
                backoff: Duration::from_millis(1),
            }),
            availability: self.availability.clone(),
            refresh: Box::new(refresh),
        });
        let erased: Arc<dyn ErasedSlot> = slot.clone();
        let mut state = if immediate {
            self.control.try_lock().map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => busy(),
                std::sync::TryLockError::Poisoned(_) => Error::storage(),
            })?
        } else {
            self.control.lock().map_err(|_| Error::storage())?
        };
        state.slots.retain(|s| s.strong_count() > 0);
        if state.slots.len() >= MAX_PUBLICATIONS {
            return Err(Error::new(
                "capacity-exceeded",
                "Media snapshot publication capacity exceeded",
            ));
        }
        state.slots.push(Arc::downgrade(&erased));
        drop(state);
        self.changed.notify_all();
        Ok(Handle { slot })
    }
    /// One control-plane refresh, never called by packet admission. The
    /// original factory owns all SQL/file work outside Registry/map locks.
    /// Success is an observation only; subsequent IO must still admit normally.
    pub(crate) fn try_prepare_current<T: SnapshotFacts>(&self, handle: &Handle<T>) -> Result<()> {
        if !Arc::ptr_eq(&self.availability, &handle.slot.availability) {
            return Err(Error::forbidden());
        }
        self.availability.check()?;
        // Off-media expiry/owner retirement only. Never refresh an old queued
        // stamp or retain the temporary admission beyond this control call.
        drop(handle.slot.published_scopes());
        if handle.admit().is_ok() {
            return Ok(());
        }
        handle.slot.refresh_if_missing(self);
        drop(handle.observe()?);
        Ok(())
    }

    /// Invoke before any local writer waits for file admission; retain returned
    /// notice until its original full Write interval and SQL outcome have ended.
    pub(crate) fn before_write(
        self: &Arc<Self>,
        scopes: &[AuthorityScope],
    ) -> Result<WriterNotice> {
        // Notification names only. Actual Manager Write proofs are untouched.
        let scopes = AuthorityScope::merged(
            &scopes
                .iter()
                .map(AuthorityScope::read_scope)
                .collect::<Vec<_>>(),
        )?;
        let mut state = self.control.lock().map_err(|_| Error::storage())?;
        if state.writers.len() >= MAX_WRITERS {
            return Err(Error::new(
                "capacity-exceeded",
                "Media writer notification capacity exceeded",
            ));
        }
        state.next_writer = state
            .next_writer
            .checked_add(1)
            .ok_or_else(Error::storage)?;
        state.sequence = state.sequence.checked_add(1).ok_or_else(Error::storage)?;
        let id = state.next_writer;
        state.slots.retain(|s| s.strong_count() > 0);
        for slot in state.slots.iter().filter_map(Weak::upgrade) {
            slot.revoke_refusal();
            slot.revoke_for(&scopes);
        }
        state.writers.push(Pending { id, scopes });
        drop(state);
        self.changed.notify_all();
        Ok(WriterNotice {
            registry: Arc::downgrade(self),
            id,
        })
    }
    pub(crate) fn invalidate_all(&self) {
        self.availability.disable();
        if let Ok(mut state) = self.control.lock() {
            state.sequence = state.sequence.saturating_add(1);
            for slot in state.slots.iter().filter_map(Weak::upgrade) {
                slot.revoke();
            }
        }
        self.changed.notify_all();
    }
    // None means this whole observation is inconclusive. Preserve previous
    // blocked scopes/failure knowledge; never clear them from partial evidence.
    fn blocked_external_scopes(
        &self,
        scopes: &[AuthorityScope],
    ) -> Result<Option<Vec<AuthorityScope>>> {
        let scopes = AuthorityScope::merged(scopes)?;
        if scopes.is_empty() {
            return Ok(Some(Vec::new()));
        }
        match (self.probe)(&scopes)? {
            ProbeObservation::Clear => return Ok(Some(Vec::new())),
            ProbeObservation::Deferred => return Ok(None),
            ProbeObservation::Blocked => {}
        }
        let mut blocked = Vec::new();
        for scope in scopes {
            match (self.probe)(std::slice::from_ref(&scope))? {
                ProbeObservation::Clear => {}
                ProbeObservation::Blocked => blocked.push(scope),
                ProbeObservation::Deferred => return Ok(None),
            }
        }
        Ok(Some(blocked))
    }
    pub(crate) fn start_watcher(self: &Arc<Self>, interval: Duration) -> Result<OwnedWatcher> {
        if self.stopped()
            || self
                .watcher_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(Error::forbidden());
        }
        let interval = interval.clamp(Duration::from_millis(1), Duration::from_millis(20));
        let registry = self.clone();
        let thread = match std::thread::Builder::new()
            .name("connect-media-auth".into())
            .spawn(move || {
                let _exit = WatcherExit(registry.clone());
                while !registry.stopped() {
                    let slots = match registry.control.lock() {
                        Ok(mut state) => {
                            state.slots.retain(|s| s.strong_count() > 0);
                            state
                                .slots
                                .iter()
                                .filter_map(Weak::upgrade)
                                .collect::<Vec<_>>()
                        }
                        Err(_) => {
                            registry.availability.stop();
                            registry.invalidate_all();
                            return;
                        }
                    };
                    // One off-media fence probe per unique Read scope, before
                    // any SQL refresh. A busy coordinated cross-process writer
                    // invalidates publication, then waits only for actual IO.
                    let scopes = slots
                        .iter()
                        .filter_map(|slot| slot.published_scopes())
                        .flat_map(|scopes| scopes.to_vec())
                        .collect::<Vec<_>>();
                    let blocked = registry.blocked_external_scopes(&scopes);
                    let deferred = matches!(&blocked, Ok(None));
                    if !deferred {
                        let Ok(mut state) = registry.control.lock() else {
                            return;
                        };
                        let (external_scopes, external_failed) = match &blocked {
                            Ok(Some(scopes)) => (scopes.as_slice(), false),
                            Ok(None) => unreachable!("deferred observation handled above"),
                            Err(_) => (&[][..], true),
                        };
                        // A scope-set change matters even when both probes are
                        // busy: an opaque failed derivation may depend on either.
                        if state.external_scopes.as_slice() != external_scopes
                            || state.external_failed != external_failed
                        {
                            state.external_scopes = external_scopes.to_vec();
                            state.external_failed = external_failed;
                            state.sequence = state.sequence.saturating_add(1);
                            for slot in state.slots.iter().filter_map(Weak::upgrade) {
                                slot.revoke_refusal();
                            }
                        }
                    }
                    match &blocked {
                        Ok(Some(scopes)) if !scopes.is_empty() => {
                            for slot in &slots {
                                slot.revoke_for(scopes);
                            }
                        }
                        Ok(_) => {}
                        Err(_) => {
                            // Namespace/storage/owner failure is not a narrow
                            // busy scope. Preserve the original global refusal.
                            for slot in &slots {
                                slot.revoke();
                            }
                        }
                    }
                    // Bound new/invalidated refresh work between writer-intent
                    // probes. Factories retain their own existing short cutoff;
                    // no callback may perform an unbounded wait or network IO.
                    let cursor = match registry.control.lock() {
                        Ok(mut state) => {
                            let cursor = state.refresh_cursor;
                            state.refresh_cursor = cursor.wrapping_add(MAX_REFRESH_PER_TURN);
                            cursor
                        }
                        Err(_) => return,
                    };
                    // An inconclusive turn neither publishes nor renews any
                    // factory result. Existing publication still owns its real
                    // kernel Readers; expiry/cancellation remain immediate.
                    if !deferred && !slots.is_empty() {
                        for offset in 0..slots.len().min(MAX_REFRESH_PER_TURN) {
                            if registry.stopped() {
                                break;
                            }
                            slots[(cursor.wrapping_add(offset)) % slots.len()]
                                .refresh_if_missing(&registry);
                        }
                    }
                    let Ok(state) = registry.control.lock() else {
                        registry.availability.stop();
                        registry.invalidate_all();
                        return;
                    };
                    if registry.stopped() {
                        break;
                    }
                    // A registration/withdrawal can happen while the bounded
                    // factories run, before this worker begins waiting. Test
                    // the actual work predicate while holding the same mutex
                    // used by producers, including registrations that do not
                    // advance the writer sequence. Keep the original eight-
                    // factory limit and probe before every following turn.
                    if !deferred
                        && state
                            .slots
                            .iter()
                            .filter_map(Weak::upgrade)
                            .any(|slot| slot.refresh_ready(&state))
                    {
                        drop(state);
                        continue;
                    }
                    #[cfg(test)]
                    registry.original_idle_waits.fetch_add(1, Ordering::Relaxed);
                    if registry.changed.wait_timeout(state, interval).is_err() {
                        registry.availability.stop();
                        registry.invalidate_all();
                        return;
                    }
                }
            }) {
            Ok(thread) => thread,
            Err(_) => {
                self.watcher_started.store(false, Ordering::Release);
                return Err(Error::storage());
            }
        };
        Ok(OwnedWatcher {
            registry: self.clone(),
            thread: Some(thread),
        })
    }
}
impl Drop for Registry {
    fn drop(&mut self) {
        self.availability.dropped();
        let state = self
            .control
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for slot in state.slots.iter().filter_map(Weak::upgrade) {
            slot.revoke();
        }
    }
}
impl crate::RuntimeRetirementSignal for Registry {
    fn retire(&self) {
        self.availability.stop();
        self.invalidate_all();
    }
}
struct WatcherExit(Arc<Registry>);
impl Drop for WatcherExit {
    fn drop(&mut self) {
        self.0.availability.stop();
        self.0.invalidate_all();
    }
}
/// Separate service-runtime custody prevents Store refresh callbacks from
/// dropping/joining their own worker when a temporary upgraded Store Arc ends.
pub struct OwnedWatcher {
    registry: Arc<Registry>,
    thread: Option<JoinHandle<()>>,
}
impl OwnedWatcher {
    pub fn shutdown(&mut self) -> Result<()> {
        self.registry.availability.stop();
        self.registry.invalidate_all();
        self.registry.changed.notify_all();
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| Error::storage())?;
        }
        Ok(())
    }
}
impl Drop for OwnedWatcher {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
pub(crate) struct WriterNotice {
    registry: Weak<Registry>,
    id: u64,
}
impl Drop for WriterNotice {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            if let Ok(mut state) = registry.control.lock() {
                state.writers.retain(|p| p.id != self.id);
                state.sequence = state.sequence.saturating_add(1);
            } else {
                registry.availability.disable();
            }
            registry.changed.notify_all();
        }
    }
}

#[cfg(test)]
mod tests;
