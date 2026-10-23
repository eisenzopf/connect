//! One frozen native-source SIP schedule; no queued permission readers.
use super::*;
use crate::initial_telephone::policy::Leg as InitialLeg;
use connect_store::{NativeDtmfGatewayOrigin, NativeDtmfGatewayRead, NativeDtmfGatewayTarget};
type WriteResult<T> = std::result::Result<T, DtmfWriteError>;

#[derive(Clone)]
pub(super) enum GatewayLeg {
    Initial(Arc<InitialLeg>),
    Established(Arc<crate::telephone_replacement::media::Policy>),
}
impl GatewayLeg {
    pub(super) fn registration(&self) -> Option<&str> {
        match self {
            Self::Initial(leg) => leg.dtmf_registration.get().map(String::as_str),
            Self::Established(policy) => Some(&policy.dtmf_registration),
        }
    }
    pub(super) fn origin(&self) -> WriteResult<NativeDtmfGatewayOrigin> {
        match self {
            Self::Initial(leg) => leg
                .media
                .get()
                .cloned()
                .map(NativeDtmfGatewayOrigin::Initial)
                .ok_or(DtmfWriteError::Refused),
            Self::Established(policy) => policy
                .established_origin()
                .map(NativeDtmfGatewayOrigin::Established)
                .ok_or(DtmfWriteError::Refused),
        }
    }
    fn matches(&self, origin: &NativeDtmfGatewayOrigin) -> bool {
        match (self, origin) {
            (Self::Initial(leg), NativeDtmfGatewayOrigin::Initial(origin)) => leg
                .media
                .get()
                .is_some_and(|actual| Arc::ptr_eq(actual, origin)),
            (Self::Established(policy), NativeDtmfGatewayOrigin::Established(origin)) => policy
                .established_origin()
                .is_some_and(|actual| Arc::ptr_eq(&actual, origin)),
            _ => false,
        }
    }
    fn cancelled(&self) -> bool {
        match self {
            Self::Initial(leg) => leg.cancel.is_cancelled(),
            Self::Established(policy) => !policy.established_signal_current(),
        }
    }
    fn connection(&self) -> WriteResult<ConnectionId> {
        self.origin()
            .map(|origin| ConnectionId::from_string(origin.runtime_connection_id()))
    }
    fn principal(&self) -> WriteResult<rvoip_core::AuthenticatedPrincipal> {
        match self {
            Self::Initial(leg) => Ok(leg.target.principal.clone()),
            Self::Established(policy) => policy.principal().map_err(|_| DtmfWriteError::Refused),
        }
    }
    fn controller(&self) -> WriteResult<Arc<rvoip_core::staged_streams::StreamController>> {
        if self.cancelled() {
            return Err(DtmfWriteError::Refused);
        }
        match self {
            Self::Initial(leg) => {
                let controller = leg
                    .controller
                    .get()
                    .and_then(Weak::upgrade)
                    .ok_or(DtmfWriteError::Refused)?;
                let prepared = leg.prepared.get().ok_or(DtmfWriteError::Refused)?;
                if !prepared.is_current() || !controller.is_bound_to_outbound(prepared) {
                    return Err(DtmfWriteError::Refused);
                }
                Ok(controller)
            }
            Self::Established(policy) => {
                let controller = policy.controller().map_err(|_| DtmfWriteError::Refused)?;
                if !controller.is_bound_to_replacement(&policy.pending) {
                    return Err(DtmfWriteError::Refused);
                }
                Ok(controller)
            }
        }
    }
    fn matches_member(&self, member: &connect_protocol::membership::MemberView) -> bool {
        match self {
            Self::Initial(leg) => {
                member.participant_id == leg.target.member.participant().as_str()
                    && member.membership_epoch == leg.target.member.epoch()
                    && member.identity_id == leg.target.principal.subject
            }
            Self::Established(policy) => policy.established_origin().is_some_and(|origin| {
                let s = origin.snapshot();
                member.participant_id == s.participant_id
                    && member.membership_epoch == s.membership_epoch
                    && member.identity_id == s.identity_id
            }),
        }
    }
}

