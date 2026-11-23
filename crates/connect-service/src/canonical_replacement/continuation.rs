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
    ReserveVideo,
    WarmVideo(ManagedAudioDeviceReplacementPlan),
    BridgeVideo(ManagedAudioDeviceReplacementPlan),
    Activate(AudioAudiencePlan),
    AudioPreroll(u64),
    Preroll(
        rvoip_core::staged_streams::device_replacement::ReplacementVideoRequest,
        u64,
    ),
    Signal(DeviceReplacementEndpointEvent),
    // Compatible scene Readers only: original old-video drain must not hold
    // publication Writer custody across physical IO or public B warmup.
    FreezeVideo(ManagedAudioDeviceReplacementPlan),
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
    pub final_turn: bool,
    pub managed_capture: bool,
    pub final_sip_route: Option<rvoip_sip::adapter::SipFinalRouteView>,
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
            Phase::ReserveVideo => "reserve-video",
            Phase::WarmVideo(_) => "warm-video",
            Phase::BridgeVideo(_) => "bridge-video",
            Phase::Activate(_) => "activate",
            Phase::Preroll(_, _) => "preroll",
            Phase::AudioPreroll(_) => "audio-preroll",
            Phase::Signal(_) => "signal",
            Phase::FreezeVideo(_) => "freeze-video",
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
    pub(super) fn check(&self) -> ControlResult<()> {
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
            if matches!(self.phase, Phase::Commit(_)) && !self.managed_capture {
                if let Some(result) = self.policy.base.probe_held_registry_for_test(
                    &self.root.origin.request().operation_id,
                    || {
                        let legacy = if self.final_turn && self.source_phone.is_some() {
                            self.final_sip_route
                                .as_ref()
                                .ok_or(StreamControlError::MetadataBusy)
                                .and_then(|view| {
                                    self.policy.current_owner_with_final_route_for_test(
                                        self.read.media(),
                                        view,
                                        self.root.origin.record_id(),
                                    )
                                })
                        } else {
                            self.policy.current_owner(self.read.media())
                        };
                        let refused = matches!(legacy, Err(StreamControlError::Authority));
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
                let source = self
                    .read
                    .media()
                    .source()
                    .source()
                    .telephone()
                    .ok_or(StreamControlError::Authority)?;
                if self.final_turn {
                    return phone.established_owner_with_final_route(
                        source,
                        self.final_sip_route
                            .as_ref()
                            .ok_or(StreamControlError::MetadataBusy)?,
                        self.root.origin.record_id(),
                    );
                }
                return phone.established_owner(source);
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
    pub(super) fn principal_for(
        &self,
        id: &ConnectionId,
    ) -> ControlResult<rvoip_core::AuthenticatedPrincipal> {
        Ok(if id == self.policy.pending.binding().connection() {
            self.policy.principal.clone()
        } else if id.as_str()
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
            self.entry(id)?.principal.clone()
        })
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
                Phase::AudioPreroll(original_revision),
                MediaControlIntent::PrepareReplacementAudio { source, expected },
            ) => {
                scope.connection == self.policy.pending.binding().connection()
                    && source.as_str()
                        == self
                            .read
                            .media()
                            .source()
                            .context()
                            .source_runtime_connection_id
                    && expected == original_revision
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
            (
                Phase::Commit(expected)
                | Phase::FreezeVideo(expected)
                | Phase::WarmVideo(expected)
                | Phase::BridgeVideo(expected),
                MediaControlIntent::DeviceReplacement { plan },
            ) => expected.media().audience().same_intent(plan),
            _ => false,
        };
        if !valid {
            tracing::debug!(target:"connect_service::canonical_replacement",candidate=scope.connection==self.policy.pending.binding().connection(),"Replacement continuation refused control intent");
            return Err(StreamControlError::Authority);
        }
        if let Phase::Commit(plan) = &self.phase {
            self.root
                .public_video
                .require_commit_ready(&self.root, plan)
                .map_err(|_| StreamControlError::Authority)?;
        }
        let owner = self.owner(scope.connection)?;
        let principal = self.principal_for(scope.connection)?;
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
    pub(super) fn final_role_scope_current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> RoleResult<()> {
        let deny = || ParticipantControlError::Authority;
        self.check().map_err(|_| deny())?;
        if scope.intent != self.root.handle.intent()
            || !match (&self.phase, scope.plan) {
                (
                    Phase::Commit(expected)
                    | Phase::FreezeVideo(expected)
                    | Phase::WarmVideo(expected)
                    | Phase::BridgeVideo(expected),
                    Some(plan),
                ) => expected.media().audience().same_intent(plan.audience()),
                (
                    Phase::Attach
                    | Phase::ReserveVideo
                    | Phase::Activate(_)
                    | Phase::Preroll(_, _)
                    | Phase::AudioPreroll(_),
                    None,
                ) => true,
                _ => false,
            }
            || !matches!(
                (&self.phase, scope.phase),
                (Phase::Preroll(_, _), ReplacementPhase::Preroll)
                    | (Phase::AudioPreroll(_), ReplacementPhase::Preroll)
                    | (Phase::Attach, ReplacementPhase::Attach)
                    | (Phase::ReserveVideo, ReplacementPhase::ReserveVideo)
                    | (Phase::WarmVideo(_), ReplacementPhase::WarmVideo)
                    | (Phase::BridgeVideo(_), ReplacementPhase::BridgeVideo)
                    | (Phase::Activate(_), ReplacementPhase::Activate)
                    | (Phase::Commit(_), ReplacementPhase::Commit)
                    | (Phase::FreezeVideo(_), ReplacementPhase::FreezeVideo)
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
        if let Phase::Commit(plan) = &self.phase {
            self.root
                .public_video
                .require_commit_ready(&self.root, plan)
                .map_err(|_| deny())?;
        }
        Ok(())
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
        self.final_role_scope_current(scope)?;
        let deny = || ParticipantControlError::Authority;
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

impl Invocation {
    pub(crate) fn public_video_reservation(
        &self,
        scope: &MediaScope<'_>,
        reserved: &rvoip_core::staged_streams::public_video_reservation::PublicVideoReservationPreparation,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.check()?;
        let intent = self
            .root
            .public_video
            .reservation()
            .map_err(|_| StreamControlError::Authority)?
            .intent();
        if !matches!(self.phase, Phase::ReserveVideo)
            || !matches!(scope.purpose, MediaPurpose::Control)
            || !reserved.is_current()
            || !reserved.matches_original_preparation(&self.root.handle)
            || reserved.original_intent() != self.root.handle.intent()
            || reserved.target().logical_id.as_str() != intent.public_stream_id
            || scope.connection != &self.root.handle.intent().replacement
            || scope.conversation != reserved.conversation()
            || scope.session != reserved.session()
            || !reliable_data::same_credential(scope.principal, &self.policy.principal)
        {
            return Err(StreamControlError::Authority);
        }
        let mut owner = self.owner(scope.connection)?;
        // This callback owns only control/resource admission, never an audience.
        owner.may_route_source = false;
        owner.send_media.clear();
        owner.receive_media.clear();
        Ok(Box::new(Held {
            invocation: self.clone(),
            target: scope.connection.clone(),
            owner,
        }))
    }
    pub(crate) fn public_video_warm(
        &self,
        scope: &MediaScope<'_>,
        warm: &rvoip_core::staged_streams::direct_subscriptions::PostCutPublicVideoScope,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.check()?;
        let Phase::WarmVideo(plan) = &self.phase else {
            return Err(StreamControlError::Authority);
        };
        let reserved = self
            .root
            .public_video
            .reserved()
            .map_err(|_| StreamControlError::Authority)?;
        let MediaPurpose::Receive { source } = scope.purpose else {
            return Err(StreamControlError::Authority);
        };
        if !warm.is_current()
            || warm.is_published()
            || warm.original_intent() != self.root.handle.intent()
            || warm.target() != reserved.target()
            || warm.frontier().operation_id() != self.root.handle.intent().operation_id
            || scope.connection != &self.root.handle.intent().replacement
            || scope.conversation != warm.conversation()
            || scope.session != warm.session()
            || !same_public_video_source(source, warm.source())
            || plan.media().request().operation_id != self.root.handle.intent().operation_id
            || !reliable_data::same_credential(scope.principal, &self.policy.principal)
        {
            return Err(StreamControlError::Authority);
        }
        // Reuse the exact held scene's current receive permission for this source.
        // The Core-owned warm path independently restricts it to the actual B/F.
        let owner = self.owner(scope.connection)?;
        let held = Held {
            invocation: self.clone(),
            target: scope.connection.clone(),
            owner,
        };
        let owner = *held.control_receive(source)?;
        Ok(Box::new(Held { owner, ..held }))
    }

    pub(crate) fn pre_cut_public_video_bridge(
        &self,
        scope: &MediaScope<'_>,
        bridge: &rvoip_core::staged_streams::direct_subscriptions::PreCutPublicVideoBridgeScope,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.check()?;
        let Phase::BridgeVideo(plan) = &self.phase else {
            return Err(StreamControlError::Authority);
        };
        let MediaPurpose::Receive { source } = scope.purpose else {
            return Err(StreamControlError::Authority);
        };
        if !bridge.is_current()
            || !self.root.public_video.bridge_resource_matches(bridge)
            || bridge.original_intent() != self.root.handle.intent()
            || bridge.target()
                != self
                    .root
                    .public_video
                    .reserved()
                    .map_err(|_| StreamControlError::Authority)?
                    .target()
            || plan.media().request().operation_id != self.root.handle.intent().operation_id
            || scope.connection != &self.root.handle.intent().replacement
            || scope.conversation != bridge.conversation()
            || scope.session != bridge.session()
            || !same_public_video_source(source, bridge.source())
            || !reliable_data::same_credential(scope.principal, &self.policy.principal)
        {
            return Err(StreamControlError::Authority);
        }
        let held = Held {
            invocation: self.clone(),
            target: scope.connection.clone(),
            owner: self.owner(scope.connection)?,
        };
        let owner = *held.control_receive(source)?;
        Ok(Box::new(Held { owner, ..held }))
    }
}
pub(super) fn same_public_video_source(a: &MediaSource, b: &MediaSource) -> bool {
    a.conversation == b.conversation
        && a.session == b.session
        && a.connection == b.connection
        && a.participant == b.participant
        && a.membership_epoch == b.membership_epoch
        && a.connection_generation == b.connection_generation
        && a.logical_id == b.logical_id
        && a.transport_id == b.transport_id
        && a.stream_generation == b.stream_generation
}
