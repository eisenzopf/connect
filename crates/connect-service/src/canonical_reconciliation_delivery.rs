//! Latest routing-only journal hints own no idle permission or native grant.
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
};
use connect_protocol::{transitions::OperationRecord, Error, Result};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, DispatchError, HandlerDelivery},
    native_requests::NativeRequestOrigin,
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
pub(crate) struct Listener {
    origin: Arc<NativeRequestOrigin>,
    entry: Arc<Connection>,
    physical: CancellationToken,
    queue: Weak<Queue>,
}
impl Listener {
    fn current(&self) -> bool {
        self.origin.current() && !self.physical.is_cancelled()
    }
    pub(crate) fn recorded(self: &Arc<Self>, record: &OperationRecord) {
        let Some(revision) = record
            .result
            .as_ref()
            .and_then(|r| r["stream_revision"].as_u64())
        else {
            return;
        };
        if record.command_type != "stream.reconcile"
            || record.state != connect_protocol::transitions::OperationState::Completed
            || record
                .result
                .as_ref()
                .is_none_or(|r| r["resources_changed"] != false)
            || record.conversation_id.as_deref() != Some(self.entry.origin.conversation_id())
            || record.session_id.as_deref() != Some(self.entry.origin.session_id())
            || record.connection_id.as_deref() != Some(self.entry.origin.connection_id())
        {
            return;
        }
        let (Some(expected), Some(parent)) = (
            record.expected_revision,
            record.subscription_record_id.clone(),
        ) else {
            return;
        };
        let hint = connect_protocol::native_streams::ReconciliationHint {
            operation_id: record.operation_id.clone(),
            record_id: record.record_id.clone(),
            command_type: connect_protocol::native_streams::Command::Reconcile,
            expected_revision: expected,
            state: record.state,
            revision: record.revision,
            subscription_record_id: parent,
            stream_revision: revision,
            journal_hint: true,
            resources_changed: false,
        };
        if hint.validate().is_err() {
            return;
        }
        tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",operation=%hint.operation_id,revision=hint.stream_revision,current=self.current(),ready=self.origin.ready(),"Routing-only outcome recorded for hint");
        if let Some(queue) = self.queue.upgrade() {
            queue.enqueue(self.clone(), hint);
        }
    }
}
#[derive(Clone)]
struct Queued {
    listener: Arc<Listener>,
    hint: connect_protocol::native_streams::ReconciliationHint,
    revision: u64,
    at: Instant,
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
    items: BTreeMap<String, Queued>,
    order: VecDeque<String>,
    flight: Option<Flight>,
    #[cfg(feature = "test-support")]
    suppressed: BTreeMap<String, Arc<std::sync::atomic::AtomicUsize>>,
}
#[derive(Default)]
pub(crate) struct Queue {
    state: Mutex<State>,
    changed: tokio::sync::Notify,
}
impl Queue {
    #[cfg(feature = "test-support")]
    pub(crate) fn suppress_for_test(
        &self,
        connection: &str,
    ) -> Result<Option<Arc<std::sync::atomic::AtomicUsize>>> {
        let mut state = self.state.lock().map_err(|_| Error::storage())?;
        let Some(id) = state
            .listeners
            .iter()
            .find(|(_, listener)| {
                listener.current() && listener.entry.origin.connection_id() == connection
            })
            .map(|(id, _)| id.clone())
        else {
            return Ok(None);
        };
        let counter = state.suppressed.entry(id).or_default().clone();
        Ok(Some(counter))
    }
    fn enqueue(
        self: &Arc<Self>,
        listener: Arc<Listener>,
        hint: connect_protocol::native_streams::ReconciliationHint,
    ) {
        let revision = hint.stream_revision;
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let id = listener.entry.origin.runtime_connection_id().to_owned();
        tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",current=listener.current(),ready=listener.origin.ready(),registered=state.listeners.get(&id).is_some_and(|l|Arc::ptr_eq(l,&listener)),revision,"Routing hint enqueue eligibility");
        if !listener.current()
            || state
                .listeners
                .get(&id)
                .is_none_or(|current| !Arc::ptr_eq(current, &listener))
        {
            return;
        }
        #[cfg(feature = "test-support")]
        if let Some(counter) = state.suppressed.get(&id) {
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return;
        }
        if state.items.get(&id).is_some_and(|q| q.revision >= revision)
            || state.flight.as_ref().is_some_and(|f| {
                f.queued.listener.entry.origin.runtime_connection_id() == id
                    && f.queued.revision >= revision
            })
        {
            return;
        }
        if !state.items.contains_key(&id) {
            if state.items.len() >= 64 {
                return;
            }
            state.order.push_back(id.clone());
        }
        state.items.insert(
            id,
            Queued {
                listener,
                hint,
                revision,
                at: Instant::now(),
            },
        );
        drop(state);
        self.changed.notify_waiters();
    }
    fn next_ready(
        self: &Arc<Self>,
        runtime: &Arc<Runtime>,
    ) -> std::result::Result<Option<HandlerDelivery>, DispatchError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DispatchError::StorageUnavailable)?;
        let done = state
            .flight
            .as_ref()
            .and_then(|f| *f.result.lock().unwrap_or_else(|p| p.into_inner()));
        if let Some(result) = done {
            let flight = state.flight.take().expect("completed hint");
            if matches!(
                result,
                Err(DispatchError::Closed
                    | DispatchError::Capacity
                    | DispatchError::AuthorityUnavailable)
            ) && flight.queued.listener.current()
                && flight.queued.at.elapsed() < Duration::from_secs(5)
            {
                let id = flight
                    .queued
                    .listener
                    .entry
                    .origin
                    .runtime_connection_id()
                    .to_owned();
                if !state.items.contains_key(&id) {
                    state.order.push_back(id.clone());
                    state.items.insert(id, flight.queued.clone());
                }
            }
        }
        if state.flight.is_some() {
            return Ok(None);
        }
        while let Some(id) = state.order.pop_front() {
            let Some(queued) = state.items.remove(&id) else {
                continue;
            };
            if !queued.listener.current() || queued.at.elapsed() >= Duration::from_secs(5) {
                continue;
            }
            if !queued.listener.origin.ready() {
                state.order.push_back(id.clone());
                state.items.insert(id, queued);
                break;
            }
            let payload = serde_json::to_value(&queued.hint)
                .map_err(|_| DispatchError::StorageUnavailable)?;
            let policy = Arc::new(Policy {
                runtime: Arc::downgrade(runtime),
                listener: queued.listener.clone(),
                record: queued.hint.record_id.clone(),
                payload: payload.clone(),
            });
            let (delivery, receipt) = match queued.listener.origin.journal_delivery(payload, policy)
            {
                Ok(value) => value,
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",?error,"Routing hint construction refused");
                    continue;
                }
            };
            tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",revision=queued.revision,"Routing hint handed to reliable writer");
            let result = Arc::new(Mutex::new(None));
            let observed = result.clone();
            let queue = Arc::downgrade(self);
            let watch = tokio::spawn(async move {
                let result = tokio::time::timeout(Duration::from_secs(2), receipt)
                    .await
                    .ok()
                    .and_then(|r| r.ok())
                    .unwrap_or(Err(DispatchError::Closed));
                tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",?result,"Routing hint reliable writer settled");
                *observed.lock().unwrap_or_else(|p| p.into_inner()) = Some(result);
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
            tokio::select! {_=&mut changed=>(),_=runtime.stop.cancelled()=>return Err(DispatchError::Closed),_=tokio::time::sleep(Duration::from_millis(10))=>()}
        }
    }
}
impl Runtime {
    pub(crate) fn install_reconciliation_delivery(
        self: &Arc<Self>,
        entry: Arc<Connection>,
        origin: Arc<NativeRequestOrigin>,
        physical: &Physical,
    ) -> Result<Arc<Listener>> {
        let listener = Arc::new(Listener {
            entry,
            origin,
            physical: physical.lifetime.clone(),
            queue: Arc::downgrade(&physical.reconciliations),
        });
        let mut state = physical
            .reconciliations
            .state
            .lock()
            .map_err(|_| Error::storage())?;
        state.listeners.retain(|_, l| l.current());
        let id = listener.entry.origin.runtime_connection_id().to_owned();
        if state.listeners.len() >= 64 && !state.listeners.contains_key(&id) {
            return Err(Error::new(
                "capacity-exceeded",
                "Native reconciliation listener capacity exhausted",
            ));
        }
        state.listeners.insert(id, listener.clone());
        Ok(listener)
    }
}

