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
    fn sip(&self, id: &str) -> Option<Arc<connect_store::SipAudienceRead>> {
        match self {
            Self::Consult(r) => r.sip(id),
            Self::Return(r) => r.sip(id),
            Self::Handoff(_) => None,
        }
    }
    fn sip_receive(
        &self,
        native: &str,
        sip: &str,
    ) -> Option<(&connect_store::SipAudienceRead, &NativeMediaRead)> {
        match self {
            Self::Consult(r) => r.sip_receive(native, sip),
            Self::Return(r) => r.sip_receive(native, sip),
            _ => None,
        }
    }
    fn sip_send(
        &self,
        sip: &str,
        native: &str,
    ) -> Option<(&connect_store::SipAudienceRead, &NativeMediaRead)> {
        match self {
            Self::Consult(r) => r.sip_send(sip, native),
            Self::Return(r) => r.sip_send(sip, native),
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
    pub(crate) fn sip_read(
        &self,
        connection: &rvoip_core::ids::ConnectionId,
        source: &str,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<Arc<connect_store::SipAudienceRead>> {
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
        {
            return Err(StreamControlError::Authority);
        }
        let private = plan.request().private_connections.contains(connection);
        let changed = plan.request().receivers.get(connection);
        let allowed = match &self.read {
            AudienceRead::Consult(r) if r.is_private_sip(source) => private && changed.is_some(),
            AudienceRead::Consult(r) if r.is_retained_sip(source) => {
                !private && changed.is_some_and(|s| s.is_empty())
            }
            AudienceRead::Return(r) if r.is_primary_sip(source) => private && changed.is_none(),
            AudienceRead::Consult(_) | AudienceRead::Return(_) => !private && changed.is_none(),
            AudienceRead::Handoff(_) => false,
        };
        if !allowed {
            return Err(StreamControlError::Authority);
        }
        self.read.sip(source).ok_or(StreamControlError::Authority)
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
        if let Some(leg) = i.policy.sip_audience_target(&source.connection)? {
            let (phone, target) = i
                .read
                .sip_receive(self.target.origin.connection_id(), &leg.source_id()?)
                .ok_or(StreamControlError::Authority)?;
            if !i.read.current()
                || !phone.current()
                || !target.current()
                || !target.permits_subscriptions()
            {
                return Err(StreamControlError::Authority);
            }
            leg.source_matches(source, phone)?;
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
        if let Some(leg) = i.policy.sip_audience_target(&source.connection)? {
            let (phone, target) = i
                .read
                .sip_receive(self.target.origin.connection_id(), &leg.source_id()?)
                .ok_or(StreamControlError::Authority)?;
            if !i.read.current()
                || !phone.current()
                || !target.current()
                || !target.permits_subscriptions()
            {
                return Err(StreamControlError::Authority);
            }
            leg.participant_source_matches(source, phone)?;
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
    fn sip_audience_target(
        &self,
        id: &rvoip_core::ids::ConnectionId,
    ) -> ControlResult<Option<SipAudienceLeg>> {
        if let Some(leg) = self
            .initial_targets
            .lock()
            .map_err(denied)?
            .get(id)
            .and_then(std::sync::Weak::upgrade)
        {
            return Ok(Some(SipAudienceLeg::Initial(leg)));
        }
        Ok(self
            .telephone_target(id)?
            .filter(|leg| leg.established_origin().is_some())
            .map(SipAudienceLeg::Established))
    }
    pub(crate) fn observe_sip_audio_audience_commit(
        &self,
        connection: &rvoip_core::ids::ConnectionId,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
        invocation: &AudienceInvocation,
    ) -> ControlResult<Option<()>> {
        let Some(leg) = self.sip_audience_target(connection)? else {
            return Ok(None);
        };
        let intent = MediaControlIntent::AudioAudience {
            plan: &invocation.plan,
        };
        let read = invocation.sip_read(connection, &leg.source_id()?, &intent)?;
        let previous = invocation
            .plan
            .expected_receiver_revisions()
            .get(connection)
            .copied()
            .ok_or(StreamControlError::Authority)?;
        match (&leg, read.as_ref()) {
            (SipAudienceLeg::Initial(leg), connect_store::SipAudienceRead::Initial(read)) => {
                leg.observe_audio_audience_commit(read, commit, previous)?
            }
            (
                SipAudienceLeg::Established(leg),
                connect_store::SipAudienceRead::Established(read),
            ) => {
                leg.established_owner(read)?;
                let controller = leg.controller()?;
                let (before, after) = commit.revisions_for(&controller)?;
                if before != previous || before.checked_add(1) != Some(after) {
                    return Err(StreamControlError::Authority);
                }
                leg.observe_audience(commit)?;
            }
            _ => return Err(StreamControlError::Authority),
        }
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
    pub(crate) fn scene_sip_origins(
        &self,
        session: &str,
    ) -> ControlResult<Vec<connect_store::SipAudienceOrigin>> {
        let mut origins = self
            .initial_targets
            .lock()
            .map_err(denied)?
            .values()
            .filter_map(std::sync::Weak::upgrade)
            .filter(|leg| !leg.cancel.is_cancelled())
            .filter_map(|leg| leg.media.get().cloned())
            .filter(|source| source.source().request.session_id == session)
            .map(connect_store::SipAudienceOrigin::Initial)
            .collect::<Vec<_>>();
        origins.extend(
            self.telephone_targets
                .lock()
                .map_err(denied)?
                .values()
                .filter_map(std::sync::Weak::upgrade)
                .filter_map(|leg| leg.established_origin())
                .filter(|source| source.snapshot().session_id == session)
                .map(connect_store::SipAudienceOrigin::Established),
        );
        Ok(origins)
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

#[derive(Clone)]
enum SipAudienceLeg {
    Initial(Arc<crate::initial_telephone::policy::Leg>),
    Established(Arc<crate::telephone_replacement::media::Policy>),
}
impl SipAudienceLeg {
    fn source_id(&self) -> ControlResult<String> {
        match self {
            Self::Initial(leg) => Ok(leg.signal.origin().connection_id().into()),
            Self::Established(leg) => Ok(leg
                .established_origin()
                .ok_or(StreamControlError::NotReady)?
                .snapshot()
                .connection_id
                .clone()),
        }
    }
    fn owner(&self, read: &connect_store::SipAudienceRead) -> ControlResult<MediaOwner> {
        match (self, read) {
            (Self::Initial(leg), connect_store::SipAudienceRead::Initial(read)) => leg.owner(read),
            (Self::Established(leg), connect_store::SipAudienceRead::Established(read)) => {
                leg.established_owner(read)
            }
            _ => Err(StreamControlError::Authority),
        }
    }
    fn source_matches(
        &self,
        source: &MediaSource,
        read: &connect_store::SipAudienceRead,
    ) -> ControlResult<()> {
        self.owner(read)?;
        match (self, read) {
            (Self::Initial(leg), connect_store::SipAudienceRead::Initial(read)) => {
                leg.physical_source_matches(source, read)
            }
            (Self::Established(leg), connect_store::SipAudienceRead::Established(_)) => {
                leg.source_matches(source)
            }
            _ => Err(StreamControlError::Authority),
        }
    }
    fn participant_source_matches(
        &self,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
        read: &connect_store::SipAudienceRead,
    ) -> ControlResult<()> {
        match (self, read) {
            (Self::Initial(leg), connect_store::SipAudienceRead::Initial(read)) => {
                leg.validate_participant_source(source, read)
            }
            (Self::Established(leg), connect_store::SipAudienceRead::Established(read)) => {
                leg.participant_source_matches(source, read)
            }
            _ => Err(StreamControlError::Authority),
        }
    }
}
struct HeldSipAudience {
    policy: Arc<Policy>,
    cancel: tokio_util::sync::CancellationToken,
    leg: SipAudienceLeg,
    read: Arc<connect_store::SipAudienceRead>,
    owner: MediaOwner,
    invocation: AudienceRead,
}
impl MediaAuthorityLease for HeldSipAudience {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        if !self.invocation.current() || self.cancel.is_cancelled() {
            return Err(StreamControlError::Authority);
        }
        self.leg.owner(&self.read).map(Box::new)
    }
    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        let (native, read, owner) = self.receiving_owner(&source.connection)?;
        source_matches(source, &native, read.target())?;
        let source_owner = native.owner(read.target(), &native.principal, false)?;
        native
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(source_owner)?;
        Ok(Box::new(owner))
    }
    fn control_participant_receive(
        &self,
        source: &rvoip_core::staged_streams::MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        let (native, read, owner) = self.receiving_owner(&source.connection)?;
        let view = read.target();
        if source.medium != Medium::Audio
            || source.conversation.as_str() != native.origin.conversation_id()
            || source.session.as_str() != native.origin.session_id()
            || source.connection.as_str() != native.origin.runtime_connection_id()
            || source.participant.as_str() != view.connection.participant_id
            || source.membership_epoch != view.connection.membership_epoch
            || source.identity.as_str() != view.session.actor.identity_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &source.principal,
                &native.principal,
            )
        {
            return Err(StreamControlError::Authority);
        }
        let source_owner = native.owner(view, &source.principal, false)?;
        native
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(source_owner)?;
        Ok(Box::new(owner))
    }
}
impl HeldSipAudience {
    fn receiving_owner(
        &self,
        source: &rvoip_core::ids::ConnectionId,
    ) -> ControlResult<(Arc<Connection>, &NativeMediaRead, MediaOwner)> {
        if !self.invocation.current() || self.cancel.is_cancelled() {
            return Err(StreamControlError::Authority);
        }
        let native = self.policy.entry(source)?;
        let (phone, read) = self
            .invocation
            .sip_send(&self.leg.source_id()?, native.origin.connection_id())
            .ok_or(StreamControlError::Authority)?;
        if !std::ptr::eq(phone, self.read.as_ref()) || !phone.may_receive() || !read.current() {
            return Err(StreamControlError::Authority);
        }
        let mut owner = self.leg.owner(phone)?;
        owner.may_route_source = true;
        Ok((native, read, owner))
    }
}
impl AudienceInvocation {
    pub(crate) fn telephone_control(
        &self,
        policy: &crate::telephone_replacement::media::Policy,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        let Some(SipAudienceLeg::Established(leg)) =
            self.policy.sip_audience_target(scope.connection)?
        else {
            return Err(StreamControlError::Authority);
        };
        if !std::ptr::eq(policy, Arc::as_ptr(&leg)) {
            return Err(StreamControlError::Authority);
        }
        self.sip_control(SipAudienceLeg::Established(leg), scope, intent)
    }
    pub(crate) fn initial_control(
        &self,
        policy: &crate::initial_telephone::policy::Leg,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        let Some(SipAudienceLeg::Initial(leg)) =
            self.policy.sip_audience_target(scope.connection)?
        else {
            return Err(StreamControlError::Authority);
        };
        if !std::ptr::eq(policy, Arc::as_ptr(&leg)) {
            return Err(StreamControlError::Authority);
        }
        self.sip_control(SipAudienceLeg::Initial(leg), scope, intent)
    }
    fn sip_control(
        &self,
        leg: SipAudienceLeg,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        let read = self.sip_read(scope.connection, &leg.source_id()?, intent)?;
        if let (SipAudienceLeg::Initial(leg), connect_store::SipAudienceRead::Initial(read)) =
            (&leg, read.as_ref())
        {
            if read.source() != leg.signal.origin().source() {
                return Err(StreamControlError::Authority);
            }
        }
        let owner = leg.owner(&read)?;
        let remaining = (self.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        MediaControl::new(
            Box::new(HeldSipAudience {
                policy: self.policy.clone(),
                cancel: self.cancel.clone(),
                leg,
                read,
                owner,
                invocation: self.read.clone(),
            }),
            self.cancel.clone(),
            tokio::time::Instant::now() + Duration::from_secs(remaining),
        )
    }
}
