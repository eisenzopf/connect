//! Exact scene-issued private audience approval under one bounded current read.
use super::*;
use connect_store::{NativeConsultationRead, NativeHandoffRead, NativeMediaRead, NativeReturnRead};

#[derive(Clone)]
pub(crate) enum AudienceRead {
    Consult(Arc<NativeConsultationRead>),
    Return(Arc<NativeReturnRead>),
    Handoff(Arc<NativeHandoffRead>),
}
impl AudienceRead {
    fn current(&self) -> bool {
        match self {
            Self::Consult(r) => r.current(),
            Self::Return(r) => r.current(),
            Self::Handoff(r) => r.current(),
        }
    }
    pub fn expires_at(&self) -> i64 {
        match self {
            Self::Consult(r) => r.expires_at(),
            Self::Return(r) => r.expires_at(),
            Self::Handoff(r) => r.expires_at(),
        }
    }
    fn initial_sip(&self, id: &str) -> Option<Arc<connect_store::InitialTelephoneMediaRead>> {
        match self {
            Self::Consult(r) => r.initial_sip(id),
            Self::Return(r) => r.initial_sip(id),
            Self::Handoff(_) => None,
        }
    }
    fn initial_sip_receive(
        &self,
        native: &str,
        sip: &str,
    ) -> Option<(&connect_store::InitialTelephoneMediaRead, &NativeMediaRead)> {
        match self {
            Self::Return(r) => r.initial_sip_receive(native, sip),
            _ => None,
        }
    }
    fn media(&self, target: &str, source: Option<&str>) -> Option<&NativeMediaRead> {
        match self {
            Self::Consult(r) => r.media(target, source),
            Self::Return(r) => r.media(target, source),
            Self::Handoff(r) => r.media(target, source),
        }
    }
}
use rvoip_core::staged_streams::audience::AudioAudiencePlan;

