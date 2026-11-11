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
}
impl PreparationCustodyPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Attach => "attach",
            Self::Signal => "signal",
            Self::AllocationCapture => "allocation-capture",
            Self::AllocationReply => "allocation-reply",
            Self::Activate => "activate",
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