struct Policy {
    runtime: Weak<Runtime>,
    listener: Arc<Listener>,
    record: String,
    payload: serde_json::Value,
}
#[async_trait::async_trait]
impl rvoip_uctp::v2::native_journal_delivery::NativeJournalWritePolicy for Policy {
    async fn acquire(
        &self,
        origin: &NativeRequestOrigin,
        payload: &serde_json::Value,
    ) -> std::result::Result<AuthorityLease, DispatchError> {
        if !std::ptr::eq(origin, self.listener.origin.as_ref()) || payload != &self.payload {
            return Err(DispatchError::Authority);
        }
        let runtime = self.runtime.upgrade().ok_or(DispatchError::Closed)?;
        let (read, hint) = loop {
            if !self.listener.current() || runtime.stop.is_cancelled() {
                return Err(DispatchError::Closed);
            }
            match runtime
                .media
                .store
                .try_native_reconciliation_hint_read(&self.listener.entry.origin, &self.record)
            {
                Ok(value) => break value,
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) =>
                {
                    tokio::select! {_=runtime.stop.cancelled()=>return Err(DispatchError::Closed),_=self.listener.physical.cancelled()=>return Err(DispatchError::Closed),_=tokio::time::sleep(Duration::from_millis(5))=>()}
                }
                Err(error) => {
                    tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",code=%error.code,"Routing hint current owner read refused");
                    return Err(DispatchError::Authority);
                }
            }
        };
        if serde_json::to_value(&hint).map_err(|_| DispatchError::StorageUnavailable)? != *payload {
            tracing::debug!(target:"connect_service::canonical_reconciliation_delivery","Routing hint immutable journal metadata mismatch");
            return Err(DispatchError::Authority);
        }
        let read = Arc::new(read);
        let view = read.media().target();
        let scope = origin.request_scope(read.stream_revision());
        let target = &self.listener.entry.origin;
        if scope.origin.media.connection.as_str() != target.runtime_connection_id()
            || scope.origin.media.wire_conversation.as_deref() != Some(target.conversation_id())
            || scope.origin.media.wire_session != target.session_id()
            || scope.origin.media.wire_connection != target.connection_id()
            || scope.origin.participant.as_str() != target.participant_id()
            || scope.origin.membership_epoch != target.membership_epoch()
            || scope.origin.device.as_str() != view.device_id
            || scope.origin.endpoint.as_str() != view.connection.endpoint_id
            || scope.origin.identity.as_str() != view.session.actor.identity_id
            || scope.connection_generation != view.connection.generation
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &scope.origin.principal,
                &self.listener.entry.principal,
            )
        {
            tracing::debug!(target:"connect_service::canonical_reconciliation_delivery",generation=scope.connection_generation,current_generation=view.connection.generation,"Routing hint physical source mismatch");
            return Err(DispatchError::Authority);
        }
        let remaining = (chrono::DateTime::from_timestamp(read.media().expires_at(), 0)
            .ok_or(DispatchError::Authority)?
            - chrono::Utc::now())
        .to_std()
        .map_err(|_| DispatchError::Authority)?;
        let current = read.clone();
        let listener = self.listener.clone();
        let stop = runtime.stop.clone();
        Ok(AuthorityLease::checking(
            read,
            Instant::now() + remaining.min(Duration::from_secs(1)),
            move || current.current() && listener.current() && !stop.is_cancelled(),
        ))
    }
}
