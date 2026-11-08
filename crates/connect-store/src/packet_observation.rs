//! Four current SQL observers, scoped to original transports and complete reads.
//! Resource leases contain no credential, metadata, authority interval or grant.
use crate::*;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, MutexGuard, TryLockError};
pub(crate) const OBSERVERS: usize = 4;
static NEXT_POOL: AtomicU64 = AtomicU64::new(1);
static NEXT_SLOT_CLAIM: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ObserverId {
    pool: u64,
    generation: u64,
    slot: usize,
}
#[cfg(test)]
impl ObserverId {
    pub(crate) fn slot_index(self) -> usize {
        self.slot
    }
}
pub(crate) struct Slot(Option<Connection>);
impl Deref for Slot {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0.as_ref().expect("leased ready observer")
    }
}
impl DerefMut for Slot {
    fn deref_mut(&mut self) -> &mut Connection {
        self.0.as_mut().expect("leased ready observer")
    }
}
enum Phase {
    Idle,
    Opening(u64),
    Ready {
        generation: u64,
        scopes: usize,
        observations: usize,
    },
    Retiring {
        generation: u64,
        closing: bool,
    },
    Failed,
}
struct State {
    next_generation: u64,
    phase: Phase,
}
// Leases retain only resource-free counters. Even a last lease outliving the
// Store cannot make its Drop close a SQLite handle under a caller's guard.
struct Ownership {
    id: u64,
    state: Mutex<State>,
    failed: AtomicBool,
    // Resource-free claims only. A transaction may release SQL while retaining
    // its original slot; no claim owns a Connection, permission or IO grant.
    claims: [AtomicU64; OBSERVERS],
    claim_clock: std::time::Instant,
    claim_diagnostics: ClaimDiagnostics,
}
struct Pool {
    ownership: Arc<Ownership>,
    path: PathBuf,
    slots: [Mutex<Slot>; OBSERVERS],
    failed_connections: Mutex<Vec<Connection>>,
    #[cfg(test)]
    before_open: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    #[cfg(test)]
    before_close: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}