struct Policy {
    runtime: Weak<Runtime>,
    source: Arc<Connection>,
    leg: GatewayLeg,
    observed: NativeDtmfObservation,
    target: NativeDtmfGatewayTarget,
    attempt: String,
    flight: Arc<Flight>,
    touched: Arc<AtomicBool>,
}
struct Owner {
    read: Option<Arc<NativeDtmfGatewayRead>>,
    flight: Arc<Flight>,
}
impl Drop for Owner {
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
    fn scope(&self, route: &DtmfRouteScope<'_>, method: DtmfMethod) -> WriteResult<()> {
        let source = &self.source.origin;
        let phone = self.target.origin();
        if !self.leg.matches(phone)
            || self.leg.registration() != Some(self.target.registration_id())
            || self.leg.cancelled()
            || route.source.connection().as_str() != source.runtime_connection_id()
            || route.source.session().as_str() != source.session_id()
            || route.source.conversation().as_str() != source.conversation_id()
            || route.source.participant().as_str() != source.participant_id()
            || route.recipient.connection().as_str() != phone.runtime_connection_id()
            || route.recipient.session().as_str()
                != match phone {
                    NativeDtmfGatewayOrigin::Initial(o) => &o.source().request.session_id,
                    NativeDtmfGatewayOrigin::Established(o) => &o.snapshot().session_id,
                }
            || route.recipient.conversation().as_str()
                != match phone {
                    NativeDtmfGatewayOrigin::Initial(o) => &o.source().request.conversation_id,
                    NativeDtmfGatewayOrigin::Established(o) => &o.snapshot().conversation_id,
                }
            || route.recipient.participant().as_str()
                != match phone {
                    NativeDtmfGatewayOrigin::Initial(o) => &o.source().target.participant_id,
                    NativeDtmfGatewayOrigin::Established(o) => &o.snapshot().participant_id,
                }
            || route.recipient.transport() != rvoip_core::Transport::Sip
            || route.intent.operation_id() != self.target.intent().operation_id
            || route.intent.digits() != self.observed.digits()
            || route.intent.duration_ms() != self.observed.duration_ms()
            || route.intent.method() != method
            || method
                != match self.observed.method() {
                    rvoip_uctp::v2::native_dtmf::NativeDtmfMethod::Info => DtmfMethod::Info,
                    rvoip_uctp::v2::native_dtmf::NativeDtmfMethod::Rfc4733 => DtmfMethod::Rfc4733,
                }
            || route.observation.kind() != DtmfSourceKind::Native
            || route.observation.observation_id() != Some(self.observed.observation_id())
            || route.observation.received_at() != self.observed.received_at()
        {
            return Err(DtmfWriteError::Refused);
        }
        Ok(())
    }
    async fn acquire(
        &self,
        route: &DtmfRouteScope<'_>,
        method: DtmfMethod,
    ) -> WriteResult<DtmfWriteLease> {
        self.scope(route, method)?;
        let runtime = self.runtime.upgrade().ok_or(DtmfWriteError::Refused)?;
        let controller = self.leg.controller()?;
        let connection = self.leg.connection()?;
        let principal = self.leg.principal()?;
        // Prepare engine metadata before acquiring any Store readers. The
        // exact source/route physical guards will surround each actual poll.
        let disclosure = self.observed.disclosure().ok_or(DtmfWriteError::Refused)?;
        let fence = loop {
            if self.leg.cancelled()
                || runtime.stop.is_cancelled()
                || self
                    .flight
                    .state
                    .lock()
                    .map_err(|_| DtmfWriteError::Refused)?
                    .0
            {
                return Err(DtmfWriteError::Refused);
            }
            match runtime.core.native_sip_dtmf_audience_fence(
                disclosure,
                &connection,
                &controller,
                &principal,
            ) {
                Ok(fence) => break fence,
                Err(
                    rvoip_core::orchestrator::participant_control::ParticipantControlError::Media(
                        rvoip_core::staged_streams::StreamControlError::NotReady,
                    ),
                ) => tokio::time::sleep(Duration::from_millis(5)).await,
                Err(_) => return Err(DtmfWriteError::Refused),
            }
        };
        loop {
            if self.leg.cancelled()
                || runtime.stop.is_cancelled()
                || self
                    .flight
                    .state
                    .lock()
                    .map_err(|_| DtmfWriteError::Refused)?
                    .0
            {
                return Err(DtmfWriteError::Refused);
            }
            let read = match runtime
                .media
                .store
                .try_native_dtmf_gateway_read(&self.target)
            {
                Ok(read) => Arc::new(read),
                Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(_) => return Err(DtmfWriteError::Refused),
            };
            if !self.leg.matches_member(read.phone_member()) {
                return Err(DtmfWriteError::Refused);
            }
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
                    .try_start_native_dtmf_gateway_write(&read, &self.attempt);
                if result.is_ok() {
                    flight.1 += 1;
                }
                result
            };
            match reserved {
                Ok(()) => (),
                Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => {
                    drop(read);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                Err(_) => return Err(DtmfWriteError::Refused),
            }
            // Only these exact named-signal readers survive into IO. No media
            // owner or RTP permission is constructed from a signal read.
            let current = Arc::downgrade(&read);
            let remaining = chrono::DateTime::from_timestamp(read.expires_at(), 0)
                .map(|at| at - chrono::Utc::now())
                .and_then(|v| v.to_std().ok())
                .unwrap_or(Duration::ZERO);
            let stop = runtime.stop.clone();
            let leg = self.leg.clone();
            return DtmfWriteLease::new(
                Owner {
                    read: Some(read),
                    flight: self.flight.clone(),
                },
                Instant::now() + remaining.min(Duration::from_millis(200)),
                move || {
                    current.upgrade().is_some_and(|r| r.current())
                        && !stop.is_cancelled()
                        && !leg.cancelled()
                },
            )
            .with_fence(fence)?
            .with_fence(Arc::new(Touch(self.touched.clone())));
        }
    }
}
#[async_trait::async_trait]
impl DtmfRoutePolicy for Policy {
    async fn acquire(
        &self,
        route: &DtmfRouteScope<'_>,
        packet: &DtmfWriteScope<'_>,
    ) -> WriteResult<DtmfWriteLease> {
        if packet.digits != self.observed.digits()
            || packet.duration_ms != self.observed.duration_ms()
        {
            return Err(DtmfWriteError::Refused);
        }
        Policy::acquire(self, route, DtmfMethod::Rfc4733).await
    }
    async fn acquire_info(
        &self,
        route: &DtmfRouteScope<'_>,
        packet: &DtmfInfoWriteScope<'_>,
    ) -> WriteResult<DtmfWriteLease> {
        if packet.digits != self.observed.digits()
            || packet.duration_ms != self.observed.duration_ms()
            || packet.wire_request.is_empty()
        {
            return Err(DtmfWriteError::Refused);
        }
        Policy::acquire(self, route, DtmfMethod::Info).await
    }
}
pub(super) async fn deliver(
    runtime: Arc<Runtime>,
    source: Arc<Connection>,
    leg: GatewayLeg,
    observed: NativeDtmfObservation,
    target: NativeDtmfGatewayTarget,
    expires_at: i64,
) -> bool {
    let policy = Arc::new(Policy {
        runtime: Arc::downgrade(&runtime),
        source,
        leg,
        observed: observed.clone(),
        target,
        attempt: format!("dtmfg_{}", uuid::Uuid::new_v4().simple()),
        flight: Arc::new(Flight::default()),
        touched: Arc::new(AtomicBool::new(false)),
    });
    let remaining = chrono::DateTime::from_timestamp(expires_at, 0)
        .map(|at| at - chrono::Utc::now())
        .and_then(|v| v.to_std().ok())
        .unwrap_or(Duration::ZERO);
    let deadline = Instant::now() + remaining;
    let cancel = runtime.stop.child_token();
    let result = match runtime
        .prepare_original_dtmf_route(
            &observed,
            ConnectionId::from_string(policy.target.origin().runtime_connection_id()),
            deadline,
        )
        .await
    {
        Ok(route) => {
            runtime
                .core
                .route_dtmf(&route, policy.clone(), cancel.clone(), deadline)
                .await
        }
        Err(e) => Err(e),
    };
    if let Err(error) = result {
        tracing::debug!(
            ?error,
            touched = policy.touched.load(Ordering::Acquire),
            "SIP DTMF delivery stopped"
        );
    }
    cancel.cancel();
    policy.flight.close();
    policy.flight.stopped().await;
    let disposition = match result {
        Ok(()) => SubmissionDisposition::LocalSubmitted,
        Err(DtmfWriteError::OutcomeUnknown) => SubmissionDisposition::OutcomeUnknown,
        Err(_) if policy.touched.load(Ordering::Acquire) => SubmissionDisposition::OutcomeUnknown,
        Err(_) => SubmissionDisposition::NotStarted,
    };
    loop {
        match runtime
            .media
            .store
            .try_record_native_dtmf_gateway_outcome(&policy.target, disposition)
        {
            Ok(()) => return true,
            Err(e) if matches!(e.code.as_str(), "authority-busy" | "storage-unavailable") => (),
            Err(e) => {
                tracing::debug!(code=%e.code,"SIP DTMF outcome awaits runtime recovery");
                return false;
            }
        }
        tokio::select! { _=runtime.stop.cancelled()=>return false,_=tokio::time::sleep(Duration::from_millis(10))=>{} }
    }
}
