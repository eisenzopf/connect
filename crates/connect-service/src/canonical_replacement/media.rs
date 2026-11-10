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
    pub(super) receive_diagnostics: super::diagnostics::PrivateReceive,
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
/// Dispatch only the exact owned startup invocation. These held references are
/// metadata, not a grant: TargetPolicy retains its current Store/physical/root,
/// scope, intent and original-deadline checks for every control admission.
pub(crate) fn held_start_control(
    base: &crate::canonical_media::Policy,
    scope: &MediaScope<'_>,
    intent: &MediaControlIntent<'_>,
    held: &(dyn Any + Send + Sync),
) -> Option<ControlResult<MediaControl>> {
    let invocation = held.downcast_ref::<StartInvocation>()?;
    Some(
        if !std::ptr::eq(base, Arc::as_ptr(&invocation.policy.base)) {
            Err(StreamControlError::Authority)
        } else {
            invocation.policy.current_control(scope, intent, held)
        },
    )
}

#[cfg(feature = "test-support")]
pub(crate) fn probe_held_start_control_for_test(
    base: &crate::canonical_media::Policy,
    scope: &MediaScope<'_>,
    intent: &MediaControlIntent<'_>,
    held: &(dyn Any + Send + Sync),
) -> Option<ControlResult<MediaControl>> {
    let invocation = held.downcast_ref::<StartInvocation>()?;
    if !matches!(intent, MediaControlIntent::Change { expected: 0, .. })
        || !matches!(&invocation.authority, StartAuthority::Resumed { .. })
    {
        return None;
    }
    let operation = invocation
        .policy
        .root
        .upgrade()?
        .origin
        .request()
        .operation_id
        .clone();
    let observed = base.probe_start_dispatch_for_test(&operation, scope.connection, || {
        let foreign = crate::canonical_media::Policy::new(base.store.clone(), 1);
        let foreign_refused = matches!(
            held_start_control(&foreign, scope, intent, held),
            Some(Err(StreamControlError::Authority))
        );
        let mut expired = invocation.clone();
        expired.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
        let expired_refused = matches!(
            held_start_control(base, scope, intent, &expired),
            Some(Err(StreamControlError::Authority))
        );
        // The probe has been consumed. Re-enter the actual shared policy under
        // all three dispatch registry exclusions, without a second gate effect.
        let current = base.current_control(scope, intent, held);
        (foreign_refused, expired_refused, current)
    })?;
    // Only this consumed, armed test probe can continue after its deliberately
    // contended old dispatch. All registry guards have now dropped. Preserve
    // the failed witness, but reacquire the same current held policy admission
    // so the original gate and all fixture owners can reach natural cleanup.
    // This is not a second gate call; no gate effect has occurred at admission.
    Some(match observed {
        Err(StreamControlError::NotReady) => held_start_control(base, scope, intent, held)
            .expect("same original typed startup invocation"),
        outcome => outcome,
    })
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
        self.receive_diagnostics.begin_preparation(output);
        let result = (|| {
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
            Ok(Box::new(Fresh { owner, _read: read }) as Box<dyn MediaAuthorityLease>)
        })();
        self.receive_diagnostics.preparation_result(output, &result);
        result
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
        if let MediaPurpose::Receive { source } = scope.purpose {
            return self.private_receive(scope, source);
        }
        self.scope(scope)?;
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
        let owner = if matches!(intent, MediaControlIntent::Change { expected: 0, .. })
            && matches!(&i.authority, StartAuthority::Resumed { .. })
        {
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
                    let published_current = self.owners(i.read.source(), i.read.target()).is_ok();
                    let mut expired = i.clone();
                    expired.deadline = tokio::time::Instant::now() - Duration::from_secs(1);
                    let expired_refused = matches!(
                        self.held_owners(&expired),
                        Err(StreamControlError::Authority)
                    );
                    (
                        published_current && expired_refused,
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

impl TargetPolicy {
    pub(crate) fn receiver_epoch_preparation(
        &self,
        scope: &rvoip_uctp::v2::native_lifecycle::NativeLifecycleScope,
        observation: &rvoip_uctp::v2::native_lifecycle::VideoEpochObservation,
    ) -> Result<(
        MediaOwner,
        Arc<connect_store::NativeReplacementMediaRead>,
        Weak<rvoip_core::staged_streams::device_replacement::VideoReplacementPreparation>,
    )> {
        let read = Arc::new(self.base.store.try_native_replacement_receiver_epoch_read(
            self.origin.get().ok_or_else(|| {
                Error::new(
                    "authority-busy",
                    "Original target is not ready for receiver metadata",
                )
            })?,
        )?);
        let context = read.source().context();
        let target = read.target();
        let binding = observation.binding();
        let root = self.root.upgrade().ok_or_else(Error::forbidden)?;
        let gate = self.readiness.as_ref().ok_or_else(Error::forbidden)?;
        let preroll = gate.receiver_epoch_preroll(binding).ok_or_else(|| {
            Error::new("authority-busy", "Original receiver preroll is not current")
        })?;
        if observation.epoch().publisher
            || !read.current()
            || root.handle.cancellation().is_cancelled()
            || root.handle.is_committed()
            || scope.media.connection != *self.pending.binding().connection()
            || scope.media.connection.as_str() != target.runtime_connection_id
            || binding.connection_generation != target.connection.generation
            || scope.media.session.as_str() != context.session_id
            || scope.media.wire_conversation.as_deref() != Some(context.conversation_id.as_str())
            || scope.media.wire_session != context.session_id
            || scope.media.wire_connection != target.connection.connection_id
            || scope.media.owner != self.principal.ownership_key()
            || scope.participant.as_str() != context.participant_id
            || scope.membership_epoch != context.membership_epoch
            || scope.device.as_str() != target.device_id
            || scope.endpoint.as_str() != target.connection.endpoint_id
            || scope.identity.as_str() != target.session.actor.identity_id
            || !reliable_data::same_credential(&scope.principal, &self.principal)
        {
            return Err(Error::forbidden());
        }
        #[cfg(feature = "test-support")]
        let probed = self
            .base
            .probe_receiver_epoch_registry_for_test(&root.origin.request().operation_id, || {
                self.receiver_epoch_owner(&read)
            });
        #[cfg(not(feature = "test-support"))]
        let probed: Option<Result<MediaOwner>> = None;
        let owner = match probed {
            Some(result) => result?,
            None => self.receiver_epoch_owner(&read)?,
        };
        Ok((owner, read, Arc::downgrade(&preroll)))
    }

    fn receiver_epoch_owner(
        &self,
        read: &connect_store::NativeReplacementMediaRead,
    ) -> Result<MediaOwner> {
        // Capture exact registry metadata under this fresh read. Contention is
        // not evidence that either original physical owner lost authority.
        let entries = self
            .base
            .replacement_preparation_entries(read.source())
            .map_err(|error| {
                if error.code == "authority-busy" {
                    error
                } else {
                    Error::forbidden()
                }
            })?;
        let source = self
            .base
            .replacement_source_owner_with_entries(read.source(), &entries)
            .map_err(|_| Error::forbidden())?;
        self.current_owner_with_source(read, source)
            .map_err(|error| match error {
                StreamControlError::NotReady | StreamControlError::MetadataBusy => {
                    Error::new("authority-busy", "Original receiver owner is busy")
                }
                _ => Error::forbidden(),
            })
    }
}

impl TargetPolicy {
    pub(crate) fn current_packet_read(
        &self,
        scope: &MediaScope<'_>,
        held: &connect_store::NativeMediaRead,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let MediaPurpose::Receive { source } = scope.purpose else {
            return Err(StreamControlError::Authority);
        };
        self.private_receive_under(scope, source, Some(held))
    }
    fn private_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.private_receive_under(scope, source, None)
    }
    fn private_receive_under(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaSource,
        held: Option<&connect_store::NativeMediaRead>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.receive_diagnostics.begin_receive(held.is_some());
        let mut leaf = 0;
        let mut medium = None;
        let mut store_busy = None;
        let result = (|| {
            self.scope(scope)?;
            leaf = 1;
            let gate = self
                .readiness
                .as_ref()
                .ok_or(StreamControlError::NotReady)?;
            leaf = 2;
            if !gate.permits(source) {
                return Err(StreamControlError::Authority);
            }
            leaf = 3;
            let entries = self.base.scene_entries(scope.session.as_str())?;
            medium = entries.get(&source.connection).and_then(|entry| {
                entry
                    .plan
                    .selected()
                    .iter()
                    .find(|binding| binding.requested.logical_id == source.logical_id)
                    .map(|binding| binding.requested.medium)
            });
            // One physical receive needs the original replacement owners and
            // this exact publisher. Generic preparation/commit still captures
            // the full scene; unrelated peers lend no permission to this IO.
            let intent = self.pending.intent();
            let origins = entries
                .iter()
                .filter(|(connection, _)| {
                    **connection == intent.source
                        || **connection == intent.retained
                        || **connection == source.connection
                })
                .map(|(_, entry)| entry.origin.clone())
                .collect::<Vec<_>>();
            leaf = 4;
            let origin = self.origin.get().ok_or(StreamControlError::NotReady)?;
            leaf = 5;
            let supplied_source = if let Some(held) = held {
                leaf = 8;
                let entry = entries
                    .get(&source.connection)
                    .ok_or(StreamControlError::Authority)?;
                leaf = 10;
                crate::canonical_media::source_matches(source, entry, held.target())?;
                Some((entry, held))
            } else {
                None
            };
            leaf = 5;
            let result = match supplied_source {
                Some((entry, held)) => self.base.store.try_native_replacement_scene_read_under(
                    origin,
                    &origins,
                    &entry.origin,
                    held,
                ),
                None => self
                    .base
                    .store
                    .try_native_replacement_scene_read(origin, &origins),
            };
            let read = super::diagnostics::private_scene_result(result, &mut store_busy)?;
            leaf = 6;
            if !read.current() {
                return Err(StreamControlError::Authority);
            }
            leaf = 7;
            if !read.permits_receive(scope.connection.as_str(), source.connection.as_str()) {
                return Err(StreamControlError::Authority);
            }
            leaf = 8;
            let entry = entries
                .get(&source.connection)
                .ok_or(StreamControlError::Authority)?;
            leaf = 9;
            let source_read = read
                .existing(source.connection.as_str(), None)
                .ok_or(StreamControlError::Authority)?;
            leaf = 10;
            crate::canonical_media::source_matches(source, entry, source_read.target())?;
            leaf = 11;
            let (owner, old) = self.owners(read.media().source(), read.media().target())?;
            leaf = 12;
            let activation = self.activation.get().ok_or(StreamControlError::NotReady)?;
            leaf = 13;
            let mut owner = activation.current_replacement(owner, &old, &self.pending)?;
            owner.may_route_source = true;
            Ok(Box::new(PrivateReceive { owner, _read: read }) as Box<dyn MediaAuthorityLease>)
        })();
        // Numeric accounting alone: an Ok still retains its ORIGINAL reader.
        // No formatting, emission, locks or additional authority lookup here.
        self.receive_diagnostics
            .receive_result(held.is_some(), medium, leaf, &result, store_busy);
        // Failed admission has already dropped all SQL/current-owner readers.
        // Each original target emits at most one diagnostic per exact leaf.
        if let Err(error) = &result {
            self.receive_diagnostics
                .refused(leaf, self, scope, source, medium, error, store_busy);
        }
        result
    }
}
