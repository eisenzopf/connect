//! Original signed signal custody; queued work retains metadata, not readers.
mod gateways;
use crate::{
    canonical_media::Connection,
    canonical_runtime::{Physical, Runtime},
    StoreInvocationAuthority,
};
use connect_protocol::{
    native_dtmf::{SourceObservation, SubmissionDisposition},
    transitions::OperationState,
    Error, Result,
};
use connect_store::{
    NativeDtmfBatch, NativeDtmfRead, NativeDtmfRecipient, NativeStreamLease, PeerAuthorityGuard,
};
use rvoip_core::{dtmf::*, ids::ConnectionId, staged_streams::MediaOwner};
use rvoip_uctp::v2::{
    auth_dispatch::{AuthorityLease, AuthorizedEnvelope, DispatchError},
    native_dtmf::{NativeDtmfObservation, NativeDtmfPolicy, NativeDtmfSink},
    native_dtmf_egress::NativeDtmfRegistration,
    Envelope, Scope,
};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(crate) struct Listener {
    pub(crate) registration: Arc<NativeDtmfRegistration>,
    pub(crate) entry: Arc<Connection>,
    pub(crate) id: String,
    pub(crate) physical: CancellationToken,
}
fn proof(a: &AuthorizedEnvelope) -> Result<&PeerAuthorityGuard> {
    a.authority::<StoreInvocationAuthority>()
        .and_then(|v| v.peer_guard())
        .ok_or_else(Error::forbidden)
}
fn reply(record: connect_protocol::transitions::OperationRecord) -> crate::canonical_native::Reply {
    crate::canonical_native::Reply {
        record,
        answered: false,
    }
}
struct Capture {
    store: Arc<connect_store::Store>,
    source: Arc<Connection>,
    authorized: Arc<AuthorizedEnvelope>,
    lease: NativeStreamLease,
    candidates: Vec<NativeDtmfRecipient>,
    gateways: Vec<connect_store::NativeDtmfGatewayRecipient>,
    batch: Mutex<Option<NativeDtmfBatch>>,
}
impl NativeDtmfPolicy for Capture {
    fn authorize(
        &self,
        observed: &NativeDtmfObservation,
        request: &AuthorizedEnvelope,
    ) -> std::result::Result<AuthorityLease, DispatchError> {
        let p = proof(request).map_err(|_| DispatchError::Authority)?;
        if !std::ptr::eq(request, self.authorized.as_ref())
            || !p.current()
            || observed.connection().as_str() != self.source.origin.runtime_connection_id()
            || Some(observed.operation_id()) != request.envelope().scope(Scope::Operation)
            || observed.participant().as_str() != self.source.origin.participant_id()
            || observed.membership_epoch() != self.source.origin.membership_epoch()
            || observed.disclosure().is_none()
        {
            return Err(DispatchError::Authority);
        }
        let a = self.authorized.clone();
        let current = a.clone();
        Ok(AuthorityLease::checking(
            a,
            Instant::now() + Duration::from_secs(1),
            move || {
                proof(&current).is_ok_and(|p| p.current())
                    && !current.invocation_cancellation().is_cancelled()
            },
        ))
    }
}
impl NativeDtmfSink for Capture {
    fn observe(
        &self,
        observed: &NativeDtmfObservation,
        current: &AuthorityLease,
    ) -> std::result::Result<(), DispatchError> {
        if !current.current() {
            return Err(DispatchError::Authority);
        }
        // A failed SQL freeze has accepted no observation and started no IO.
        // Reuse only these original registrations on bounded source retries.
        let candidates = self
            .candidates
            .iter()
            .map(|target| NativeDtmfRecipient {
                origin: target.origin.clone(),
                registration_id: target.registration_id.clone(),
            })
            .collect();
        let batch = self
            .store
            .prepare_native_mixed_dtmf_with_peer(
                &self.lease,
                self.source.origin.clone(),
                candidates,
                self.gateways
                    .iter()
                    .map(|t| connect_store::NativeDtmfGatewayRecipient {
                        origin: t.origin.clone(),
                        registration_id: t.registration_id.clone(),
                    })
                    .collect(),
                proof(&self.authorized).map_err(|_| DispatchError::Authority)?,
                SourceObservation {
                    envelope_id: observed.observation_id().into(),
                    received_at: observed.received_at(),
                    stream_revision: observed.stream_revision(),
                    credential_revision: observed.credential_revision(),
                    signing_key_id: observed.signing_key_id().into(),
                    signing_algorithm: match observed.signing_algorithm() {
                        rvoip_uctp::v2::payloads::SignatureAlg::EdDSA => "EdDSA",
                        rvoip_uctp::v2::payloads::SignatureAlg::ES256 => "ES256",
                        _ => return Err(DispatchError::Authority),
                    }
                    .into(),
                },
            )
            .map_err(|e| {
                if matches!(e.code.as_str(), "storage-unavailable" | "authority-busy") {
                    DispatchError::StorageUnavailable
                } else {
                    DispatchError::Authority
                }
            })?;
        *self
            .batch
            .lock()
            .map_err(|_| DispatchError::StorageUnavailable)? = Some(batch);
        Ok(())
    }
}
impl Runtime {
    async fn prepare_original_dtmf_route(
        &self,
        observed: &NativeDtmfObservation,
        recipient: ConnectionId,
        deadline: Instant,
    ) -> std::result::Result<DtmfRoute, DtmfWriteError> {
        loop {
            if self.stop.is_cancelled() {
                return Err(DtmfWriteError::Refused);
            }
            if Instant::now() >= deadline {
                return Err(DtmfWriteError::DeadlineBeforeWrite);
            }
            match observed.prepare_core_route(&self.core, recipient.clone()) {
                Err(DtmfWriteError::NotReady) => {
                    // Only the same immutable source/recipient metadata is
                    // recaptured. No route, writer job, permission reader or
                    // physical effect exists yet. Actual IO is invoked once.
                    tokio::select! {biased;
                        _ = self.stop.cancelled() => return Err(DtmfWriteError::Refused),
                        _ = tokio::time::sleep_until(deadline) => return Err(DtmfWriteError::DeadlineBeforeWrite),
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {},
                    }
                }
                result => return result,
            }
        }
    }
    pub(crate) fn enable_native_dtmf(
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
            .connection_native_adapter(id)?
            .native_dtmf_registration(id, a)
            .map_err(crate::canonical_native::registration_error)?;
        self.install_native_dtmf(id, entry, registration, physical)
    }
    pub(crate) fn refresh_native_dtmf(
        self: &Arc<Self>,
        id: &ConnectionId,
        result: &rvoip_uctp::v2::native::RefreshedNativeMedia,
        physical: &Physical,
    ) -> std::result::Result<(), DispatchError> {
        let entry = self.media.entry(id).map_err(|_| DispatchError::Authority)?;
        self.install_native_dtmf(id, entry, result.dtmf()?, physical)
            .map_err(|_| DispatchError::Authority)
    }
    fn install_native_dtmf(
        self: &Arc<Self>,
        id: &ConnectionId,
        entry: Arc<Connection>,
        registration: Arc<NativeDtmfRegistration>,
        physical: &Physical,
    ) -> Result<()> {
        let listener = Arc::new(Listener {
            registration: registration.clone(),
            entry,
            id: format!("dtmfreg_{}", uuid::Uuid::new_v4().simple()),
            physical: physical.lifetime.clone(),
        });
        let mut local = physical
            .dtmf_listeners
            .lock()
            .map_err(|_| Error::storage())?;
        local.retain(|_, v| v.registration.current() && !v.physical.is_cancelled());
        if local.len() >= 64 && !local.contains_key(id.as_str()) {
            return Err(Error::new(
                "capacity-exceeded",
                "Native DTMF registrations are full",
            ));
        }
        physical
            .dtmf
            .register(id.clone(), registration)
            .map_err(|_| Error::forbidden())?;
        local.insert(id.as_str().into(), listener.clone());
        drop(local);
        let mut registry = self.dtmf_listeners.lock().map_err(|_| Error::storage())?;
        registry.retain(|_, v| v.strong_count() > 0);
        registry.insert(id.as_str().into(), Arc::downgrade(&listener));
        Ok(())
    }
    pub(crate) async fn native_dtmf(
        self: &Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
    ) -> Result<crate::canonical_native::Reply> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let runtime = self.clone();
        {
            let mut work = self.work.lock().map_err(|_| Error::storage())?;
            while work.try_join_next().is_some() {}
            if self.stop.is_cancelled()
                || work.len() >= self.media.store.limits().connections.saturating_mul(2)
            {
                return Err(Error::new("busy", "Native control workers are at capacity"));
            }
            work.spawn(async move {
                runtime.dtmf_worker(a, physical, credential, tx).await;
            });
        }
        rx.await.map_err(|_| Error::storage())?
    }
    async fn dtmf_worker(
        self: Arc<Self>,
        a: Arc<AuthorizedEnvelope>,
        physical: Arc<Physical>,
        credential: String,
        tx: tokio::sync::oneshot::Sender<Result<crate::canonical_native::Reply>>,
    ) {
        let native = match self.native_adapter(physical.binding.transport()) {
            Ok(native) => native,
            Err(error) => {
                let _ = tx.send(Err(error));
                return;
            }
        };
        let store = &self.media.store;
        match proof(&a).and_then(|p| store.native_stream_record_with_peer(&credential, p)) {
            Ok(Some(record)) if record.state != OperationState::Accepted => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Ok(_) => (),
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
        let preflight = (|| {
            physical.validate(
                &a,
                a.authority::<StoreInvocationAuthority>()
                    .ok_or_else(Error::forbidden)?,
            )?;
            if physical.lifetime.is_cancelled() || self.stop.is_cancelled() {
                return Err(Error::forbidden());
            }
            let projected = store.project_native_stream_with_peer(&credential, proof(&a)?)?;
            let id = ConnectionId::from_string(projected.connection.runtime_connection_id);
            let actual = native
                .physical_source(&id)
                .map_err(|_| Error::forbidden())?;
            if !Arc::ptr_eq(&actual, &physical.binding) {
                return Err(Error::forbidden());
            }
            let source = self.media.entry(&id).map_err(|_| Error::forbidden())?;
            Ok((id, source))
        })();
        let (id, source) = match preflight {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        match proof(&a).and_then(|p| store.accept_native_stream_with_peer(&credential, p)) {
            Ok(record) if record.state == OperationState::Accepted => (),
            Ok(record) => {
                let _ = tx.send(Ok(reply(record)));
                return;
            }
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        }
        let (lease, started) =
            match proof(&a).and_then(|p| store.begin_native_stream_with_peer(&credential, p)) {
                Ok(Some(v)) => v,
                Ok(None) => {
                    let _ = tx.send(
                        proof(&a)
                            .and_then(|p| store.native_stream_record_with_peer(&credential, p))
                            .and_then(|v| v.ok_or_else(Error::forbidden))
                            .map(reply),
                    );
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            };
        let signal_expires_at = lease.expires_at();
        let remaining =
            Duration::from_secs((signal_expires_at - chrono::Utc::now().timestamp()).max(0) as u64);
        let preparation_deadline = Instant::now() + remaining.min(Duration::from_secs(2));
        let capture = async {
            let listeners = self
                .dtmf_listeners
                .lock()
                .map_err(|_| Error::storage())?
                .values()
                .filter_map(Weak::upgrade)
                .filter(|v| {
                    v.registration.current()
                        && !v.physical.is_cancelled()
                        && v.entry.origin.runtime_connection_id()
                            != source.origin.runtime_connection_id()
                        && v.entry.origin.conversation_id() == source.origin.conversation_id()
                        && v.entry.origin.session_id() == source.origin.session_id()
                })
                .map(|v| (v.id.clone(), v))
                .collect::<BTreeMap<_, _>>();
            let original_candidates = listeners
                .values()
                .map(|target| NativeDtmfRecipient {
                    origin: target.entry.origin.clone(),
                    registration_id: target.id.clone(),
                })
                .collect::<Vec<_>>();
            let candidates = self
                .native_preflight(&a, preparation_deadline, || {
                    let candidates = original_candidates
                        .iter()
                        .map(|target| NativeDtmfRecipient {
                            origin: target.origin.clone(),
                            registration_id: target.registration_id.clone(),
                        })
                        .collect();
                    store
                        .select_native_dtmf_recipients_with_peer(
                            &lease,
                            &source.origin,
                            candidates,
                            proof(&a)?,
                        )
                        .map_err(|error| {
                            if matches!(
                                error.code.as_str(),
                                "authority-busy" | "storage-unavailable"
                            ) {
                                Error::new(
                                    "authority-busy",
                                    "Native signal recipient selection is busy",
                                )
                            } else {
                                error
                            }
                        })
                })
                .await?;
            let gateway_legs = self
                .native_preflight(&a, preparation_deadline, || {
                    let mut legs = self
                        .media
                        .initial_dtmf_targets(&source.origin)
                        .map_err(|_| {
                            Error::new("authority-busy", "Gateway registration metadata is busy")
                        })?
                        .into_iter()
                        .map(crate::canonical_sip_signal::SipLeg::Initial)
                        .collect::<Vec<_>>();
                    legs.extend(
                        self.media
                            .established_dtmf_targets(&source.origin)
                            .map_err(|_| {
                                Error::new("authority-busy", "Established gateway metadata is busy")
                            })?
                            .into_iter()
                            .map(crate::canonical_sip_signal::SipLeg::Established),
                    );
                    let mut registrations = BTreeMap::new();
                    for leg in legs {
                        let id = leg.registration().ok_or_else(Error::forbidden)?.to_owned();
                        if registrations.insert(id, leg).is_some() {
                            return Err(Error::forbidden());
                        }
                    }
                    Ok(registrations)
                })
                .await?;
            let gateways = self
                .native_preflight(&a, preparation_deadline, || {
                    let candidates = gateway_legs
                        .iter()
                        .map(|(id, leg)| {
                            Ok(connect_store::NativeDtmfGatewayRecipient {
                                origin: leg.origin().map_err(|_| Error::forbidden())?,
                                registration_id: id.clone(),
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    store.select_native_dtmf_gateways_with_peer(
                        &lease,
                        source.origin.clone(),
                        candidates,
                        proof(&a)?,
                    )
                })
                .await?;
            if candidates.is_empty() && gateways.is_empty() {
                return Err(Error::new("not-recipient", "No current signal recipient"));
            }
            Ok((listeners, candidates, gateway_legs, gateways))
        }
        .await;
        let (listeners, candidates, gateway_legs, gateways) = match capture {
            Ok(v) => v,
            Err(e) => {
                let _ = tx.send(store.fail_native_stream_without_effect(lease, e).map(reply));
                return;
            }
        };
        let capture = Capture {
            store: store.clone(),
            source: source.clone(),
            authorized: a.clone(),
            lease,
            candidates,
            gateways,
            batch: Mutex::new(None),
        };
        let observed = match self
            .native_preflight(&a, preparation_deadline, || {
                native
                    .receive_managed_native_dtmf(&self.core, &id, &a, &capture, &capture)
                    .map_err(|error| {
                        tracing::debug!(
                            ?error,
                            stage = "source-capture",
                            "Native DTMF source capture deferred or refused"
                        );
                        match error {
                            rvoip_uctp::v2::native_negotiation::NativeOfferError::NotReady
                            | rvoip_uctp::v2::native_negotiation::NativeOfferError::Dispatch(
                                DispatchError::StorageUnavailable,
                            ) => {
                                Error::new("authority-busy", "Native signal source capture is busy")
                            }
                            _ => Error::forbidden(),
                        }
                    })
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(code=%e.code, "Native DTMF source capture refused");
                let _ = tx.send(
                    store
                        .fail_native_stream_without_effect(capture.lease, e)
                        .map(reply),
                );
                return;
            }
        };
        let Some(batch) = capture.batch.lock().ok().and_then(|mut b| b.take()) else {
            let _ = tx.send(Err(Error::storage()));
            return;
        };
        let batch = Arc::new(batch);
        drop(capture);
        drop(a);
        drop(credential);
        // Release the handler's original invocation before downstream work.
        // The first reply is durable preparation, not a local/remote delivery.
        let _ = tx.send(Ok(reply(started)));
        // Only frozen metadata is queued. Each worker obtains fresh short-lived
        // source/recipient permission at actual IO and owns its physical cleanup.
        // Match Core's bounded scheduler; the approved 50-recipient profile fits
        // in one wave. Queued work still uses the original operation expiry.
        let mut deliveries = tokio::task::JoinSet::new();
        let mut recorded = true;
        for (index, target) in batch.targets().iter().enumerate() {
            if deliveries.len() == rvoip_core::orchestrator::dtmf_routing::MAX_ACTIVE_DTMF_SEQUENCES
            {
                recorded &= matches!(deliveries.join_next().await, Some(Ok(true)));
            }
            let listener = listeners
                .get(target.registration_id())
                .expect("frozen original registration")
                .clone();
            deliveries.spawn(self.clone().deliver_native_dtmf(
                source.clone(),
                listener,
                observed.clone(),
                batch.clone(),
                index,
                signal_expires_at,
            ));
        }
        for target in batch.gateway_targets() {
            if deliveries.len() == rvoip_core::orchestrator::dtmf_routing::MAX_ACTIVE_DTMF_SEQUENCES
            {
                recorded &= matches!(deliveries.join_next().await, Some(Ok(true)));
            }
            let Some(leg) = gateway_legs.get(target.registration_id()).cloned() else {
                tracing::debug!("Original SIP DTMF registration is unavailable");
                recorded = false;
                continue;
            };
            deliveries.spawn(gateways::deliver(
                self.clone(),
                source.clone(),
                leg,
                observed.clone(),
                target.clone(),
                signal_expires_at,
            ));
        }
        // An uncertain/failed recipient must never abandon another recipient's
        // physical leases or captured outcome. Await every started worker before
        // settling; missing facts remain for the existing runtime recovery path.
        while let Some(result) = deliveries.join_next().await {
            recorded &= matches!(result, Ok(true));
        }
        if !recorded {
            return;
        }
        loop {
            match store.finish_native_dtmf(&batch) {
                Ok(_) => return,
                Err(e) if matches!(e.code.as_str(), "storage-unavailable" | "authority-busy") => (),
                Err(e) => {
                    tracing::debug!(code=%e.code,"Native DTMF settlement awaits runtime recovery");
                    return;
                }
            }
            tokio::select! {_=self.stop.cancelled()=>return,_=tokio::time::sleep(Duration::from_millis(10))=>{}}
        }
    }
    async fn deliver_native_dtmf(
        self: Arc<Self>,
        source: Arc<Connection>,
        listener: Arc<Listener>,
        observed: NativeDtmfObservation,
        batch: Arc<NativeDtmfBatch>,
        index: usize,
        signal_expires_at: i64,
    ) -> bool {
        let store = &self.media.store;
        let target = &batch.targets()[index];
        let policy = Arc::new(Policy {
            runtime: Arc::downgrade(&self),
            source: source.clone(),
            target: listener,
            observed: observed.clone(),
            batch: batch.clone(),
            index,
            flight: Arc::new(Flight::default()),
            touched: Arc::new(AtomicBool::new(false)),
        });
        let cancel = self.stop.child_token();
        let remaining = chrono::DateTime::from_timestamp(signal_expires_at, 0)
            .map(|expiry| expiry - chrono::Utc::now())
            .and_then(|remaining| remaining.to_std().ok())
            .unwrap_or(Duration::ZERO);
        let deadline = Instant::now() + remaining.min(Duration::from_secs(2));
        let result = match self
            .prepare_original_dtmf_route(
                &observed,
                ConnectionId::from_string(target.origin().runtime_connection_id()),
                deadline,
            )
            .await
        {
            Ok(route) => {
                self.core
                    .route_dtmf(&route, policy.clone(), cancel.clone(), deadline)
                    .await
            }
            Err(e) => {
                tracing::debug!(
                    ?e,
                    stage = "route-preparation",
                    "Native DTMF delivery refused"
                );
                Err(e)
            }
        };
        if let Err(error) = result {
            tracing::debug!(
                ?error,
                touched = policy.touched.load(Ordering::Acquire),
                stage = "route-outcome",
                "Native DTMF delivery stopped"
            );
        }
        cancel.cancel();
        // Core receipt/deadline can race physical cleanup. Close admission,
        // then wait until all actual writer leases have been destroyed.
        policy.flight.close();
        policy.flight.stopped().await;
        let disposition = match result {
            Ok(()) => SubmissionDisposition::LocalSubmitted,
            Err(DtmfWriteError::OutcomeUnknown) => SubmissionDisposition::OutcomeUnknown,
            Err(_) if policy.touched.load(Ordering::Acquire) => {
                SubmissionDisposition::OutcomeUnknown
            }
            Err(_) => SubmissionDisposition::NotStarted,
        };
        loop {
            match store.try_record_native_dtmf_outcome(target, disposition) {
                Ok(()) => break,
                Err(e) if matches!(e.code.as_str(), "storage-unavailable" | "authority-busy") => (),
                Err(e) => {
                    tracing::debug!(code=%e.code,"Native DTMF outcome awaits runtime recovery");
                    return false;
                }
            }
            tokio::select! {_=self.stop.cancelled()=>return false,_=tokio::time::sleep(Duration::from_millis(10))=>{}}
        }
        true
    }
}
#[derive(Default)]
pub(crate) struct Flight {
    pub(crate) state: Mutex<(bool, usize)>,
    pub(crate) changed: tokio::sync::Notify,
}
impl Flight {
    pub(crate) fn close(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).0 = true;
    }
    pub(crate) async fn stopped(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().unwrap_or_else(|p| p.into_inner()).1 == 0 {
                return;
            }
            changed.await;
        }
    }
}
struct LeaseOwner {
    read: Option<Arc<NativeDtmfRead>>,
    _owners: (MediaOwner, MediaOwner),
    flight: Arc<Flight>,
}
impl Drop for LeaseOwner {
    fn drop(&mut self) {
        drop(self.read.take());
        self.flight
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .1 -= 1;
        self.flight.changed.notify_waiters();
    }
}
pub(crate) struct Touch(pub(crate) Arc<AtomicBool>);
impl DtmfIoFence for Touch {
    fn poll(&self, io: &mut dyn FnMut() -> DtmfIoPoll) -> DtmfIoPoll {
        self.0.store(true, Ordering::Release);
        io()
    }
}
struct Policy {
    runtime: Weak<Runtime>,
    source: Arc<Connection>,
    target: Arc<Listener>,
    observed: NativeDtmfObservation,
    batch: Arc<NativeDtmfBatch>,
    index: usize,
    flight: Arc<Flight>,
    touched: Arc<AtomicBool>,
}
#[async_trait::async_trait]
impl DtmfRoutePolicy for Policy {
    async fn acquire_native(
        &self,
        route: &DtmfRouteScope<'_>,
        native: &DtmfNativeWriteScope<'_>,
    ) -> std::result::Result<DtmfWriteLease, DtmfWriteError> {
        let runtime = self.runtime.upgrade().ok_or(DtmfWriteError::Refused)?;
        let target = &self.batch.targets()[self.index];
        let source = &self.source.origin;
        let recipient = &self.target.entry.origin;
        let actual = native.recipient;
        let wire = &self.batch.intent();
        tracing::debug!(
            stage = "native-scope",
            "Native DTMF writer acquiring current signal authority"
        );
        if route.source.connection().as_str() != source.runtime_connection_id()
            || route.recipient.connection().as_str() != recipient.runtime_connection_id()
            || route.source.session().as_str() != source.session_id()
            || route.recipient.session().as_str() != recipient.session_id()
            || route.source.participant().as_str() != source.participant_id()
            || route.recipient.participant().as_str() != recipient.participant_id()
            || route.source.conversation().as_str() != source.conversation_id()
            || route.recipient.conversation().as_str() != recipient.conversation_id()
            || route.intent != native.intent
            || route.observation != native.observation
            || route.intent.operation_id() != wire.operation_id
            || route.intent.digits() != self.observed.digits()
            || route.intent.duration_ms() != self.observed.duration_ms()
            || route.intent.method()
                != match self.observed.method() {
                    rvoip_uctp::v2::native_dtmf::NativeDtmfMethod::Info => DtmfMethod::Info,
                    rvoip_uctp::v2::native_dtmf::NativeDtmfMethod::Rfc4733 => DtmfMethod::Rfc4733,
                }
            || route.observation.observation_id() != Some(self.observed.observation_id())
            || route.observation.received_at() != self.observed.received_at()
            || route.observation.kind() != DtmfSourceKind::Native
            || actual.connection.as_str() != recipient.runtime_connection_id()
            || actual.session.as_str() != recipient.session_id()
            || actual.conversation.as_str() != recipient.conversation_id()
            || actual.participant.as_str() != recipient.participant_id()
            || actual.membership_epoch != recipient.membership_epoch()
            || actual.wire_connection != recipient.connection_id()
            || actual.wire_session != recipient.session_id()
            || actual.wire_conversation != recipient.conversation_id()
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &actual.principal,
                &self.target.entry.principal,
            )
            || !self.target.registration.current()
            || self.target.physical.is_cancelled()
            || runtime.stop.is_cancelled()
        {
            tracing::debug!(
                stage = "native-scope",
                "Native DTMF immutable scope refused"
            );
            return Err(DtmfWriteError::Refused);
        }
        let frame = Envelope::parse(native.wire_envelope).map_err(|_| DtmfWriteError::Refused)?;
        if frame.kind() != "dtmf.received"
            || frame.scope(Scope::Operation) != Some(wire.operation_id.as_str())
            || frame.scope(Scope::Conversation) != Some(recipient.conversation_id())
            || frame.scope(Scope::Session) != Some(recipient.session_id())
            || frame.scope(Scope::Connection) != Some(recipient.connection_id())
            || frame.original()["payload"]["digits"] != self.observed.digits()
            || frame.original()["payload"]["duration_ms"] != self.observed.duration_ms()
            || frame.original()["payload"]["source"] != "uctp-native"
            || frame.original()["payload"]["received_at"]
                != serde_json::json!(self.observed.received_at())
        {
            tracing::debug!(
                stage = "signed-frame",
                "Native DTMF signed frame scope refused"
            );
            return Err(DtmfWriteError::Refused);
        }
        // Scene metadata carries no permission. Wait for its try-locks before
        // acquiring domain readers, so a queued authority writer can progress.
        // Actual IO still composes this original/current scene fence with the
        // subsequently acquired exact fresh source/recipient Store interval.
        let disclosure = self.observed.disclosure().ok_or(DtmfWriteError::Refused)?;
        let fence = loop {
            if self
                .flight
                .state
                .lock()
                .map_err(|_| DtmfWriteError::Refused)?
                .0
                || runtime.stop.is_cancelled()
                || self.target.physical.is_cancelled()
            {
                return Err(DtmfWriteError::Refused);
            }
            match runtime.core.native_dtmf_audience_fence(disclosure, actual) {
                Ok(fence) => break fence,
                Err(
                    rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                        rvoip_core::staged_streams::StreamControlError::NotReady,
                    ),
                ) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(error) => {
                    tracing::debug!(?error, stage = "audience", "Native DTMF admission refused");
                    return Err(DtmfWriteError::Refused);
                }
            }
        };
        'reservation: loop {
            let read = loop {
                if self
                    .flight
                    .state
                    .lock()
                    .map_err(|_| DtmfWriteError::Refused)?
                    .0
                    || runtime.stop.is_cancelled()
                    || self.target.physical.is_cancelled()
                {
                    return Err(DtmfWriteError::Refused);
                }
                match runtime.media.store.try_native_dtmf_read(target) {
                    Ok(v) => break Arc::new(v),
                    Err(e)
                        if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") =>
                    {
                        tokio::time::sleep(Duration::from_millis(5)).await
                    }
                    Err(error) => {
                        tracing::debug!(code=%error.code,stage="fresh-signal-read","Native DTMF admission refused");
                        return Err(DtmfWriteError::Refused);
                    }
                }
            };
            let view = read.media().target();
            if actual.identity.as_str() != view.session.actor.identity_id
                || actual.device.as_str() != view.device_id
                || actual.endpoint.as_str() != view.connection.endpoint_id
            {
                return Err(DtmfWriteError::Refused);
            }
            let source_view = read.media().source().ok_or(DtmfWriteError::Refused)?;
            let source_owner = self
                .source
                .owner(source_view, &self.source.principal, false)
                .map_err(|error| {
                    tracing::debug!(
                        ?error,
                        stage = "source-owner",
                        "Native DTMF admission refused"
                    );
                    DtmfWriteError::Refused
                })?;
            let target_owner = self
                .target
                .entry
                .owner(view, &actual.principal, false)
                .map_err(|error| {
                    tracing::debug!(
                        ?error,
                        stage = "recipient-owner",
                        "Native DTMF admission refused"
                    );
                    DtmfWriteError::Refused
                })?;
            let source_owner = self
                .source
                .activation()
                .map_err(|_| DtmfWriteError::Refused)?
                .ok_or(DtmfWriteError::Refused)?
                .current(source_owner)
                .map_err(|error| {
                    tracing::debug!(
                        ?error,
                        stage = "source-activation",
                        "Native DTMF admission refused"
                    );
                    DtmfWriteError::Refused
                })?;
            let target_owner = self
                .target
                .entry
                .activation()
                .map_err(|_| DtmfWriteError::Refused)?
                .ok_or(DtmfWriteError::Refused)?
                .current(target_owner)
                .map_err(|error| {
                    tracing::debug!(
                        ?error,
                        stage = "recipient-activation",
                        "Native DTMF admission refused"
                    );
                    DtmfWriteError::Refused
                })?;
            let remaining = (chrono::DateTime::from_timestamp(read.expires_at(), 0)
                .ok_or(DtmfWriteError::Refused)?
                - chrono::Utc::now())
            .to_std()
            .map_err(|_| DtmfWriteError::Refused)?;
            let current = Arc::downgrade(&read);
            let stop = runtime.stop.clone();
            let physical = self.target.physical.clone();
            let reserved = {
                let mut flight = self
                    .flight
                    .state
                    .lock()
                    .map_err(|_| DtmfWriteError::Refused)?;
                if flight.0 || !read.current() {
                    return Err(DtmfWriteError::Refused);
                }
                let result = runtime
                    .media
                    .store
                    .try_start_native_dtmf_write(&read, frame.id());
                if result.is_ok() {
                    flight.1 += 1;
                }
                result
            };
            match reserved {
                Ok(()) => (),
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "authority-busy" | "storage-unavailable"
                    ) =>
                {
                    tracing::debug!(code=%error.code,stage="frame-reservation","Native DTMF reservation deferred before IO");
                    // The signed frame is unchanged and no write lease was issued.
                    // Release all actual readers/owners before the bounded wait;
                    // the next pass must reacquire complete current authority.
                    drop(source_owner);
                    drop(target_owner);
                    drop(read);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue 'reservation;
                }
                Err(error) => {
                    tracing::debug!(code=%error.code,stage="frame-reservation","Native DTMF reservation refused");
                    return Err(DtmfWriteError::Refused);
                }
            }
            return DtmfWriteLease::new(
                LeaseOwner {
                    read: Some(read),
                    _owners: (source_owner, target_owner),
                    flight: self.flight.clone(),
                },
                Instant::now() + remaining.min(Duration::from_millis(200)),
                move || {
                    current.upgrade().is_some_and(|r| r.current())
                        && !stop.is_cancelled()
                        && !physical.is_cancelled()
                },
            )
            .with_fence(fence.clone())?
            .with_fence(Arc::new(Touch(self.touched.clone())));
        }
    }
}
