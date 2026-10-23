//! Latest native descriptor hints retain metadata while awaiting a writer.
//! Current source/recipient readers exist only during actual bounded IO.
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
};
use connect_protocol::{Error, Result};
use rvoip_core::ids::ConnectionId;
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError, HandlerDelivery},
    native_capability_delivery::{
        NativeCapabilityDeclaration, NativeCapabilityRegistration, NativeCapabilityWritePolicy,
    },
    native_lifecycle::NativeLifecycleScope,
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(crate) struct Listener {
    registration: Arc<NativeCapabilityRegistration>,
    entry: Arc<Connection>,
    physical: CancellationToken,
    queue: Weak<Queue>,
}
struct Source {
    declaration: Arc<NativeCapabilityDeclaration>,
    entry: Arc<Connection>,
    record: String,
}
#[derive(Clone)]
struct Queued {
    source: Arc<Source>,
    target: Arc<Listener>,
    at: Instant,
}
type Key = (String, String);
impl Queued {
    fn key(&self) -> Key {
        (
            self.source.entry.origin.runtime_connection_id().into(),
            self.target.entry.origin.runtime_connection_id().into(),
        )
    }
    fn current(&self) -> bool {
        self.source.declaration.current()
            && self.target.registration.current()
            && !self.target.physical.is_cancelled()
    }
}
struct Flight {
    queued: Queued,
    result: Arc<Mutex<Option<std::result::Result<(), DispatchError>>>>,
    watch: tokio::task::JoinHandle<()>,
}
impl Drop for Flight {
    fn drop(&mut self) {
        self.watch.abort();
    }
}
#[derive(Default)]
struct State {
    listeners: BTreeMap<String, Arc<Listener>>,
    items: BTreeMap<Key, Queued>,
    order: VecDeque<Key>,
    flight: Option<Flight>,
}
pub(crate) struct Queue {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}
impl Default for Queue {
    fn default() -> Self {
        Self {
            state: Mutex::new(State::default()),
            changed: tokio::sync::Notify::new(),
        }
    }
}
impl Queue {
    fn enqueue(
        &self,
        store: &connect_store::Store,
        source: Arc<Source>,
        target: Arc<Listener>,
        capacity: usize,
    ) -> Result<()> {
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        // SQL completion has already released its transaction. Queue->SQL is
        // the sole order; delayed old enqueue cannot overwrite a newer item.
        if !store.native_capability_is_latest(
            &source.entry.origin,
            &source.record,
            source.declaration.payload(),
        )? {
            return Ok(());
        }
        state.items.retain(|_, q| q.current());
        let keys = state.items.keys().cloned().collect::<BTreeSet<_>>();
        state.order.retain(|key| keys.contains(key));
        let queued = Queued {
            source,
            target,
            at: Instant::now(),
        };
        let key = queued.key();
        if !state.items.contains_key(&key) {
            if state.items.len() >= capacity {
                return Err(Error::new(
                    "capacity-exceeded",
                    "Native descriptor queue is full",
                ));
            }
            state.order.push_back(key.clone());
        }
        state.items.insert(key, queued);
        drop(state);
        self.changed.notify_waiters();
        Ok(())
    }
    fn next_ready(
        self: &Arc<Self>,
        runtime: &Arc<Runtime>,
    ) -> std::result::Result<Option<HandlerDelivery>, DispatchError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DispatchError::StorageUnavailable)?;
        let completed = state
            .flight
            .as_ref()
            .and_then(|f| *f.result.lock().unwrap_or_else(|p| p.into_inner()));
        if let Some(result) = completed {
            let flight = state.flight.take().expect("observed flight");
            // Healthy in-flight frames finish before any queued replacement.
            // A capacity/deadline refusal may retry only if no newer queued
            // descriptor replaced it; actual admission rechecks the Store.
            if matches!(
                result,
                Err(DispatchError::Closed
                    | DispatchError::Capacity
                    | DispatchError::AuthorityUnavailable)
            ) && flight.queued.current()
                && flight.queued.at.elapsed() < Duration::from_secs(5)
            {
                let key = flight.queued.key();
                if !state.items.contains_key(&key) {
                    state.order.push_back(key.clone());
                    state.items.insert(key, flight.queued.clone());
                }
            }
        }
        if state.flight.is_none() {
            while let Some(key) = state.order.pop_front() {
                let Some(queued) = state.items.remove(&key) else {
                    continue;
                };
                if !queued.current() {
                    continue;
                }
                if !queued.source.declaration.ready() || !queued.target.registration.ready() {
                    state.order.push_back(key.clone());
                    state.items.insert(key, queued);
                    break;
                }
                let policy = Arc::new(Policy {
                    runtime: Arc::downgrade(runtime),
                    source: queued.source.clone(),
                    target: queued.target.clone(),
                });
                let (delivery, receipt) = match queued
                    .target
                    .registration
                    .delivery(queued.source.declaration.clone(), policy)
                {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let result = Arc::new(Mutex::new(None));
                let observed = result.clone();
                let queue = Arc::downgrade(self);
                let watch = tokio::spawn(async move {
                    let done = tokio::time::timeout(Duration::from_secs(2), receipt)
                        .await
                        .ok()
                        .and_then(|r| r.ok())
                        .unwrap_or(Err(DispatchError::Closed));
                    *observed.lock().unwrap_or_else(|p| p.into_inner()) = Some(done);
                    if let Some(queue) = queue.upgrade() {
                        queue.changed.notify_waiters();
                    }
                });
                state.flight = Some(Flight {
                    queued,
                    result,
                    watch,
                });
                return Ok(Some(delivery));
            }
        }
        Ok(None)
    }
    pub(crate) async fn next(
        self: &Arc<Self>,
        runtime: &Arc<Runtime>,
    ) -> std::result::Result<HandlerDelivery, DispatchError> {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if let Some(delivery) = self.next_ready(runtime)? {
                return Ok(delivery);
            }
            tokio::select! {_=&mut changed=>{},_=runtime.stop.cancelled()=>return Err(DispatchError::Closed),_=tokio::time::sleep(Duration::from_millis(5))=>{}}
        }
    }
}

