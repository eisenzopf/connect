//! Protected host-to-adapter custody. This capability has no wire representation.
use crate::Failure;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

/// Implemented by the trusted host. Checks must not block or enter storage from
/// an I/O poll. Current write fences and owned runtime/role custody live here.
pub trait Authority: Send + Sync {
    fn current(&self) -> bool;
    fn deadline(&self) -> Instant;
}
#[derive(Clone)]
pub struct DispatchPermit(Arc<State>);
struct State {
    authority: Arc<dyn Authority>,
    admitted: AtomicBool,
    effect_allowed: bool,
}
impl DispatchPermit {
    pub fn from_host(authority: Arc<dyn Authority>) -> Self {
        Self::host(authority, true)
    }
    /// Current custody for an observation of an existing effect. It cannot
    /// authorize creation, even when cloned or forwarded through a send API.
    pub fn from_host_readonly(authority: Arc<dyn Authority>) -> Self {
        Self::host(authority, false)
    }
    fn host(authority: Arc<dyn Authority>, effect_allowed: bool) -> Self {
        Self(Arc::new(State {
            authority,
            admitted: AtomicBool::new(false),
            effect_allowed,
        }))
    }
    pub fn check(&self) -> Result<(), Failure> {
        if Instant::now() < self.0.authority.deadline() && self.0.authority.current() {
            Ok(())
        } else {
            Err(Failure::Uncertain)
        }
    }
    /// One creation attempt across all clones; read-only provider preflights do
    /// not consume this admission. Adapters call immediately before their effect.
    pub fn admit_effect(&self) -> Result<(), Failure> {
        self.check()?;
        if !self.0.effect_allowed {
            return Err(Failure::Rejected);
        }
        self.0
            .admitted
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Failure::Uncertain)?;
        Ok(())
    }
    pub fn deadline(&self) -> Instant {
        self.0.authority.deadline()
    }
}
impl std::fmt::Debug for DispatchPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DispatchPermit(redacted)")
    }
}
