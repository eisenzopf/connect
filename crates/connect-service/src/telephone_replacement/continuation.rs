//! Exact current grants for owned local replacement continuation, never wire claims.
use super::*;
use crate::canonical_replacement::PreparedReplacement;
use connect_store::TelephoneReplacementSceneRead;
use rvoip_core::{
    ids::ConnectionId,
    orchestrator::participant_control::{
        ControlResult as RoleResult, DeviceReplacementEndpointEvent, DeviceReplacementSignalScope,
        ManagedAudioDeviceReplacementPlan, ParticipantControlError,
        ParticipantReplacementAuthority, ParticipantReplacementControl,
        ParticipantReplacementLease, ParticipantReplacementScope, ReplacementPhase,
    },
    staged_streams::audience::AudioAudiencePlan,
    staged_streams::*,
    stream_lifecycle::Medium,
};
#[derive(Clone)]
pub(crate) enum Phase {
    Attach,
    Activate(AudioAudiencePlan),
    Signal(DeviceReplacementEndpointEvent),
    Readiness,
    Commit(ManagedAudioDeviceReplacementPlan),
}
#[derive(Clone)]
pub(crate) struct Invocation {
    pub runtime: Arc<Runtime>,
    pub started: Arc<media::Started>,
    pub read: Arc<TelephoneReplacementSceneRead>,
    pub root: Arc<PreparedReplacement>,
    pub phase: Phase,
    pub deadline: tokio::time::Instant,
}
impl Invocation {
    fn check(&self) -> ControlResult<()> {
        if !self.read.current()
            || self.root.handle.is_committed()
            || self.root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn owner(&self, id: &ConnectionId) -> ControlResult<MediaOwner> {
        self.check()?;
        if id == &self.root.handle.intent().replacement {
            return self.started.policy.owner(self.read.media());
        }
        let entry = self.runtime.media.entry(id)?;
        let read = self
            .read
            .existing(id.as_str(), None)
            .ok_or(StreamControlError::Authority)?;
        let owner = entry.owner(read.target(), &entry.principal, false)?;
        entry
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
    pub(crate) fn media_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        self.check()?;
        let valid = match (&self.phase, intent) {
            (Phase::Attach, MediaControlIntent::PreparePrivateAudio { expected, .. }) => {
                scope.connection == &self.root.handle.intent().replacement && *expected == 2
            }
            (Phase::Activate(expected), MediaControlIntent::AudioAudience { plan }) => {
                expected.same_intent(plan)
            }
            (Phase::Commit(expected), MediaControlIntent::DeviceReplacement { plan }) => {
                expected.media().audience().same_intent(plan)
            }
            _ => false,
        };
        if !valid {
            tracing::debug!(target:"connect_service::canonical_replacement",candidate=scope.connection==&self.root.handle.intent().replacement,"Replacement continuation refused control intent");
            return Err(StreamControlError::Authority);
        }
        let owner = self.owner(scope.connection)?;
        let principal = if scope.connection == &self.root.handle.intent().replacement {
            self.started.policy.principal()?
        } else {
            self.runtime
                .media
                .entry(scope.connection)?
                .principal
                .clone()
        };
        if scope.conversation != owner.member.conversation()
            || scope.session != owner.presence.session()
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &principal,
            )
        {
            return Err(StreamControlError::Authority);
        }
        MediaControl::new(
            Box::new(Held {
                invocation: self.clone(),
                target: scope.connection.clone(),
                owner,
            }),
            self.root.handle.cancellation(),
            self.deadline,
        )
    }
    pub(crate) fn role_control(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> RoleResult<ParticipantReplacementControl> {
        self.current(scope)?;
        ParticipantReplacementControl::new(
            Box::new(self.clone()),
            self.root.handle.cancellation(),
            self.deadline,
        )
    }
    pub(crate) fn signal(
        &self,
        scope: &DeviceReplacementSignalScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.check()?;
        let principal = self.started.policy.principal()?;
        if !matches!(&self.phase,Phase::Signal(event) if event==scope.event)
            || !matches!(scope.source, rvoip_core::orchestrator::participant_control::DeviceReplacementSignalSource::Adapter(_))
            || scope.pending.intent() != self.root.handle.intent()
            || scope.pending.binding().connection() != &self.root.handle.intent().replacement
            || scope.pending.generation() != self.started.policy.pending.generation()
            || scope.conversation.as_str() != self.read.media().source().context().conversation_id
            || scope.session.as_str() != self.read.media().source().context().session_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &principal,
            )
            || scope.target_principal.is_some_and(|p| {
                !rvoip_core::staged_streams::reliable_data::same_credential(
                    p,
                    &principal,
                )
            })
        {
            return Err(StreamControlError::Authority);
        }
        let id = self.root.handle.intent().replacement.clone();
        Ok(Box::new(Held {
            owner: self.owner(&id)?,
            target: id,
            invocation: self.clone(),
        }))
    }
}
impl ParticipantReplacementLease for Invocation {
    fn current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> RoleResult<ParticipantReplacementAuthority> {
        let deny = || ParticipantControlError::Authority;
        self.check().map_err(|_| deny())?;
        if scope.intent != self.root.handle.intent()
            || !match (&self.phase, scope.plan) {
                (Phase::Attach | Phase::Activate(_), None) => true,
                (Phase::Commit(expected), Some(plan)) => {
                    expected.media().audience().same_intent(plan.audience())
                }
                _ => false,
            }
            || !matches!(
                (&self.phase, scope.phase),
                (Phase::Attach, ReplacementPhase::Attach)
                    | (Phase::Activate(_), ReplacementPhase::Activate)
                    | (Phase::Commit(_), ReplacementPhase::Commit)
            )
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.root.operator,
            )
            || scope.conversation.as_str() != self.read.media().source().context().conversation_id
            || scope.session.as_str() != self.read.media().source().context().session_id
        {
            return Err(deny());
        }
        let owner = self
            .owner(&self.root.handle.intent().replacement)
            .map_err(|_| deny())?;
        let actor = crate::canonical_runtime::current_member(
            &self.root.operator,
            self.read.media().source().caller(),
        )
        .map_err(|_| deny())?;
        Ok(ParticipantReplacementAuthority {
            actor,
            target: owner.member,
            endpoint: owner.endpoint,
            may_send_audio: self.read.media().source().context().role != "observer"
                && owner.send_media.contains(&Medium::Audio),
            may_receive_audio: owner.receive_media.contains(&Medium::Audio),
        })
    }
}
struct Held {
    invocation: Invocation,
    target: ConnectionId,
    owner: MediaOwner,
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        self.invocation.owner(&self.target).map(Box::new)
    }
    fn control_receive(&self, source: &MediaSource) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        let phone = &i.root.handle.intent().replacement;
        if &self.target == phone || &source.connection == phone {
            if !i
                .read
                .permits_pair(self.target.as_str(), source.connection.as_str())
            {
                return Err(StreamControlError::Authority);
            }
            let original = i.owner(&source.connection)?;
            if source.conversation != *original.member.conversation()
                || source.session != *original.presence.session()
                || source.participant != *original.member.participant()
                || source.membership_epoch != original.member.epoch()
                || source.connection_generation != original.connection.generation()
            {
                return Err(StreamControlError::Authority);
            }
            if &source.connection == phone {
                i.started.policy.source_matches(source)?;
            } else {
                let entry = i.runtime.media.entry(&source.connection)?;
                let read = i
                    .read
                    .existing(source.connection.as_str(), None)
                    .ok_or(StreamControlError::Authority)?;
                crate::canonical_media::source_matches(source, &entry, read.target())?;
            }
        } else {
            let read = i
                .read
                .existing(self.target.as_str(), Some(source.connection.as_str()))
                .ok_or(StreamControlError::Authority)?;
            if !read.permits_subscriptions() {
                return Err(StreamControlError::Authority);
            }
            let entry = i.runtime.media.entry(&source.connection)?;
            crate::canonical_media::source_matches(
                source,
                &entry,
                read.source().ok_or(StreamControlError::Authority)?,
            )?;
        }
        let mut owner = i.owner(&self.target)?;
        owner.may_route_source = true;
        Ok(Box::new(owner))
    }
    fn control_participant_receive(
        &self,
        source: &MediaParticipantSource,
    ) -> ControlResult<Box<MediaOwner>> {
        let i = &self.invocation;
        let phone = &i.root.handle.intent().replacement;
        let allowed = if &self.target == phone || &source.connection == phone {
            i.read
                .permits_pair(self.target.as_str(), source.connection.as_str())
        } else {
            i.read
                .existing(self.target.as_str(), Some(source.connection.as_str()))
                .is_some_and(|r| r.permits_subscriptions())
        };
        if !allowed {
            return Err(StreamControlError::Authority);
        }
        let original = i.owner(&source.connection)?;
        if source.conversation != *original.member.conversation()
            || source.session != *original.presence.session()
            || source.participant != *original.member.participant()
            || source.identity != *original.member.identity()
            || source.membership_epoch != original.member.epoch()
            || source.medium != Medium::Audio
        {
            return Err(StreamControlError::Authority);
        }
        if &source.connection == phone {
            if !rvoip_core::staged_streams::reliable_data::same_credential(
                &source.principal,
                &i.started.policy.principal()?,
            ) {
                return Err(StreamControlError::Authority);
            }
        } else {
            original
                .member
                .revalidate_principal(&source.principal, chrono::Utc::now())
                .map_err(|_| StreamControlError::Authority)?;
        }
        let mut owner = i.owner(&self.target)?;
        owner.may_route_source = true;
        Ok(Box::new(owner))
    }
}