#[derive(Clone)]
pub(crate) struct Observers(Arc<Pool>);
/// Resource custody only. Clones share one original scope, never permission.
#[derive(Clone)]
pub struct PacketObserverScope {
    _scope: Arc<Scope>,
}
struct Scope {
    ownership: Arc<Ownership>,
    generation: u64,
}
impl Drop for Scope {
    fn drop(&mut self) {
        self.ownership.release(self.generation, true);
    }
}
pub(crate) struct Observation {
    ownership: Arc<Ownership>,
    generation: u64,
}
impl Observation {
    pub(crate) fn current(&self, id: ObserverId) -> bool {
        id.pool == self.ownership.id && id.generation == self.generation
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        self.ownership.release(self.generation, false);
    }
}
// Separate claim availability from actual SQL mutex custody. No IO or logs
// run on these paths; the existing retention observer reads fixed atomics.
const CLAIM_PHASE_NAMES: [&str; 4] = ["unreserved", "capture", "file-admit", "reseal"];
const UNRESERVED: usize = 0;
const CAPTURE: usize = 1;
const FILE_ADMIT: usize = 2;
const RESEAL: usize = 3;
#[derive(Default)]
struct PhaseTimes {
    entered: AtomicU64,
    exited: AtomicU64,
    total_ns: AtomicU64,
    maximum_ns: AtomicU64,
}
fn diagnostic_add(value: &AtomicU64, amount: u64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        Some(old.saturating_add(amount))
    });
}
impl PhaseTimes {
    fn enter(&self) {
        diagnostic_add(&self.entered, 1);
    }
    fn exit(&self, elapsed: u64) {
        diagnostic_add(&self.total_ns, elapsed);
        self.maximum_ns.fetch_max(elapsed, Ordering::Relaxed);
        diagnostic_add(&self.exited, 1);
    }
    fn snapshot(&self, phase: &'static str) -> serde_json::Value {
        let entered = self.entered.load(Ordering::Relaxed);
        let exited = self.exited.load(Ordering::Relaxed);
        let total = self.total_ns.load(Ordering::Relaxed);
        let maximum = self.maximum_ns.load(Ordering::Relaxed);
        let saturated = [entered, exited, total, maximum].contains(&u64::MAX);
        serde_json::json!({"phase":phase,"entered":entered,"exited":exited,
            "in_flight_sample":entered.saturating_sub(exited),
            "completed_intervals_total_ns":total,"completed_intervals_max_ns":maximum,
            "saturated":saturated})
    }
}
#[derive(Default)]
struct ClaimDiagnostics {
    timed_claims: AtomicU64,
    untimed_claims: AtomicU64,
    reserved_claim_attempts: AtomicU64,
    reserved_results: [AtomicU64; 2],
    claims: [PhaseTimes; 4],
    sql: [PhaseTimes; 4],
}
struct ClaimTiming {
    phase: std::sync::atomic::AtomicUsize,
    started_ns: AtomicU64,
    result_observed: AtomicBool,
}
// This owns only the existing resource-free ownership metadata. Guard declares
// it last, so original SQL unlock and claim release order stays unchanged.
struct SqlTiming {
    ownership: Arc<Ownership>,
    phase: usize,
    started_ns: u64,
}
impl Ownership {
    fn claim_now_ns(&self) -> u64 {
        self.claim_clock
            .elapsed()
            .as_nanos()
            .try_into()
            .unwrap_or(u64::MAX)
    }
}
fn claim_timing_enabled() -> bool {
    tracing::enabled!(target: "connect_service::runtime_retention", tracing::Level::DEBUG)
}
#[cfg(test)]
pub(crate) fn with_claim_timing_for_test(f: impl FnOnce()) {
    struct Trace;
    impl tracing::Subscriber for Trace {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.target() == "connect_service::runtime_retention"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            unreachable!("claim timing creates no spans")
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
        fn event(&self, _: &tracing::Event<'_>) {
            panic!("claim timing must not emit packet-path events")
        }
    }
    tracing::subscriber::with_default(Trace, f);
}
impl Drop for SqlTiming {
    fn drop(&mut self) {
        self.ownership.claim_diagnostics.sql[self.phase].exit(
            self.ownership
                .claim_now_ns()
                .saturating_sub(self.started_ns),
        );
    }
}
struct SlotClaim {
    ownership: Arc<Ownership>,
    slot: usize,
    token: u64,
    timing: Option<ClaimTiming>,
}
impl SlotClaim {
    fn phase(&self, next: usize) {
        if let Some(timing) = &self.timing {
            let now = self.ownership.claim_now_ns();
            let previous = timing.phase.swap(next, Ordering::Relaxed);
            let started = timing.started_ns.swap(now, Ordering::Relaxed);
            self.ownership.claim_diagnostics.claims[previous].exit(now.saturating_sub(started));
            self.ownership.claim_diagnostics.claims[next].enter();
        }
    }
    fn sql_timing(&self) -> Option<SqlTiming> {
        let timing = self.timing.as_ref()?;
        let phase = timing.phase.load(Ordering::Relaxed);
        let started_ns = self.ownership.claim_now_ns();
        self.ownership.claim_diagnostics.sql[phase].enter();
        Some(SqlTiming {
            ownership: self.ownership.clone(),
            phase,
            started_ns,
        })
    }
}
impl Drop for SlotClaim {
    fn drop(&mut self) {
        if self.ownership.claims[self.slot]
            .compare_exchange(self.token, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.ownership.failed.store(true, Ordering::Release);
        }
        // Release the exact original claim FIRST, before observational teardown.
        if let Some(timing) = &self.timing {
            self.ownership.claim_diagnostics.claims[timing.phase.load(Ordering::Relaxed)].exit(
                self.ownership
                    .claim_now_ns()
                    .saturating_sub(timing.started_ns.load(Ordering::Relaxed)),
            );
        }
    }
}
/// Only the original compound capture/admit/reseal owns this resource-free
/// claim. The borrowed Observation pins its generation, not a media lifetime.
/// Never return this reservation inside current reads or metadata tokens.
pub(crate) struct Reservation<'a> {
    claim: SlotClaim,
    id: ObserverId,
    _observation: &'a Observation,
}
impl Reservation<'_> {
    pub(crate) fn begin_file_admission(&self) {
        self.claim.phase(FILE_ADMIT);
    }
    pub(crate) fn begin_reseal(&self) {
        self.claim.phase(RESEAL);
    }
    pub(crate) fn observe_result(&self, success: bool) {
        if self
            .claim
            .timing
            .as_ref()
            .is_some_and(|timing| !timing.result_observed.swap(true, Ordering::Relaxed))
        {
            diagnostic_add(
                &self.claim.ownership.claim_diagnostics.reserved_results[usize::from(!success)],
                1,
            );
        }
    }
}
pub(crate) struct Guard<'a> {
    slot: MutexGuard<'a, Slot>,
    id: ObserverId,
    _observation: &'a Observation,
    _reservation: Option<&'a Reservation<'a>>,
    // SQL slot is declared first: unlock it before a plain read releases its
    // short claim, so an unclaimed slot cannot still own an ordinary SQL guard.
    _claim: Option<SlotClaim>,
    _sql_timing: Option<SqlTiming>,
}
impl Guard<'_> {
    pub(crate) fn id(&self) -> ObserverId {
        self.id
    }
}
impl Deref for Guard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.slot
    }
}
impl DerefMut for Guard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.slot
    }
}
fn busy(message: &'static str) -> Error {
    Error::new("authority-busy", message)
}
fn open(path: &Path) -> Result<Vec<Connection>> {
    open_each(path, |_| Ok(()))
}
fn open_each(path: &Path, mut before: impl FnMut(usize) -> Result<()>) -> Result<Vec<Connection>> {
    (0..OBSERVERS)
        .map(|index| {
            before(index)?;
            let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| Error::storage())?;
            c.set_prepared_statement_cache_capacity(OPERATION_SQL_CACHE_CAPACITY);
            c.busy_timeout(Duration::ZERO)
                .map_err(|_| Error::storage())?;
            c.execute_batch("PRAGMA query_only=ON")
                .map_err(|_| Error::storage())?;
            Ok(c)
        })
        .collect()
}
impl Observers {
    pub(crate) fn new(path: &Path) -> Result<Self> {
        let id = NEXT_POOL
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| Error::storage())?;
        Ok(Self(Arc::new(Pool {
            ownership: Arc::new(Ownership {
                id,
                state: Mutex::new(State {
                    next_generation: 0,
                    phase: Phase::Idle,
                }),
                failed: AtomicBool::new(false),
                claims: std::array::from_fn(|_| AtomicU64::new(0)),
                claim_clock: std::time::Instant::now(),
                claim_diagnostics: ClaimDiagnostics::default(),
            }),
            path: path.into(),
            slots: std::array::from_fn(|_| Mutex::new(Slot(None))),
            failed_connections: Mutex::new(Vec::new()),
            #[cfg(test)]
            before_open: Mutex::new(None),
            #[cfg(test)]
            before_close: Mutex::new(None),
        })))
    }
    /// Trusted physical admission outside every Core/Store/physical registry.
    /// Finishes an already queued old batch before preparing a cold generation.
    pub(crate) fn scope(&self) -> Result<PacketObserverScope> {
        self.reap()?;
        let generation = {
            let mut s = self.0.ownership.try_state()?;
            match &mut s.phase {
                Phase::Ready {
                    generation, scopes, ..
                } if *scopes > 0 => {
                    *scopes = scopes.checked_add(1).ok_or_else(Error::storage)?;
                    return Ok(PacketObserverScope {
                        _scope: Arc::new(Scope {
                            ownership: self.0.ownership.clone(),
                            generation: *generation,
                        }),
                    });
                }
                Phase::Idle => {
                    let g = s
                        .next_generation
                        .checked_add(1)
                        .ok_or_else(Error::storage)?;
                    s.next_generation = g;
                    s.phase = Phase::Opening(g);
                    g
                }
                Phase::Failed => return Err(Error::storage()),
                _ => return Err(busy("Packet observers are preparing or retiring")),
            }
        };
        // All file opens/configuration/failed partial closes happen outside locks.
        #[cfg(test)]
        {
            let callback = self.0.before_open.lock().unwrap().take();
            if let Some(callback) = callback {
                callback();
            }
        }
        let opened = open(&self.0.path);
        let mut s = self
            .0
            .ownership
            .state
            .lock()
            .map_err(|_| Error::storage())?;
        if !matches!(s.phase,Phase::Opening(g) if g==generation) {
            self.0.ownership.failed.store(true, Ordering::Release);
            drop(s);
            drop(opened);
            return Err(Error::storage());
        }
        match opened {
            Err(e) => {
                s.phase = Phase::Idle;
                Err(e)
            }
            Ok(connections) => {
                let mut guards = Vec::with_capacity(OBSERVERS);
                for slot in &self.0.slots {
                    match slot.try_lock() {
                        Ok(g) if g.0.is_none() => guards.push(g),
                        _ => {
                            self.0.ownership.failed.store(true, Ordering::Release);
                            drop(guards);
                            drop(s);
                            drop(connections);
                            return Err(Error::storage());
                        }
                    }
                }
                for (slot, c) in guards.iter_mut().zip(connections) {
                    slot.0 = Some(c);
                }
                drop(guards);
                s.phase = Phase::Ready {
                    generation,
                    scopes: 1,
                    observations: 0,
                };
                Ok(PacketObserverScope {
                    _scope: Arc::new(Scope {
                        ownership: self.0.ownership.clone(),
                        generation,
                    }),
                })
            }
        }
    }
    pub(crate) fn observe(&self) -> Result<Observation> {
        let mut s = self.0.ownership.try_state()?;
        match &mut s.phase {
            Phase::Ready {
                generation,
                scopes,
                observations,
            } if *scopes > 0 => {
                *observations = observations.checked_add(1).ok_or_else(Error::storage)?;
                Ok(Observation {
                    ownership: self.0.ownership.clone(),
                    generation: *generation,
                })
            }
            Phase::Failed => Err(Error::storage()),
            _ => Err(busy("No current physical packet observer scope")),
        }
    }
    pub(crate) fn connection<'a>(
        &'a self,
        observation: &'a Observation,
        original: Option<ObserverId>,
    ) -> Result<Guard<'a>> {
        if !Arc::ptr_eq(&self.0.ownership, &observation.ownership)
            || self.0.ownership.failed.load(Ordering::Acquire)
        {
            return Err(Error::storage());
        }
        if original.is_some_and(|id| {
            id.pool != self.0.ownership.id
                || id.generation != observation.generation
                || id.slot >= OBSERVERS
        }) {
            return Err(busy("Original packet observer generation changed"));
        }
        let token = Self::next_claim()?;
        for (index, slot) in self.0.slots.iter().enumerate() {
            if original.is_some_and(|id| id.slot != index) {
                continue;
            }
            let Some(claim) = self.claim(index, token, UNRESERVED) else {
                continue;
            };
            let g = match slot.try_lock() {
                Ok(g) => g,
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Poisoned(_)) => return Err(Error::storage()),
            };
            if g.0.is_none() {
                return Err(Error::storage());
            }
            return Ok(Guard {
                slot: g,
                id: ObserverId {
                    pool: self.0.ownership.id,
                    generation: observation.generation,
                    slot: index,
                },
                _observation: observation,
                _reservation: None,
                _sql_timing: claim.sql_timing(),
                _claim: Some(claim),
            });
        }
        Err(busy(if original.is_some() {
            "Original packet observer is busy during snapshot reseal"
        } else {
            "All current packet observers are busy"
        }))
    }
    fn next_claim() -> Result<u64> {
        NEXT_SLOT_CLAIM
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| Error::storage())
    }
    fn claim(&self, slot: usize, token: u64, phase: usize) -> Option<SlotClaim> {
        self.0.ownership.claims[slot]
            .compare_exchange(0, token, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| {
                let d = &self.0.ownership.claim_diagnostics;
                let timing = if claim_timing_enabled() {
                    diagnostic_add(&d.timed_claims, 1);
                    if phase == CAPTURE {
                        diagnostic_add(&d.reserved_claim_attempts, 1);
                    }
                    d.claims[phase].enter();
                    Some(ClaimTiming {
                        phase: std::sync::atomic::AtomicUsize::new(phase),
                        started_ns: AtomicU64::new(self.0.ownership.claim_now_ns()),
                        result_observed: AtomicBool::new(false),
                    })
                } else {
                    diagnostic_add(&d.untimed_claims, 1);
                    None
                };
                SlotClaim {
                    ownership: self.0.ownership.clone(),
                    slot,
                    token,
                    timing,
                }
            })
    }
    /// Reserve one successful SQL slot for this exact compound transaction.
    /// SQL is unlocked before return; unrelated readers use the other original
    /// slots. All slots claimed/busy still refuses immediately, without waiting.
    pub(crate) fn reserve<'a>(&'a self, observation: &'a Observation) -> Result<Reservation<'a>> {
        if !Arc::ptr_eq(&self.0.ownership, &observation.ownership)
            || self.0.ownership.failed.load(Ordering::Acquire)
        {
            return Err(Error::storage());
        }
        let token = Self::next_claim()?;
        for (index, slot) in self.0.slots.iter().enumerate() {
            let Some(claim) = self.claim(index, token, CAPTURE) else {
                continue;
            };
            let sql = match slot.try_lock() {
                Ok(sql) => sql,
                Err(TryLockError::WouldBlock) => continue,
                Err(TryLockError::Poisoned(_)) => return Err(Error::storage()),
            };
            if sql.0.is_none() {
                return Err(Error::storage());
            }
            let sql_timing = claim.sql_timing();
            drop(sql);
            drop(sql_timing);
            return Ok(Reservation {
                claim,
                id: ObserverId {
                    pool: self.0.ownership.id,
                    generation: observation.generation,
                    slot: index,
                },
                _observation: observation,
            });
        }
        Err(busy("All current packet observers are busy"))
    }
    pub(crate) fn connection_reserved<'a>(
        &'a self,
        observation: &'a Observation,
        reservation: &'a Reservation<'a>,
    ) -> Result<Guard<'a>> {
        if !Arc::ptr_eq(&self.0.ownership, &observation.ownership)
            || !std::ptr::eq(observation, reservation._observation)
            || !Arc::ptr_eq(&self.0.ownership, &reservation.claim.ownership)
            || !observation.current(reservation.id)
            || reservation.id.slot >= OBSERVERS
            || reservation.claim.slot != reservation.id.slot
            || self.0.ownership.claims[reservation.id.slot].load(Ordering::Acquire)
                != reservation.claim.token
            || self.0.ownership.failed.load(Ordering::Acquire)
        {
            return Err(Error::storage());
        }
        let sql = self.0.slots[reservation.id.slot]
            .try_lock()
            .map_err(|error| match error {
                TryLockError::WouldBlock => {
                    busy("Original packet observer is busy during snapshot reseal")
                }
                TryLockError::Poisoned(_) => Error::storage(),
            })?;
        if sql.0.is_none() {
            return Err(Error::storage());
        }
        Ok(Guard {
            slot: sql,
            id: reservation.id,
            _observation: observation,
            _reservation: Some(reservation),
            _claim: None,
            _sql_timing: reservation.claim.sql_timing(),
        })
    }
    #[cfg(feature = "memory-diagnostics")]
    pub(crate) fn memory_counts(&self) -> serde_json::Value {
        let mut open = 0usize;
        let mut unknown = 0usize;
        for slot in &self.0.slots {
            match slot.try_lock() {
                Ok(slot) => open += usize::from(slot.0.is_some()),
                Err(_) => unknown += 1,
            }
        }
        let failed = self.0.failed_connections.try_lock().ok().map(|c| c.len());
        serde_json::json!({"open_slot_sql_handles":(unknown==0).then_some(open),
            "unknown_slots":unknown,"slot_capacity":OBSERVERS,"failed_sql_handles":failed,
            "statement_cache_capacity_per_handle":OPERATION_SQL_CACHE_CAPACITY,
            "actual_statement_cache_entries":null,"sqlite_page_cache_bytes":null})
    }
    pub(crate) fn claim_diagnostics(&self) -> serde_json::Value {
        let d = &self.0.ownership.claim_diagnostics;
        let reserved = d.reserved_claim_attempts.load(Ordering::Relaxed);
        let results = d
            .reserved_results
            .each_ref()
            .map(|v| v.load(Ordering::Relaxed));
        serde_json::json!({
            "format":"connect/packet-observer-claim-phases/1", "concurrent_snapshot":true,
            "slot_capacity":OBSERVERS, "tracking_enabled_now":claim_timing_enabled(),
            "occupied_slots_sample":self.0.ownership.claims.iter().filter(|v|v.load(Ordering::Acquire)!=0).count(),
            "timed_claims":d.timed_claims.load(Ordering::Relaxed),
            "untimed_claims":d.untimed_claims.load(Ordering::Relaxed),
            "reserved_claim_attempts":reserved,"projection_returned_ok":results[0],"projection_returned_error":results[1],
            "reserved_result_unreported_sample":reserved.saturating_sub(results[0].saturating_add(results[1])),
            "claim_intervals":CLAIM_PHASE_NAMES.iter().enumerate().map(|(i,name)|d.claims[i].snapshot(name)).collect::<Vec<_>>(),
            "sql_locked_intervals":CLAIM_PHASE_NAMES.iter().enumerate().map(|(i,name)|d.sql[i].snapshot(name)).collect::<Vec<_>>(),
            "limitations":["Independent atomic samples can race; timings include only completed intervals and are not active-age bounds",
                "Claim phases describe slot exclusion, not SQL locking; SQL custody is measured separately after actual acquisition",
                "Unreported includes active, early reservation refusal and unwound captures; no failed/aborted result is invented",
                "Projection Ok is metadata admission, never media delivery or decoder readiness; untimed claims are not observed durations"]
        })
    }
    /// Only an existing owned blocking custodian or trusted physical admission
    /// may call this. Scope/query Drop only marks retirement, never closes SQLite.
    pub(crate) fn reap(&self) -> Result<bool> {
        let (generation, connections) = {
            let mut s = self.0.ownership.try_state()?;
            match &mut s.phase {
                Phase::Retiring {
                    generation,
                    closing,
                } if !*closing => {
                    let mut guards = Vec::with_capacity(OBSERVERS);
                    for slot in &self.0.slots {
                        match slot.try_lock() {
                            Ok(g) => guards.push(g),
                            Err(TryLockError::WouldBlock) => return Ok(false),
                            Err(TryLockError::Poisoned(_)) => {
                                self.0.ownership.failed.store(true, Ordering::Release);
                                return Err(Error::storage());
                            }
                        }
                    }
                    let connections = guards
                        .iter_mut()
                        .filter_map(|slot| slot.0.take())
                        .collect::<Vec<_>>();
                    drop(guards);
                    *closing = true;
                    (*generation, connections)
                }
                Phase::Failed => return Err(Error::storage()),
                Phase::Idle => return Ok(true),
                _ => return Ok(false),
            }
        };
        // Exact original batch, outside ALL pool/slot/file/query guards.
        #[cfg(test)]
        {
            let callback = self.0.before_close.lock().unwrap().take();
            if let Some(callback) = callback {
                callback();
            }
        }
        let mut failed = Vec::new();
        for c in connections {
            if let Err((c, _)) = c.close() {
                failed.push(c);
            }
        }
        let mut s = self
            .0
            .ownership
            .state
            .lock()
            .map_err(|_| Error::storage())?;
        if !matches!(s.phase,Phase::Retiring {generation:g,closing:true} if g==generation) {
            self.0.ownership.failed.store(true, Ordering::Release);
            drop(s);
            drop(failed);
            return Err(Error::storage());
        }
        if failed.is_empty() {
            s.phase = Phase::Idle;
            Ok(true)
        } else {
            s.phase = Phase::Failed;
            self.0.ownership.failed.store(true, Ordering::Release);
            drop(s);
            // Quarantine failures on the Store-owned pool, never in a lease.
            self.0
                .failed_connections
                .lock()
                .map_err(|_| Error::storage())?
                .extend(failed);
            Err(Error::storage())
        }
    }
}
impl Ownership {
    fn try_state(&self) -> Result<MutexGuard<'_, State>> {
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::storage());
        }
        self.state.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => busy("Packet observer ownership is busy"),
            TryLockError::Poisoned(_) => Error::storage(),
        })
    }
    fn release(&self, generation: u64, physical: bool) {
        let Ok(mut s) = self.state.lock() else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        let Phase::Ready {
            generation: current,
            scopes,
            observations,
        } = &mut s.phase
        else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        let count = if physical { scopes } else { observations };
        if *current != generation || *count == 0 {
            self.failed.store(true, Ordering::Release);
            return;
        }
        *count -= 1;
        let generation = match s.phase {
            Phase::Ready {
                generation,
                scopes: 0,
                observations: 0,
            } => generation,
            _ => return,
        };
        s.phase = Phase::Retiring {
            generation,
            closing: false,
        };
    }
}
// Test fixtures retain an explicit physical scope while controlling slots.
#[cfg(test)]
impl Deref for Observers {
    type Target = [Mutex<Slot>];
    fn deref(&self) -> &Self::Target {
        &self.0.slots
    }
}
#[cfg(test)]
mod tests;
