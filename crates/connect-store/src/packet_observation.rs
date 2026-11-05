//! Four current SQL observers, scoped to original transports and complete reads.
//! Resource leases contain no credential, metadata, authority interval or grant.
use crate::*;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, MutexGuard, TryLockError};
pub(crate) const OBSERVERS: usize = 4;
static NEXT_POOL: AtomicU64 = AtomicU64::new(1);
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
pub(crate) struct Guard<'a> {
    slot: MutexGuard<'a, Slot>,
    id: ObserverId,
    _observation: &'a Observation,
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
        for (index, slot) in self.0.slots.iter().enumerate() {
            if original.is_some_and(|id| id.slot != index) {
                continue;
            }
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
            });
        }
        Err(busy(if original.is_some() {
            "Original packet observer is busy during snapshot reseal"
        } else {
            "All current packet observers are busy"
        }))
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