impl Runtime {
    pub(crate) fn enable_capability_delivery(
        self: &Arc<Self>,
        id: &ConnectionId,
        a: &AuthorizedEnvelope,
        physical: &Physical,
    ) -> Result<()> {
        let entry = self
            .media
            .entry(id)
            .map_err(|error| crate::canonical_native::registration_error(error.into()))?;
        let registration = self
            .native
            .native_capability_registration(&self.core, id, a)
            .map_err(|error| {
                tracing::debug!(
                    ?error,
                    "Native capability listener registration deferred or refused"
                );
                match error {
                    rvoip_uctp::v2::native_negotiation::NativeOfferError::NotReady => {
                        Error::new("authority-busy", "Native capability listener is busy")
                    }
                    _ => Error::forbidden(),
                }
            })?;
        self.install_capability_delivery(id, entry, registration, physical)
    }
    pub(crate) fn refresh_capability_delivery(
        self: &Arc<Self>,
        id: &ConnectionId,
        result: &rvoip_uctp::v2::native::RefreshedNativeMedia,
        physical: &Physical,
    ) -> std::result::Result<(), DispatchError> {
        let entry = self.media.entry(id).map_err(|_| DispatchError::Authority)?;
        self.install_capability_delivery(id, entry, result.capability(&self.core)?, physical)
            .map_err(|_| DispatchError::Authority)
    }
    fn install_capability_delivery(
        self: &Arc<Self>,
        id: &ConnectionId,
        entry: Arc<Connection>,
        registration: Arc<NativeCapabilityRegistration>,
        physical: &Physical,
    ) -> Result<()> {
        let listener = Arc::new(Listener {
            registration,
            entry,
            physical: physical.lifetime.clone(),
            queue: Arc::downgrade(&physical.capabilities),
        });
        let mut state = physical
            .capabilities
            .state
            .lock()
            .map_err(|_| Error::storage())?;
        state
            .listeners
            .retain(|_, l| l.registration.current() && !l.physical.is_cancelled());
        if state.listeners.len() >= 64 && !state.listeners.contains_key(id.as_str()) {
            return Err(Error::forbidden());
        }
        state.listeners.insert(id.as_str().into(), listener.clone());
        drop(state);
        let mut registry = self
            .capability_listeners
            .lock()
            .map_err(|_| Error::storage())?;
        registry.retain(|_, l| l.strong_count() > 0);
        registry.insert(id.as_str().into(), Arc::downgrade(&listener));
        Ok(())
    }
    pub(crate) async fn publish_capability(
        self: &Arc<Self>,
        declaration: Arc<NativeCapabilityDeclaration>,
        entry: Arc<Connection>,
        record: String,
    ) {
        let source = Arc::new(Source {
            declaration,
            entry,
            record,
        });
        let mut targets = {
            let Ok(mut registry) = self.capability_listeners.lock() else {
                return;
            };
            registry.retain(|_, l| l.strong_count() > 0);
            registry
                .values()
                .filter_map(Weak::upgrade)
                .filter(|l| {
                    let a = source.declaration.scope();
                    let b = l.registration.scope();
                    a.media.connection != b.media.connection
                        && a.media.session == b.media.session
                        && a.media.wire_conversation == b.media.wire_conversation
                        && a.media.wire_session == b.media.wire_session
                })
                .collect::<Vec<_>>()
        };
        let end = Instant::now() + Duration::from_secs(1);
        while !targets.is_empty() && Instant::now() < end && !self.stop.is_cancelled() {
            targets.retain(|target| {
                let Some(queue) = target.queue.upgrade() else {
                    return false;
                };
                match queue.enqueue(
                    &self.media.store,
                    source.clone(),
                    target.clone(),
                    self.media.store.limits().connections * 64,
                ) {
                    Ok(()) => false,
                    Err(e)
                        if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") =>
                    {
                        true
                    }
                    Err(_) => false,
                }
            });
            if !targets.is_empty() {
                tokio::select! {_=self.stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(5))=>{}}
            }
        }
    }
}

