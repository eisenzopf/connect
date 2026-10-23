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