pub(crate) struct AudienceInvocation {
    pub policy: Arc<Policy>,
    pub plan: AudioAudiencePlan,
    pub read: AudienceRead,
    pub cancel: tokio_util::sync::CancellationToken,
}
impl AudienceInvocation {
    pub(crate) fn initial_sip_read(
        &self,
        connection: &rvoip_core::ids::ConnectionId,
        source: &str,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<Arc<connect_store::InitialTelephoneMediaRead>> {
        let MediaControlIntent::AudioAudience { plan } = intent else {
            return Err(StreamControlError::Authority);
        };
        if !self.read.current()
            || self.cancel.is_cancelled()
            || !self.plan.same_intent(plan)
            || !matches!(
                self.read,
                AudienceRead::Consult(_) | AudienceRead::Return(_)
            )
            || plan.request().private_connections.contains(connection)
            || plan.request().receivers.contains_key(connection)
        {
            return Err(StreamControlError::Authority);
        }
        self.read
            .initial_sip(source)
            .ok_or(StreamControlError::Authority)
    }
    pub(crate) fn expires_at(&self) -> i64 {
        self.read.expires_at()
    }
    fn owner(&self, target: &Connection, route: bool) -> ControlResult<MediaOwner> {
        if !self.read.current() || self.cancel.is_cancelled() {
            return Err(StreamControlError::Authority);
        }
        let read = self
            .read
            .media(target.origin.connection_id(), None)
            .ok_or(StreamControlError::Authority)?;
        let owner = target.owner(read.target(), &target.principal, route)?;
        target
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
}
struct HeldAudience {
    invocation: Arc<AudienceInvocation>,
    target: Arc<Connection>,
    owner: MediaOwner,
}
impl MediaAuthorityLease for HeldAudience {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(self.invocation.owner(&self.target, false)?))
    }
    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        if let Some(leg) = i.policy.initial_audience_target(&source.connection)? {
            let (phone, target) = i
                .read
                .initial_sip_receive(
                    self.target.origin.connection_id(),
                    leg.signal.origin().connection_id(),
                )
                .ok_or(StreamControlError::Authority)?;
            if !i.read.current()
                || !phone.current()
                || !target.current()
                || !target.permits_subscriptions()
            {
                return Err(StreamControlError::Authority);
            }
            leg.physical_source_matches(source, phone)?;
            return Ok(Box::new(i.owner(&self.target, true)?));
        }
        let source_entry = i.policy.entry(&source.connection)?;
        let read = i
            .read
            .media(
                self.target.origin.connection_id(),
                Some(source_entry.origin.connection_id()),
            )
            .ok_or(StreamControlError::Authority)?;
        if !read.permits_subscriptions() {
            return Err(StreamControlError::Authority);
        }
        source_matches(
            source,
            &source_entry,
            read.source().ok_or(StreamControlError::Authority)?,
        )?;
        Ok(Box::new(i.owner(&self.target, true)?))
    }
    fn control_participant_receive(
        &self,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        if let Some(leg) = i.policy.initial_audience_target(&source.connection)? {
            let (phone, target) = i
                .read
                .initial_sip_receive(
                    self.target.origin.connection_id(),
                    leg.signal.origin().connection_id(),
                )
                .ok_or(StreamControlError::Authority)?;
            if !i.read.current()
                || !phone.current()
                || !target.current()
                || !target.permits_subscriptions()
            {
                return Err(StreamControlError::Authority);
            }
            leg.validate_participant_source(source, phone)?;
            return Ok(Box::new(i.owner(&self.target, true)?));
        }
        let source_entry = i.policy.entry(&source.connection)?;
        let read = i
            .read
            .media(
                self.target.origin.connection_id(),
                Some(source_entry.origin.connection_id()),
            )
            .ok_or(StreamControlError::Authority)?;
        if !i.read.current() || !read.permits_subscriptions() {
            return Err(StreamControlError::Authority);
        }
        Ok(Box::new(i.policy.participant_receive_owner(
            &self.target,
            read.target(),
            &self.target.principal,
            source,
            &source_entry,
            read.source().ok_or(StreamControlError::Authority)?,
        )?))
    }
}
impl Policy {
    fn initial_audience_target(
        &self,
        id: &rvoip_core::ids::ConnectionId,
    ) -> ControlResult<Option<Arc<crate::initial_telephone::policy::Leg>>> {
        Ok(self
            .initial_targets
            .lock()
            .map_err(denied)?
            .get(id)
            .and_then(std::sync::Weak::upgrade))
    }
    pub(crate) fn observe_initial_audio_audience_commit(
        &self,
        connection: &rvoip_core::ids::ConnectionId,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
        invocation: &AudienceInvocation,
    ) -> ControlResult<Option<()>> {
        // Host metadata boundary, outside Core and without retaining the map
        // lock through controller validation. A busy packet lookup is no
        // reason to discard an already returned physical commit receipt.
        let leg = self.initial_audience_target(connection)?;
        let Some(leg) = leg else {
            return Ok(None);
        };
        let intent = MediaControlIntent::AudioAudience {
            plan: &invocation.plan,
        };
        let read = invocation.initial_sip_read(
            connection,
            leg.signal.origin().connection_id(),
            &intent,
        )?;
        let previous = invocation
            .plan
            .expected_receiver_revisions()
            .get(connection)
            .copied()
            .ok_or(StreamControlError::Authority)?;
        leg.observe_audio_audience_commit(&read, commit, previous)?;
        Ok(Some(()))
    }
    pub(super) fn consultation_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        i: &AudienceInvocation,
    ) -> ControlResult<MediaControl> {
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy)) || !i.read.current() {
            return Err(StreamControlError::Authority);
        }
        let plan = match (intent, &i.read) {
            (MediaControlIntent::Handoff { plan }, AudienceRead::Handoff(_)) => *plan,
            (
                MediaControlIntent::AudioAudience { plan },
                AudienceRead::Consult(_) | AudienceRead::Return(_),
            ) => *plan,
            _ => return Err(StreamControlError::Authority),
        };
        if !i.plan.same_intent(plan) {
            return Err(StreamControlError::Authority);
        }
        let target = self.entry(scope.connection)?;
        self.scope(scope, &target)?;
        let owner = i.owner(&target, false)?;
        let remaining = (i.read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        let invocation = Arc::new(AudienceInvocation {
            policy: i.policy.clone(),
            plan: i.plan.clone(),
            read: i.read.clone(),
            cancel: i.cancel.clone(),
        });
        MediaControl::new(
            Box::new(HeldAudience {
                invocation,
                target,
                owner,
            }),
            i.cancel.clone(),
            tokio::time::Instant::now() + Duration::from_secs(remaining),
        )
    }
    pub(crate) fn scene_initial_sip_origins(
        &self,
        session: &str,
    ) -> ControlResult<Vec<Arc<connect_store::InitialTelephoneMediaOrigin>>> {
        Ok(self
            .initial_targets
            .lock()
            .map_err(denied)?
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .filter(|leg| !leg.cancel.is_cancelled())
            .filter_map(|leg| leg.media.get().cloned())
            .filter(|source| source.source().request.session_id == session)
            .collect())
    }
    pub(crate) fn scene_origins(
        &self,
        session: &str,
    ) -> ControlResult<Vec<Arc<NativeMediaOrigin>>> {
        // Host continuations call this before entering Core or acquiring Store
        // readers. Copy the bounded metadata under the short registry lock;
        // packet lookup contention must not masquerade as revoked authority.
        Ok(self
            .entries
            .lock()
            .map_err(denied)?
            .values()
            .filter(|v| v.origin.session_id() == session)
            .map(|v| v.origin.clone())
            .collect())
    }
}
