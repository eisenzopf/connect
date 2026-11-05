//! Exact provisional Core replacement binding. No ordinary generation-one grant.
use super::*;
use crate::canonical_runtime::current_member;
use connect_store::{NativeReplacementMediaOrigin, NativeReplacementStartRead};
use rvoip_core::{
    ids::IdentityId,
    participant_transitions::PreparedParticipantReplacement,
    relationships::{EndpointBinding, IdentityBinding},
    staged_streams::*,
    stream_lifecycle::{Medium, StreamGateAction},
    AuthenticatedPrincipal,
};
use rvoip_uctp::v2::{
    auth_dispatch::AuthorizedEnvelope,
    native_negotiation::{NativeMediaActivation, NativeMediaProfile, NativeOfferPlan},
};
use std::{
    any::Any,
    collections::BTreeSet,
    sync::{OnceLock, Weak},
};

pub(crate) struct TargetPolicy {
    pub base: Arc<crate::canonical_media::Policy>,
    pub pending: PreparedParticipantReplacement,
    pub principal: AuthenticatedPrincipal,
    pub profile: Arc<NativeMediaProfile>,
    pub plan: NativeOfferPlan,
    pub root: Weak<PreparedReplacement>,
    pub activation: OnceLock<Arc<Activation>>,
    pub(crate) public_continuation:
        OnceLock<Arc<crate::canonical_media::public_continuation::Continuation>>,
    pub origin: OnceLock<Arc<NativeReplacementMediaOrigin>>,
    pub preparation_operation: OnceLock<String>,
    pub data_observer: Option<Arc<crate::canonical_media::data_writes::PendingObserver>>,
    pub(super) readiness: Option<Arc<super::readiness::ReadyGate>>,
}
/// Both adapters retain their exact sealed Core resource observation. The enum
/// only dispatches typed operations; it cannot manufacture readiness or replace
/// an original activation with another implementation during publication.
pub(crate) enum Activation {
    Native(Arc<NativeMediaActivation>),
    WebRtc(Arc<rvoip_webrtc::canonical::RtcMediaActivation>),
}
impl Activation {
    pub(crate) fn confirmed_generation(&self) -> Option<u64> {
        MediaActivation::confirmed_generation(self)
    }
    pub(crate) fn try_confirmed_generation(&self) -> ControlResult<Option<u64>> {
        match self {
            Self::Native(a) => a.try_confirmed_generation(),
            Self::WebRtc(a) => a.try_confirmed_generation(),
        }
    }
    pub(crate) fn try_publish_installed<R>(
        &self,
        generation: u64,
        publish: impl FnOnce() -> R,
    ) -> ControlResult<R> {
        match self {
            Self::Native(a) => a.try_publish_installed(generation, publish),
            Self::WebRtc(a) => a.try_publish_installed(generation, publish),
        }
    }
}
impl MediaActivation for Activation {
    fn prepare(
        &self,
        prepared: &Arc<PreparedMediaObservation>,
        current: MediaOwner,
    ) -> ControlResult<MediaOwner> {
        match self {
            Self::Native(a) => a.prepare(prepared, current),
            Self::WebRtc(a) => a.prepare(prepared, current),
        }
    }
    fn current(&self, current: MediaOwner) -> ControlResult<MediaOwner> {
        match self {
            Self::Native(a) => a.current(current),
            Self::WebRtc(a) => a.current(current),
        }
    }
    fn confirmed_generation(&self) -> Option<u64> {
        match self {
            Self::Native(a) => a.confirmed_generation(),
            Self::WebRtc(a) => a.confirmed_generation(),
        }
    }
    fn prepare_replacement(
        &self,
        prepared: &Arc<PreparedMediaObservation>,
        current: MediaOwner,
        source: &MediaOwner,
        pending: &PreparedParticipantReplacement,
    ) -> ControlResult<MediaOwner> {
        match self {
            Self::Native(a) => a.prepare_replacement(prepared, current, source, pending),
            Self::WebRtc(a) => a.prepare_replacement(prepared, current, source, pending),
        }
    }
    fn current_replacement(
        &self,
        current: MediaOwner,
        source: &MediaOwner,
        pending: &PreparedParticipantReplacement,
    ) -> ControlResult<MediaOwner> {
        match self {
            Self::Native(a) => a.current_replacement(current, source, pending),
            Self::WebRtc(a) => a.current_replacement(current, source, pending),
        }
    }
}
#[derive(Clone)]
pub(super) enum StartAuthority {
    Signed(Arc<AuthorizedEnvelope>),
    Resumed {
        proof: Arc<connect_store::PeerAuthorityGuard>,
        peer: Arc<rvoip_core::peer_admission::PeerAdmission>,
        cancel: tokio_util::sync::CancellationToken,
    },
}
impl StartAuthority {
    fn current(&self, principal: &AuthenticatedPrincipal) -> bool {
        match self {
            Self::Signed(a) => {
                !a.invocation_cancellation().is_cancelled()
                    && rvoip_core::staged_streams::reliable_data::same_credential(
                        a.principal(),
                        principal,
                    )
            }
            Self::Resumed {
                proof,
                peer,
                cancel,
            } => {
                proof.current()
                    && !cancel.is_cancelled()
                    && peer.validate_principal(principal).is_ok()
            }
        }
    }
    fn cancellation(&self) -> tokio_util::sync::CancellationToken {
        match self {
            Self::Signed(a) => a.invocation_cancellation(),
            Self::Resumed { cancel, .. } => cancel.clone(),
        }
    }
}
#[derive(Clone)]
pub(super) struct StartInvocation {
    pub policy: Arc<TargetPolicy>,
    pub authority: StartAuthority,
    pub read: Arc<NativeReplacementStartRead>,
    pub entries: crate::canonical_media::replacement::ReplacementPreparationEntries,
    pub deadline: tokio::time::Instant,
}
fn deny<T>(_: T) -> StreamControlError {
    StreamControlError::Authority
}
impl TargetPolicy {
    fn scope(&self, scope: &MediaScope<'_>) -> ControlResult<()> {
        let pending = &self.pending;
        if scope.connection != pending.binding().connection()
            || scope.session != pending.appearance().session()
            || self
                .root
                .upgrade()
                .is_none_or(|r| r.origin.request().conversation_id != scope.conversation.as_str())
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.principal,
            )
            || self
                .root
                .upgrade()
                .is_none_or(|r| r.handle.cancellation().is_cancelled() || r.handle.is_committed())
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn owners(
        &self,
        source: &connect_store::NativeReplacementPreparationRead,
        target: &connect_store::PeerConnectionProjection,
    ) -> ControlResult<(MediaOwner, MediaOwner)> {
        let old = self.base.replacement_source_owner(source).map_err(deny)?;
        self.owners_with_source(source, target, old)
    }
    fn owners_with_source(
        &self,
        source: &connect_store::NativeReplacementPreparationRead,
        target: &connect_store::PeerConnectionProjection,
        old: MediaOwner,
    ) -> ControlResult<(MediaOwner, MediaOwner)> {
        if !source.current() {
            return Err(StreamControlError::Authority);
        }
        let member = current_member(&self.principal, &target.session).map_err(deny)?;
        let identity = IdentityBinding::from_authenticated(
            &self.principal,
            IdentityId::from_string(target.session.actor.identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(deny)?;
        let mut endpoint = EndpointBinding::provision(
            identity.clone(),
            EndpointId::from_string(target.connection.endpoint_id.clone()),
            DeviceId::from_string(target.device_id.clone()),
            crate::canonical_runtime::core_transport(&target.endpoint_transport).map_err(deny)?,
            target.endpoint_address.clone(),
        )
        .map_err(deny)?;
        endpoint
            .verify(&identity, "connect-store".into(), chrono::Utc::now())
            .map_err(deny)?;
        let binding = self.pending.binding().clone();
        let presence = self.pending.appearance().clone();
        if binding.connection().as_str() != target.runtime_connection_id
            || binding.generation() != target.connection.generation
            || binding.endpoint() != endpoint.endpoint()
            || binding.device() != endpoint.device()
            || binding.replaces() != Some(&self.pending.intent().source)
            || presence.role() != &crate::canonical_runtime::participant_role(&target.presence_role)
        {
            return Err(StreamControlError::Authority);
        }
        binding.require_current(&member, &presence).map_err(deny)?;
        let local = crate::canonical_media::filtered(target, self.profile.engine_capabilities());
        let remote = crate::canonical_media::filtered(target, self.plan.remote_capabilities());
        let intersection =
            |local: &[rvoip_core::stream_lifecycle::MediaCapability],
             remote: &[rvoip_core::stream_lifecycle::MediaCapability]| {
                local
                    .iter()
                    .filter(|a| {
                        matches!(a.medium, Medium::Audio | Medium::Video | Medium::Data)
                            && remote.iter().any(|b| {
                                b.medium == a.medium
                                    && a.formats.iter().any(|f| b.formats.contains(f))
                            })
                    })
                    .map(|c| c.medium)
                    .collect::<BTreeSet<_>>()
            };
        let owner = MediaOwner {
            actor: member.clone(),
            member,
            presence,
            endpoint,
            connection: binding,
            may_control: true,
            send_media: intersection(&local.send, &remote.receive),
            receive_media: intersection(&local.receive, &remote.send),
            local,
            remote,
            may_route_source: false,
        };
        Ok((owner, old))
    }
    pub(super) fn current_owner(
        &self,
        read: &connect_store::NativeReplacementMediaRead,
    ) -> ControlResult<MediaOwner> {
        let (owner, old) = self.owners(read.source(), read.target())?;
        self.activation
            .get()
            .ok_or(StreamControlError::NotReady)?
            .current_replacement(owner, &old, &self.pending)
    }
    /// The held invocation derives this source from its exact owned scene
    /// entries and current read; packet callbacks continue to use current_owner.
    pub(super) fn current_owner_with_source(
        &self,
        read: &connect_store::NativeReplacementMediaRead,
        old: MediaOwner,
    ) -> ControlResult<MediaOwner> {
        let (owner, old) = self.owners_with_source(read.source(), read.target(), old)?;
        self.activation
            .get()
            .ok_or(StreamControlError::NotReady)?
            .current_replacement(owner, &old, &self.pending)
    }
    fn preparation_authority(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
        output: bool,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.scope(scope)?;
        let local_audio::LocalAudioPurpose::EndpointPreparation { operation_id } = purpose;
        if self.preparation_operation.get() != Some(operation_id) {
            return Err(StreamControlError::Authority);
        }
        let read = self
            .base
            .store
            .try_native_replacement_media_read(
                self.origin.get().ok_or(StreamControlError::NotReady)?,
            )
            .map_err(|error| {
                tracing::debug!(target:"connect_service::canonical_replacement",code=%error.code,"Private replacement media read deferred");
                crate::canonical_media::packet_authority_error(error)
            })?;
        let mut owner = self.current_owner(&read)?;
        owner.may_route_source = output;
        Ok(Box::new(Fresh { owner, _read: read }))
    }
    fn held_owners(&self, i: &StartInvocation) -> ControlResult<(MediaOwner, MediaOwner)> {
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy))
            || !i.read.current()
            || !i.authority.current(&self.principal)
            || tokio::time::Instant::now() >= i.deadline
        {
            return Err(StreamControlError::Authority);
        }
        // These references are metadata, not grants. Derive both original
        // owners under this same current Store read on every control check.
        // Re-entering the packet registry here would turn unrelated metadata
        // contention during negotiation/mute into a permanent Authority error.
        let old = self
            .base
            .replacement_source_owner_with_entries(i.read.source(), &i.entries)
            .map_err(deny)?;
        self.owners_with_source(i.read.source(), i.read.target(), old)
    }
}
struct Held {
    invocation: StartInvocation,
    owner: MediaOwner,
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        let (owner, old) = self.invocation.policy.held_owners(&self.invocation)?;
        match self
            .invocation
            .policy
            .activation
            .get()
            .filter(|a| a.confirmed_generation().is_some())
        {
            Some(activation) => activation
                .current_replacement(owner, &old, &self.invocation.policy.pending)
                .map(Box::new),
            None => Ok(Box::new(owner)),
        }
    }
    fn control_prepared(
        &self,
        prepared: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        let (owner, old) = self.invocation.policy.held_owners(&self.invocation)?;
        self.invocation
            .policy
            .activation
            .get()
            .ok_or(StreamControlError::NotReady)?
            .prepare_replacement(prepared, owner, &old, &self.invocation.policy.pending)
            .map(Box::new)
    }
}
struct Fresh {
    owner: MediaOwner,
    _read: connect_store::NativeReplacementMediaRead,
}
impl MediaAuthorityLease for Fresh {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
impl MediaPolicy for TargetPolicy {
    fn current_audio_preparation(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.preparation_authority(scope, purpose, false)
    }
    fn current_local_audio(
        &self,
        scope: &MediaScope<'_>,
        purpose: &local_audio::LocalAudioPurpose,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.preparation_authority(scope, purpose, true)
    }
    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.scope(scope)?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            let gate = self
                .readiness
                .as_ref()
                .ok_or(StreamControlError::NotReady)?;
            if !gate.permits(source) {
                return Err(StreamControlError::Authority);
            }
            let entries = self.base.scene_entries(scope.session.as_str())?;
            let origins = entries
                .values()
                .map(|entry| entry.origin.clone())
                .collect::<Vec<_>>();
            let read = self
                .base
                .store
                .try_native_replacement_scene_read(
                    self.origin.get().ok_or(StreamControlError::NotReady)?,
                    &origins,
                )
                .map_err(crate::canonical_media::packet_authority_error)?;
            if !read.current()
                || !read.permits_receive(scope.connection.as_str(), source.connection.as_str())
            {
                return Err(StreamControlError::Authority);
            }
            let entry = entries
                .get(&source.connection)
                .ok_or(StreamControlError::Authority)?;
            let source_read = read
                .existing(source.connection.as_str(), None)
                .ok_or(StreamControlError::Authority)?;
            crate::canonical_media::source_matches(source, entry, source_read.target())?;
            let mut owner = self.current_owner(read.media())?;
            owner.may_route_source = true;
            return Ok(Box::new(PrivateReceive { owner, _read: read }));
        }
        // Resource preparation grants no public speaking/subscription route.
        // Explicit quarantined input/output is installed by a later phase.
        if !matches!(scope.purpose, MediaPurpose::Control) {
            return Err(StreamControlError::NotReady);
        }
        let origin = self.origin.get().ok_or(StreamControlError::NotReady)?;
        let read = self
            .base
            .store
            .try_native_replacement_media_read(origin)
            .map_err(crate::canonical_media::packet_authority_error)?;
        let (owner, old) = self.owners(read.source(), read.target())?;
        let owner = self
            .activation
            .get()
            .ok_or(StreamControlError::NotReady)?
            .current_replacement(owner, &old, &self.pending)?;
        Ok(Box::new(Fresh { owner, _read: read }))
    }
    fn current_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<MediaControl> {
        self.scope(scope)?;
        if let Some(i) = held.downcast_ref::<super::continuation::Invocation>() {
            if !std::ptr::eq(self, Arc::as_ptr(&i.policy)) {
                return Err(StreamControlError::Authority);
            }
            return i.media_control(scope, intent);
        }
        let i = held
            .downcast_ref::<StartInvocation>()
            .ok_or(StreamControlError::Authority)?;
        let allowed = match intent {
            MediaControlIntent::Initialize => true,
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } => {
                *expected == 0
                    && replace.is_empty()
                    && desired
                        .iter()
                        .eq(self.plan.selected().iter().map(|s| &s.requested))
            }
            MediaControlIntent::Gate {
                expected,
                streams,
                action,
            } => {
                *expected == 1
                    && *action == StreamGateAction::Mute
                    && **streams
                        == self
                            .plan
                            .selected()
                            .iter()
                            .filter(|s| {
                                s.requested.direction
                                    != rvoip_core::stream_lifecycle::StreamDirection::Send
                            })
                            .map(|s| s.requested.logical_id.clone())
                            .collect()
            }
            _ => false,
        };
        if !allowed {
            return Err(StreamControlError::Authority);
        }
        let (owner, _) = self.held_owners(i)?;
        #[cfg(feature = "test-support")]
        let owner = if matches!(intent, MediaControlIntent::Gate { .. }) {
            let operation = self
                .root
                .upgrade()
                .ok_or(StreamControlError::Authority)?
                .origin
                .request()
                .operation_id
                .clone();
            match self
                .base
                .probe_held_registry_for_test(&format!("start:{operation}"), || {
                    let legacy_refused = matches!(
                        self.owners(i.read.source(), i.read.target()),
                        Err(StreamControlError::Authority)
                    );
                    let mut expired = i.clone();
                    expired.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
                    let expired_refused = matches!(
                        self.held_owners(&expired),
                        Err(StreamControlError::Authority)
                    );
                    (
                        legacy_refused && expired_refused,
                        self.held_owners(i).map(|(owner, _)| owner),
                    )
                }) {
                Some(result) => result?,
                None => owner,
            }
        } else {
            owner
        };
        MediaControl::new(
            Box::new(Held {
                invocation: i.clone(),
                owner,
            }),
            i.authority.cancellation(),
            i.deadline,
        )
    }
}

struct PrivateReceive {
    owner: MediaOwner,
    _read: connect_store::NativeReplacementSceneRead,
}
impl MediaAuthorityLease for PrivateReceive {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self._read.media().source().expires_at(), 0)
    }
}
