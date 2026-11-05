//! Exact current grants for owned local replacement continuation, never wire claims.
use super::media::TargetPolicy;
use super::*;
use connect_store::NativeReplacementSceneRead;
use rvoip_core::{
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
    Preroll(
        rvoip_core::staged_streams::device_replacement::ReplacementVideoRequest,
        u64,
    ),
    Signal(DeviceReplacementEndpointEvent),
    Commit(ManagedAudioDeviceReplacementPlan),
}
#[derive(Clone)]
pub(crate) struct Invocation {
    pub entries: std::collections::BTreeMap<ConnectionId, Arc<crate::canonical_media::Connection>>,
    pub policy: Arc<TargetPolicy>,
    pub source_phone: Option<Arc<crate::telephone_replacement::media::Policy>>,
    pub read: Arc<NativeReplacementSceneRead>,
    pub root: Arc<PreparedReplacement>,
    pub phase: Phase,
    pub deadline: tokio::time::Instant,
}
impl Invocation {
    fn refused(
        &self,
        stage: &'static str,
        id: &ConnectionId,
        error: StreamControlError,
    ) -> StreamControlError {
        let phase = match &self.phase {
            Phase::Attach => "attach",
            Phase::Activate(_) => "activate",
            Phase::Preroll(_, _) => "preroll",
            Phase::Signal(_) => "signal",
            Phase::Commit(_) => "commit",
        };
        tracing::debug!(target: "connect_service::canonical_replacement",
            operation = %self.root.origin.request().operation_id,
            connection = %id, phase, stage, ?error,
            "Replacement held authority refused");
        error
    }
    pub(crate) fn controls_telephone(
        &self,
        source: &crate::telephone_replacement::media::Policy,
    ) -> bool {
        self.check().is_ok()
            && self
                .source_phone
                .as_ref()
                .is_some_and(|p| std::ptr::eq(Arc::as_ptr(p), source))
    }
    fn check(&self) -> ControlResult<()> {
        if !self.read.current()
            || self.root.handle.is_committed()
            || self.root.handle.cancellation().is_cancelled()
            || tokio::time::Instant::now() >= self.deadline
            || !self
                .policy
                .root
                .upgrade()
                .is_some_and(|r| Arc::ptr_eq(&r, &self.root))
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn entry(&self, id: &ConnectionId) -> ControlResult<Arc<crate::canonical_media::Connection>> {
        self.entries
            .get(id)
            .cloned()
            .ok_or(StreamControlError::Authority)
    }
    fn candidate_owner(&self, id: &ConnectionId) -> ControlResult<MediaOwner> {
        let context = self.read.media().source().context();
        let source = ConnectionId::from_string(context.source_runtime_connection_id.clone());
        let retained = ConnectionId::from_string(context.retained_runtime_connection_id.clone());
        if id == &source || id == &retained || source == retained {
            return Err(StreamControlError::Authority);
        }
        // Both original owners must remain live under this exact owned
        // scene read. Never consult packet registries inside a Core fence.
        let old = self
            .owner(&source)
            .map_err(|error| self.refused("candidate-source-owner", &source, error))?;
        self.owner(&retained)
            .map_err(|error| self.refused("candidate-retained-owner", &retained, error))?;
        if old.member.participant().as_str() != context.participant_id
            || old.member.epoch() != context.membership_epoch
        {
            return Err(StreamControlError::Authority);
        }
        self.policy
            .current_owner_with_source(self.read.media(), old)
            .map_err(|error| self.refused("candidate-owner", id, error))
    }
    pub(super) fn owner(&self, id: &ConnectionId) -> ControlResult<MediaOwner> {
        self.check()?;
        if id == self.policy.pending.binding().connection() {
            #[cfg(feature = "test-support")]
            if matches!(self.phase, Phase::Commit(_)) {
                if let Some(result) = self.policy.base.probe_held_registry_for_test(
                    &self.root.origin.request().operation_id,
                    || {
                        let refused = matches!(
                            self.policy.current_owner(self.read.media()),
                            Err(StreamControlError::Authority)
                        );
                        let mut expired = self.clone();
                        expired.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
                        let expired_refused =
                            matches!(expired.owner(id), Err(StreamControlError::Authority));
                        (refused && expired_refused, self.candidate_owner(id))
                    },
                ) {
                    return result;
                }
            }
            return self.candidate_owner(id);
        }
        if id.as_str()
            == self
                .read
                .media()
                .source()
                .context()
                .source_runtime_connection_id
        {
            if let Some(phone) = &self.source_phone {
                return phone.established_owner(
                    self.read
                        .media()
                        .source()
                        .source()
                        .telephone()
                        .ok_or(StreamControlError::Authority)?,
                );
            }
        }
        let entry = self
            .entry(id)
            .map_err(|error| self.refused("existing-registry", id, error))?;
        let read = self
            .read
            .existing(id.as_str(), None)
            .ok_or(StreamControlError::Authority)?;
        let owner = entry
            .owner(read.target(), &entry.principal, false)
            .map_err(|error| self.refused("existing-binding", id, error))?;
        entry
            .activation()?
            .ok_or_else(|| {
                self.refused(
                    "existing-activation-missing",
                    id,
                    StreamControlError::NotReady,
                )
            })?
            .current(owner)
            .map_err(|error| self.refused("existing-activation-current", id, error))
    }
    pub(crate) fn media_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        self.check()?;
        let valid = match (&self.phase, intent) {
            (Phase::Attach, MediaControlIntent::PreparePrivateAudio { expected, .. }) => {
                scope.connection == self.policy.pending.binding().connection() && *expected == 2
            }
            (Phase::Activate(expected), MediaControlIntent::AudioAudience { plan }) => {
                expected.same_intent(plan)
            }
            (
                Phase::Preroll(expected, original_revision),
                MediaControlIntent::PrepareReplacementVideo {
                    source,
                    expected: revision,
                    request,
                },
            ) => {
                scope.connection == self.policy.pending.binding().connection()
                    && source.as_str()
                        == self
                            .read
                            .media()
                            .source()
                            .context()
                            .source_runtime_connection_id
                    && revision == original_revision
                    && *request == expected
            }
            (Phase::Commit(expected), MediaControlIntent::DeviceReplacement { plan }) => {
                expected.media().audience().same_intent(plan)
            }
            _ => false,
        };
        if !valid {
            tracing::debug!(target:"connect_service::canonical_replacement",candidate=scope.connection==self.policy.pending.binding().connection(),"Replacement continuation refused control intent");
            return Err(StreamControlError::Authority);
        }
        let owner = self.owner(scope.connection)?;
        let principal = if scope.connection == self.policy.pending.binding().connection() {
            self.policy.principal.clone()
        } else if scope.connection.as_str()
            == self
                .read
                .media()
                .source()
                .context()
                .source_runtime_connection_id
            && self.source_phone.is_some()
        {
            self.source_phone
                .as_ref()
                .ok_or(StreamControlError::Authority)?
                .principal()?
        } else {
            self.entry(scope.connection)?.principal.clone()
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
        if !matches!(&self.phase,Phase::Signal(event) if event==scope.event)
            || scope.pending.intent() != self.policy.pending.intent()
            || scope.pending.binding().connection() != self.policy.pending.binding().connection()
            || scope.pending.generation() != self.policy.pending.generation()
            || scope.conversation.as_str() != self.root.origin.request().conversation_id
            || scope.session.as_str() != self.root.origin.request().session_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.policy.principal,
            )
            || scope.target_principal.is_none_or(|p| {
                !rvoip_core::staged_streams::reliable_data::same_credential(
                    p,
                    &self.policy.principal,
                )
            })
        {
            return Err(StreamControlError::Authority);
        }
        let id = self.policy.pending.binding().connection().clone();
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
                (Phase::Commit(expected), Some(plan)) => {
                    expected.media().audience().same_intent(plan.audience())
                }
                (Phase::Attach | Phase::Activate(_) | Phase::Preroll(_, _), None) => true,
                _ => false,
            }
            || !matches!(
                (&self.phase, scope.phase),
                (Phase::Preroll(_, _), ReplacementPhase::Preroll)
                    | (Phase::Attach, ReplacementPhase::Attach)
                    | (Phase::Activate(_), ReplacementPhase::Activate)
                    | (Phase::Commit(_), ReplacementPhase::Commit)
            )
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.root.operator,
            )
            || scope.conversation.as_str() != self.root.origin.request().conversation_id
            || scope.session.as_str() != self.root.origin.request().session_id
        {
            return Err(deny());
        }
        let owner = self
            .owner(self.policy.pending.binding().connection())
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
        if !i
            .read
            .permits_receive(self.target.as_str(), source.connection.as_str())
        {
            return Err(StreamControlError::Authority);
        }
        if source.connection == *i.policy.pending.binding().connection() {
            let owner = i.owner(&source.connection)?;
            let view = i.read.media().target();
            if source.conversation != *owner.member.conversation()
                || source.session != *owner.presence.session()
                || source.participant != *owner.member.participant()
                || source.membership_epoch != owner.member.epoch()
                || source.connection_generation != owner.connection.generation()
                || view.runtime_connection_id != source.connection.as_str()
                || !i.policy.plan.selected().iter().any(|s| {
                    s.requested.logical_id == source.logical_id
                        && crate::canonical_media::allows(view, s.requested.medium, &s.format)
                })
            {
                return Err(StreamControlError::Authority);
            }
        } else if source.connection.as_str()
            == i.read
                .media()
                .source()
                .context()
                .source_runtime_connection_id
            && i.source_phone.is_some()
        {
            i.source_phone
                .as_ref()
                .ok_or(StreamControlError::Authority)?
                .source_matches(source)?;
            let _ = i.owner(&source.connection)?;
        } else {
            let entry = i
                .entry(&source.connection)
                .map_err(|error| i.refused("receive-registry", &source.connection, error))?;
            let view = i
                .read
                .existing(source.connection.as_str(), None)
                .ok_or(StreamControlError::Authority)?
                .target();
            crate::canonical_media::source_matches(source, &entry, view)
                .map_err(|error| i.refused("receive-source", &source.connection, error))?;
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
        if !i
            .read
            .permits_receive(self.target.as_str(), source.connection.as_str())
        {
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
        original
            .member
            .revalidate_principal(&source.principal, chrono::Utc::now())
            .map_err(|_| StreamControlError::Authority)?;
        let mut owner = i.owner(&self.target)?;
        owner.may_route_source = true;
        Ok(Box::new(owner))
    }
}