struct Policy {
    runtime: Weak<Runtime>,
    source: Arc<Source>,
    target: Arc<Listener>,
}
fn matches(
    entry: &Connection,
    scope: &NativeLifecycleScope,
    view: &connect_store::PeerConnectionProjection,
) -> bool {
    let origin = &entry.origin;
    scope.media.connection.as_str() == origin.runtime_connection_id()
        && scope.media.session.as_str() == origin.session_id()
        && scope.media.wire_conversation.as_deref() == Some(origin.conversation_id())
        && scope.media.wire_session == origin.session_id()
        && scope.media.wire_connection == origin.connection_id()
        && scope.media.owner == entry.principal.ownership_key()
        && scope.participant.as_str() == origin.participant_id()
        && scope.membership_epoch == origin.membership_epoch()
        && scope.device.as_str() == view.device_id
        && scope.endpoint.as_str() == view.connection.endpoint_id
        && scope.identity.as_str() == view.session.actor.identity_id
        && rvoip_core::staged_streams::reliable_data::same_credential(
            &scope.principal,
            &entry.principal,
        )
        && matches!(view.connection.state.as_str(), "ready" | "active")
}
#[async_trait::async_trait]
impl NativeCapabilityWritePolicy for Policy {
    async fn acquire(
        &self,
        declaration: &NativeCapabilityDeclaration,
        listener: &NativeCapabilityRegistration,
    ) -> std::result::Result<AuthorityLease, DispatchError> {
        if !std::ptr::eq(declaration, self.source.declaration.as_ref())
            || !std::ptr::eq(listener, self.target.registration.as_ref())
        {
            return Err(DispatchError::Authority);
        }
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        let read = loop {
            if runtime.stop.is_cancelled() || self.target.physical.is_cancelled() {
                return Err(DispatchError::Closed);
            }
            match runtime.media.store.try_native_capability_delivery_read(
                &self.target.entry.origin,
                &self.source.entry.origin,
                &self.source.record,
                declaration.payload(),
            ) {
                Ok(read) => break Arc::new(read),
                Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => {
                    tokio::select! {_=runtime.stop.cancelled()=>return Err(DispatchError::Closed),_=self.target.physical.cancelled()=>return Err(DispatchError::Closed),_=tokio::time::sleep(Duration::from_millis(5))=>{}}
                }
                Err(_) => return Err(DispatchError::Authority),
            }
        };
        if !read.current()
            || !matches(&self.target.entry, listener.scope(), read.target())
            || !read
                .source()
                .is_some_and(|v| matches(&self.source.entry, declaration.scope(), v))
        {
            return Err(DispatchError::Authority);
        }
        let remaining = (chrono::DateTime::from_timestamp(read.expires_at(), 0)
            .ok_or(DispatchError::Authority)?
            - chrono::Utc::now())
        .to_std()
        .map_err(|_| DispatchError::Authority)?;
        let current = read.clone();
        let physical = self.target.physical.clone();
        let stop = runtime.stop.clone();
        Ok(AuthorityLease::checking(
            read,
            Instant::now() + remaining.min(Duration::from_secs(1)),
            move || current.current() && !physical.is_cancelled() && !stop.is_cancelled(),
        ))
    }
}
