//! Actual managed SIP receive custody. Queues contain no permission readers.
use crate::{
    canonical_native_dtmf::{Flight, Listener, Touch},
    canonical_runtime::Runtime,
    canonical_sip_signal::SipLeg,
};
use connect_protocol::{
    native_dtmf::SubmissionDisposition as D,
    received_dtmf::{RtpObservation, SourceKind, SourceObservation},
    Error, Result,
};
use connect_store::{
    NativeDtmfRecipient, ReceivedDtmfPreparation, ReceivedDtmfRead, ReceivedDtmfTarget,
};
use rvoip_core::{dtmf::*, ids::ConnectionId, staged_streams::MediaOwner};
use rvoip_uctp::v2::{Envelope, Scope};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Weak,
    },
    time::Duration,
};
use tokio::time::Instant;
fn busy(e: &Error) -> bool {
    matches!(e.code.as_str(), "storage-unavailable" | "authority-busy")
}
fn observation(observed: &ReceivedDtmfObservation) -> Result<SourceObservation> {
    let input = observed.input();
    let kind = match input.kind() {
        DtmfSourceKind::SipInfo => SourceKind::SipInfo,
        DtmfSourceKind::SipRfc4733 => SourceKind::SipRfc4733,
        _ => return Err(Error::forbidden()),
    };
    let o = SourceObservation {
        kind,
        observation_id: input.observation_id().into(),
        received_at: input.received_at(),
        digits: input.digits().into(),
        duration_ms: input.duration_ms(),
        rtp: input.rtp().map(|r| RtpObservation {
            ssrc: r.ssrc,
            payload_type: r.payload_type,
            clock_rate_hz: r.clock_rate_hz,
            rtp_timestamp: r.rtp_timestamp,
            extended_timestamp: r.extended_timestamp,
            last_segment_timestamp: r.last_segment_timestamp,
            duration_samples: r.duration_samples,
            event: r.event,
            start_observed: r.start_observed,
        }),
    };
    o.validate()?;
    Ok(o)
}
impl Runtime {
    pub(crate) fn enqueue_received_dtmf(
        self: &Arc<Self>,
        leg: SipLeg,
        observed: ReceivedDtmfObservation,
    ) -> Result<()> {
        let source = leg.origin().map_err(|_| Error::forbidden())?;
        if observed.disclosure().is_none()
            || observed.source().connection()
                != &leg.connection().map_err(|_| Error::forbidden())?
            || observed.source().session().as_str() != source.session_id()
            || observed.source().conversation().as_str() != source.conversation_id()
            || observed.source().participant().as_str() != source.participant_id()
        {
            return Err(Error::forbidden());
        }
        let facts = observation(&observed)?;
        // Capture original installed registrations before queuing or metadata
        // waits. The sealed Core disclosure independently closes late audiences.
        let listeners = self
            .dtmf_listeners
            .lock()
            .map_err(|_| Error::storage())?
            .values()
            .filter_map(Weak::upgrade)
            .filter(|v| {
                v.registration.current()
                    && !v.physical.is_cancelled()
                    && v.entry.origin.session_id() == source.session_id()
                    && v.entry.origin.conversation_id() == source.conversation_id()
            })
            .map(|v| (v.id.clone(), v))
            .collect::<BTreeMap<_, _>>();
        let mut work = self.work.lock().map_err(|_| Error::storage())?;
        while work.try_join_next().is_some() {}
        if self.stop.is_cancelled()
            || leg.cancelled()
            || work.len() >= self.media.store.limits().connections.saturating_mul(2)
        {
            return Err(Error::new(
                "busy",
                "Received signal workers are at capacity",
            ));
        }
        let runtime = self.clone();
        work.spawn(async move {
            if let Err(error) = runtime.received_dtmf_worker(leg, observed, facts, listeners).await {
                tracing::debug!(code=%error.code, "Received DTMF remains unsubmitted or awaits journal recovery");
            }
        });
        Ok(())
    }
    async fn received_dtmf_worker(
        self: Arc<Self>,
        leg: SipLeg,
        observed: ReceivedDtmfObservation,
        facts: SourceObservation,
        listeners: BTreeMap<String, Arc<Listener>>,
    ) -> Result<()> {
        let source = leg.origin().map_err(|_| Error::forbidden())?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let batch = loop {
            if self.stop.is_cancelled() || leg.cancelled() || Instant::now() >= deadline {
                return Err(Error::forbidden());
            }
            let candidates = listeners
                .values()
                .map(|v| NativeDtmfRecipient {
                    origin: v.entry.origin.clone(),
                    registration_id: v.id.clone(),
                })
                .collect();
            match self
                .media
                .store
                .prepare_received_dtmf(source.clone(), facts.clone(), candidates)
            {
                Ok(ReceivedDtmfPreparation::New(batch)) => break batch,
                Ok(ReceivedDtmfPreparation::Replay(_)) => return Ok(()),
                Err(e) if busy(&e) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(e) => return Err(e),
            }
        };
        let mut jobs = tokio::task::JoinSet::new();
        let mut recorded = true;
        for target in batch.targets() {
            if jobs.len() == rvoip_core::orchestrator::dtmf_routing::MAX_ACTIVE_DTMF_SEQUENCES {
                recorded &= matches!(jobs.join_next().await, Some(Ok(true)));
            }
            let listener = listeners
                .get(target.registration_id())
                .ok_or_else(Error::forbidden)?
                .clone();
            jobs.spawn(self.clone().deliver_received_dtmf(
                leg.clone(),
                observed.clone(),
                target.clone(),
                listener,
            ));
        }
        while let Some(result) = jobs.join_next().await {
            recorded &= matches!(result, Ok(true));
        }
        if !recorded {
            return Err(Error::new(
                "reconciliation-required",
                "Received signal outcomes await runtime recovery",
            ));
        }
        loop {
            match self.media.store.finish_received_dtmf(&batch) {
                Ok(_) => return Ok(()),
                Err(e) if busy(&e) => (),
                Err(e) => return Err(e),
            }
            tokio::select! {biased;_=self.stop.cancelled()=>return Err(Error::forbidden()),_=tokio::time::sleep(Duration::from_millis(10))=>{}}
        }
    }
    async fn deliver_received_dtmf(
        self: Arc<Self>,
        leg: SipLeg,
        observed: ReceivedDtmfObservation,
        target: ReceivedDtmfTarget,
        listener: Arc<Listener>,
    ) -> bool {
        let policy = Arc::new(Policy {
            runtime: Arc::downgrade(&self),
            leg: leg.clone(),
            target: target.clone(),
            listener,
            observed: observed.clone(),
            flight: Arc::new(Flight::default()),
            touched: Arc::new(AtomicBool::new(false)),
        });
        let remaining = (chrono::DateTime::from_timestamp(target.expires_at(), 0)
            .unwrap_or_else(chrono::Utc::now)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or(Duration::ZERO);
        let deadline = Instant::now() + remaining.min(Duration::from_secs(2));
        let cancel = self.stop.child_token();
        let result = async {
            if !target.observation().forwardable() {
                return Err(DtmfWriteError::Refused);
            }
            let route = loop {
                if self.stop.is_cancelled() || leg.cancelled() || Instant::now() >= deadline {
                    return Err(DtmfWriteError::Refused);
                }
                let method = match target.observation().kind {
                    SourceKind::SipInfo => DtmfMethod::Info,
                    SourceKind::SipRfc4733 => DtmfMethod::Rfc4733,
                };
                match observed.prepare_route(
                    &self.core,
                    ConnectionId::from_string(target.origin().runtime_connection_id()),
                    target.operation_id().into(),
                    method,
                ) {
                    Err(DtmfWriteError::NotReady) => {
                        tokio::time::sleep(Duration::from_millis(5)).await
                    }
                    result => break result?,
                }
            };
            self.core
                .route_dtmf(&route, policy.clone(), cancel.clone(), deadline)
                .await
        }
        .await;
        cancel.cancel();
        policy.flight.close();
        policy.flight.stopped().await;
        let disposition = match result {
            Ok(()) => D::LocalSubmitted,
            Err(DtmfWriteError::OutcomeUnknown) => D::OutcomeUnknown,
            Err(_) if policy.touched.load(Ordering::Acquire) => D::OutcomeUnknown,
            Err(_) => D::NotStarted,
        };
        loop {
            match self
                .media
                .store
                .try_record_received_dtmf_outcome(&target, disposition)
            {
                Ok(()) => return true,
                Err(e) if busy(&e) => (),
                Err(_) => return false,
            }
            tokio::select! {biased;_=self.stop.cancelled()=>return false,_=tokio::time::sleep(Duration::from_millis(10))=>{}}
        }
    }
}
struct Policy {
    runtime: Weak<Runtime>,
    leg: SipLeg,
    target: ReceivedDtmfTarget,
    listener: Arc<Listener>,
    observed: ReceivedDtmfObservation,
    flight: Arc<Flight>,
    touched: Arc<AtomicBool>,
}
struct Lease {
    read: Option<Arc<ReceivedDtmfRead>>,
    _owner: MediaOwner,
    flight: Arc<Flight>,
}
impl Drop for Lease {
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
impl Policy {
    fn stopped(&self, runtime: &Runtime) -> bool {
        self.flight.state.lock().map_or(true, |s| s.0)
            || runtime.stop.is_cancelled()
            || self.leg.cancelled()
            || self.listener.physical.is_cancelled()
            || !self.listener.registration.current()
    }
}
#[async_trait::async_trait]
impl DtmfRoutePolicy for Policy {
    async fn acquire_native(
        &self,
        route: &DtmfRouteScope<'_>,
        native: &DtmfNativeWriteScope<'_>,
    ) -> std::result::Result<DtmfWriteLease, DtmfWriteError> {
        let runtime = self.runtime.upgrade().ok_or(DtmfWriteError::Refused)?;
        let source = self.target.source();
        let recipient = self.target.origin();
        let actual = native.recipient;
        let facts = self.target.observation();
        let method = match facts.kind {
            SourceKind::SipInfo => DtmfMethod::Info,
            SourceKind::SipRfc4733 => DtmfMethod::Rfc4733,
        };
        if self.stopped(&runtime)
            || !self.leg.matches(source)
            || !Arc::ptr_eq(&self.listener.entry.origin, recipient)
            || route.source.connection() != &self.leg.connection()?
            || route.source.session().as_str() != source.session_id()
            || route.source.conversation().as_str() != source.conversation_id()
            || route.source.participant().as_str() != source.participant_id()
            || route.recipient.connection().as_str() != recipient.runtime_connection_id()
            || route.recipient.session().as_str() != recipient.session_id()
            || route.recipient.conversation().as_str() != recipient.conversation_id()
            || route.recipient.participant().as_str() != recipient.participant_id()
            || route.intent != native.intent
            || route.observation != native.observation
            || route.intent.operation_id() != self.target.operation_id()
            || route.intent.digits() != facts.digits
            || route.intent.duration_ms() != facts.duration_ms
            || route.intent.method() != method
            || route.observation.kind().as_str() != facts.kind.as_str()
            || route.observation.observation_id() != Some(facts.observation_id.as_str())
            || route.observation.received_at() != facts.received_at
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
                &self.listener.entry.principal,
            )
        {
            return Err(DtmfWriteError::Refused);
        }
        let frame = Envelope::parse(native.wire_envelope).map_err(|_| DtmfWriteError::Refused)?;
        if frame.kind() != "dtmf.received"
            || frame.scope(Scope::Operation) != Some(self.target.operation_id())
            || frame.scope(Scope::Conversation) != Some(recipient.conversation_id())
            || frame.scope(Scope::Session) != Some(recipient.session_id())
            || frame.scope(Scope::Connection) != Some(recipient.connection_id())
            || frame.original()["payload"]["digits"] != facts.digits
            || frame.original()["payload"]["duration_ms"] != facts.duration_ms
            || frame.original()["payload"]["source"] != facts.kind.as_str()
            || frame.original()["payload"]["received_at"] != serde_json::json!(facts.received_at)
        {
            return Err(DtmfWriteError::Refused);
        }
        let disclosure = self.observed.disclosure().ok_or(DtmfWriteError::Refused)?;
        let fence = loop {
            if self.stopped(&runtime) {
                return Err(DtmfWriteError::Refused);
            }
            match runtime
                .core
                .received_native_dtmf_audience_fence(disclosure, actual)
            {
                Ok(f) => break f,
                Err(
                    rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                        rvoip_core::staged_streams::StreamControlError::NotReady,
                    ),
                ) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(_) => return Err(DtmfWriteError::Refused),
            }
        };
        loop {
            if self.stopped(&runtime) {
                return Err(DtmfWriteError::Refused);
            }
            let read = match runtime.media.store.try_received_dtmf_read(&self.target) {
                Ok(r) => Arc::new(r),
                Err(e) if busy(&e) => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(_) => return Err(DtmfWriteError::Refused),
            };
            let view = read.pair().native().target();
            if actual.identity.as_str() != view.session.actor.identity_id
                || actual.device.as_str() != view.device_id
                || actual.endpoint.as_str() != view.connection.endpoint_id
            {
                return Err(DtmfWriteError::Refused);
            }
            let owner = self
                .listener
                .entry
                .owner(view, &actual.principal, false)
                .map_err(|_| DtmfWriteError::Refused)?;
            let owner = self
                .listener
                .entry
                .activation()
                .map_err(|_| DtmfWriteError::Refused)?
                .ok_or(DtmfWriteError::Refused)?
                .current(owner)
                .map_err(|_| DtmfWriteError::Refused)?;
            let remaining = (chrono::DateTime::from_timestamp(read.expires_at(), 0)
                .ok_or(DtmfWriteError::Refused)?
                - chrono::Utc::now())
            .to_std()
            .map_err(|_| DtmfWriteError::Refused)?;
            let reserved = {
                let mut flight = self
                    .flight
                    .state
                    .lock()
                    .map_err(|_| DtmfWriteError::Refused)?;
                if flight.0 || !read.current() {
                    return Err(DtmfWriteError::Refused);
                }
                let r = runtime
                    .media
                    .store
                    .try_start_received_dtmf_write(&read, frame.id());
                if r.is_ok() {
                    flight.1 += 1;
                }
                r
            };
            match reserved {
                Ok(()) => (),
                Err(e) if busy(&e) => {
                    drop(owner);
                    drop(read);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(_) => return Err(DtmfWriteError::Refused),
            }
            let current = Arc::downgrade(&read);
            let stop = runtime.stop.clone();
            let leg = self.leg.clone();
            let physical = self.listener.physical.clone();
            return DtmfWriteLease::new(
                Lease {
                    read: Some(read),
                    _owner: owner,
                    flight: self.flight.clone(),
                },
                Instant::now() + remaining.min(Duration::from_millis(200)),
                move || {
                    current.upgrade().is_some_and(|r| r.current())
                        && !stop.is_cancelled()
                        && !leg.cancelled()
                        && !physical.is_cancelled()
                },
            )
            .with_fence(fence.clone())?
            .with_fence(Arc::new(Touch(self.touched.clone())));
        }
    }
}
