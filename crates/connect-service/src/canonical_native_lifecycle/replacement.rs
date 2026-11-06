//! Receiver-only signed metadata through the original captured registration.
use super::*;
use crate::canonical_replacement::TargetPolicy;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Mutex, OnceLock,
};
const PREPARATION: u8 = 0;
const MOVING: u8 = 1;
const ORDINARY: u8 = 2;
struct State {
    entry: OnceLock<Arc<Connection>>,
    target: OnceLock<Weak<TargetPolicy>>,
    phase: Arc<AtomicU8>,
}
pub(crate) struct PreparedReplacementLifecycle {
    id: ConnectionId,
    state: Arc<State>,
    physical: Weak<Physical>,
    registration: Arc<rvoip_uctp::v2::native_lifecycle::NativeLifecycleRegistration>,
    registered: Mutex<bool>,
}
struct ReplacementPolicy {
    runtime: Weak<Runtime>,
    state: Arc<State>,
    physical: CancellationToken,
}
struct PrivateDelivery {
    _owner: MediaOwner,
    _read: Arc<connect_store::NativeReplacementMediaRead>,
}
fn phase_media_error(error: rvoip_core::staged_streams::StreamControlError) -> DispatchError {
    match error {
        rvoip_core::staged_streams::StreamControlError::MetadataBusy => {
            DispatchError::StorageUnavailable
        }
        other => media_error(other),
    }
}
fn domain_error(error: connect_protocol::Error) -> DispatchError {
    match error.code.as_str() {
        "authority-busy" | "storage-unavailable" => DispatchError::StorageUnavailable,
        _ => DispatchError::Authority,
    }
}
#[async_trait]
impl NativeLifecyclePolicy for ReplacementPolicy {
    fn receiver_epochs_only(&self) -> bool {
        self.state.phase.load(Ordering::Acquire) != ORDINARY
    }
    async fn authorize(
        &self,
        scope: &NativeLifecycleScope,
        event: &NativeLifecycleEvent,
    ) -> Result<AuthorityLease, DispatchError> {
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        if runtime.stop.is_cancelled() || self.physical.is_cancelled() {
            return Err(DispatchError::Closed);
        }
        let phase = self.state.phase.load(Ordering::Acquire);
        if phase == ORDINARY {
            let entry = self
                .state
                .entry
                .get()
                .cloned()
                .ok_or(DispatchError::StorageUnavailable)?;
            return Policy {
                runtime: self.runtime.clone(),
                entry,
                physical: self.physical.clone(),
            }
            .authorize(scope, event)
            .await;
        }
        let NativeLifecycleEvent::StreamEpoch { observation } = event else {
            return Err(DispatchError::StorageUnavailable);
        };
        if observation.epoch().publisher {
            return Err(DispatchError::StorageUnavailable);
        }
        let target = self
            .state
            .target
            .get()
            .and_then(Weak::upgrade)
            .ok_or(DispatchError::StorageUnavailable)?;
        if phase == MOVING {
            let continuation = target
                .current_continuation()
                .map_err(phase_media_error)?
                .ok_or(DispatchError::StorageUnavailable)?;
            let (entry, read, publication) = continuation
                .receiver_epoch_read(&scope.principal)
                .map_err(phase_media_error)?;
            return Policy {
                runtime: self.runtime.clone(),
                entry,
                physical: self.physical.clone(),
            }
            .authorize_read(
                scope,
                event,
                read,
                Some(publication),
                Some((self.state.phase.clone(), MOVING)),
            );
        }
        let (owner, read, preroll) = target
            .receiver_epoch_preparation(scope, observation)
            .map_err(domain_error)?;
        if self.state.phase.load(Ordering::Acquire) != PREPARATION {
            return Err(DispatchError::StorageUnavailable);
        }
        let remaining = (chrono::DateTime::from_timestamp(read.source().expires_at(), 0)
            .ok_or(DispatchError::Authority)?
            - chrono::Utc::now())
        .to_std()
        .map_err(|_| DispatchError::Authority)?;
        let current = read.clone();
        let phase = self.state.phase.clone();
        let physical = self.physical.clone();
        let stop = runtime.stop.clone();
        Ok(AuthorityLease::checking(
            PrivateDelivery {
                _owner: owner,
                _read: read,
            },
            tokio::time::Instant::now() + remaining.min(Duration::from_secs(1)),
            move || {
                phase.load(Ordering::Acquire) == PREPARATION
                    && current.current()
                    && preroll.upgrade().is_some_and(|p| p.is_current())
                    && !physical.is_cancelled()
                    && !stop.is_cancelled()
            },
        ))
    }
}
impl Runtime {
    pub(crate) fn prepare_replacement_native_lifecycle(
        self: &Arc<Self>,
        id: &ConnectionId,
        a: &AuthorizedEnvelope,
        physical: &Arc<Physical>,
    ) -> connect_protocol::Result<PreparedReplacementLifecycle> {
        let state = Arc::new(State {
            entry: OnceLock::new(),
            target: OnceLock::new(),
            phase: Arc::new(AtomicU8::new(PREPARATION)),
        });
        let registration = self
            .connection_native_adapter(id)?
            .native_lifecycle_registration(
                id,
                a,
                Arc::new(ReplacementPolicy {
                    runtime: Arc::downgrade(self),
                    state: state.clone(),
                    physical: physical.lifetime.clone(),
                }),
            )
            .map_err(crate::canonical_native::registration_error)?;
        Ok(PreparedReplacementLifecycle {
            id: id.clone(),
            state,
            physical: Arc::downgrade(physical),
            registration,
            registered: Mutex::new(false),
        })
    }
}
impl PreparedReplacementLifecycle {
    fn register(&self) -> connect_protocol::Result<()> {
        let physical = self
            .physical
            .upgrade()
            .filter(|p| !p.lifetime.is_cancelled())
            .ok_or_else(connect_protocol::Error::forbidden)?;
        let mut registered = self
            .registered
            .lock()
            .map_err(|_| connect_protocol::Error::storage())?;
        if !*registered {
            physical
                .lifecycle
                .register(self.id.clone(), self.registration.clone())
                .map_err(|_| connect_protocol::Error::forbidden())?;
            *registered = true;
        }
        Ok(())
    }
    pub(crate) fn start_private(&self, target: &Arc<TargetPolicy>) -> connect_protocol::Result<()> {
        if let Some(old) = self.state.target.get() {
            if !Weak::ptr_eq(old, &Arc::downgrade(target)) {
                return Err(connect_protocol::Error::forbidden());
            }
        } else {
            self.state
                .target
                .set(Arc::downgrade(target))
                .map_err(|_| connect_protocol::Error::forbidden())?;
        }
        self.register()
    }
    pub(crate) fn begin_movement(&self) {
        self.state
            .phase
            .compare_exchange(PREPARATION, MOVING, Ordering::AcqRel, Ordering::Acquire)
            .ok();
    }
    pub(crate) fn activate(&self, runtime: &Runtime) -> connect_protocol::Result<()> {
        let entry = runtime
            .media
            .entry(&self.id)
            .map_err(|_| connect_protocol::Error::forbidden())?;
        if let Some(old) = self.state.entry.get() {
            if !Arc::ptr_eq(old, &entry) {
                return Err(connect_protocol::Error::forbidden());
            }
        } else {
            self.state
                .entry
                .set(entry)
                .map_err(|_| connect_protocol::Error::forbidden())?;
        }
        self.state.phase.store(ORDINARY, Ordering::Release);
        self.register()
    }
}
