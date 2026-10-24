//! Initial SIP keeps its own actual ticket, principal and current media facts.
use super::*;
use crate::{authentication::StoreValidator, canonical_runtime::Runtime};
use connect_store::{
    InitialTelephoneMediaOrigin, InitialTelephoneMediaRead, InitialTelephoneRead,
    InitialTelephoneSignaling,
};
use rvoip_core::staged_streams::ControlResult;
use rvoip_core::{
    ids::*,
    orchestrator::participant_control::{
        OutboundParticipantPreparation, ParticipantAuthority, ParticipantControl,
        ParticipantControlError, ParticipantControlLease, ParticipantScope,
    },
    participant_transitions::*,
    relationships::*,
    staged_streams::*,
    stream_lifecycle::*,
    AuthenticatedPrincipal, Transport,
};
use std::{
    any::Any,
    collections::BTreeSet,
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio_util::sync::CancellationToken;

pub(crate) struct Target {
    pub principal: AuthenticatedPrincipal,
    pub member: Membership,
    pub endpoint: EndpointBinding,
}
pub(crate) fn target(
    runtime: &Runtime,
    read: &InitialTelephoneRead,
) -> connect_protocol::Result<Target> {
    let source = read.source();
    let principal = StoreValidator::canonical(runtime.media.store.clone())
        .principal(read.endpoint_actor().clone())
        .map_err(|_| Error::forbidden())?;
    let member = member(&principal, &source.target, read.conversation_revision())
        .map_err(|_| Error::forbidden())?;
    let identity = IdentityBinding::from_authenticated(
        &principal,
        IdentityId::from_string(source.target.identity_id.clone()),
        chrono::Utc::now(),
    )
    .map_err(|_| Error::forbidden())?;
    let mut endpoint = EndpointBinding::provision(
        identity.clone(),
        EndpointId::from_string(source.endpoint_id.clone()),
        DeviceId::from_string(source.device_id.clone()),
        Transport::Sip,
        source.destination.clone(),
    )
    .map_err(|_| Error::forbidden())?;
    endpoint
        .verify(
            &identity,
            "connect-store-initial-telephone".into(),
            chrono::Utc::now(),
        )
        .map_err(|_| Error::forbidden())?;
    Ok(Target {
        principal,
        member,
        endpoint,
    })
}
fn member(
    principal: &AuthenticatedPrincipal,
    value: &connect_protocol::membership::MemberView,
    revision: u64,
) -> std::result::Result<Membership, StreamControlError> {
    let identity = IdentityBinding::from_authenticated(
        principal,
        IdentityId::from_string(value.identity_id.clone()),
        chrono::Utc::now(),
    )
    .map_err(|_| StreamControlError::Authority)?;
    Membership::from_current(
        identity,
        ConversationId::from_string(value.conversation_id.clone()),
        ParticipantId::from_string(value.participant_id.clone()),
        match value.kind {
            connect_protocol::provisioning::IdentityKind::Human => {
                rvoip_core::participant::ParticipantKind::Human
            }
            connect_protocol::provisioning::IdentityKind::Ai => {
                rvoip_core::participant::ParticipantKind::Ai
            }
            connect_protocol::provisioning::IdentityKind::System => {
                rvoip_core::participant::ParticipantKind::System
            }
            connect_protocol::provisioning::IdentityKind::External => {
                rvoip_core::participant::ParticipantKind::External
            }
        },
        crate::canonical_runtime::participant_role(&value.role),
        value.membership_epoch,
        revision,
        value.active,
    )
    .map_err(|_| StreamControlError::Authority)
}
pub(crate) struct Leg {
    pub self_ref: Weak<Leg>,
    pub runtime: Weak<Runtime>,
    pub signal: Arc<InitialTelephoneSignaling>,
    pub target: Target,
    pub cancel: CancellationToken,
    pub deadline: tokio::time::Instant,
    pub prepared: OnceLock<Arc<OutboundParticipantPreparation>>,
    pub media: OnceLock<Arc<InitialTelephoneMediaOrigin>>,
    pub context: OnceLock<Weak<rvoip_sip::SipInitialParticipantContext>>,
    pub controller: OnceLock<Weak<StreamController>>,
    pub codec: OnceLock<rvoip_core::capability::CodecInfo>,
    pub binding: OnceLock<ReadyMediaBinding>,
    pub proof: Mutex<Option<Arc<PreparedMediaObservation>>>,
    pub admitted: OnceLock<ParticipantObservation>,
    pub dtmf_registration: OnceLock<String>,
}
#[derive(Clone)]
pub(crate) struct DialInvocation {
    pub leg: Arc<Leg>,
    pub revision: u64,
    // Initial dialing uses signaling authority. Once the actual answer has
    // its own consent, media admission owns this fresh independent interval.
    pub media: Option<Arc<InitialTelephoneMediaRead>>,
}
#[derive(Clone)]
pub(crate) struct MediaInvocation {
    pub leg: Arc<Leg>,
    pub read: Arc<InitialTelephoneMediaRead>,
    pub revision: u64,
}
fn participant_error<T>(_: T) -> ParticipantControlError {
    ParticipantControlError::Authority
}
impl Leg {
    pub(crate) fn observe_audio_audience_commit(
        &self,
        read: &InitialTelephoneMediaRead,
        commit: &rvoip_core::staged_streams::audience::AudioAudienceCommit,
        previous: u64,
    ) -> ControlResult<()> {
        if read.source() != self.signal.origin().source() {
            return Err(StreamControlError::Authority);
        }
        self.owner(read)?;
        let controller = self
            .controller
            .get()
            .and_then(Weak::upgrade)
            .ok_or(StreamControlError::NotReady)?;
        let (before, after) = commit.revisions_for(&controller)?;
        if before != previous || before.checked_add(1) != Some(after) {
            return Err(StreamControlError::Authority);
        }
        // SIP keeps its real codec/dialog admission and exact active ports.
        // It has no native signed resource confirmation to fabricate or carry.
        Ok(())
    }
    pub fn connection(&self) -> ConnectionId {
        ConnectionId::from_string(self.signal.runtime_connection_id().to_owned())
    }
    pub fn runtime(&self) -> std::result::Result<Arc<Runtime>, StreamControlError> {
        self.runtime
            .upgrade()
            .filter(|r| !r.stop.is_cancelled() && !self.cancel.is_cancelled())
            .ok_or(StreamControlError::Cancelled)
    }
    pub fn origin(
        &self,
    ) -> std::result::Result<Arc<InitialTelephoneMediaOrigin>, StreamControlError> {
        self.media
            .get()
            .cloned()
            .ok_or(StreamControlError::NotReady)
    }
    pub(crate) fn current_media_read(&self) -> connect_protocol::Result<InitialTelephoneMediaRead> {
        let runtime = self.runtime().map_err(|_| Error::forbidden())?;
        let origin = self.origin().map_err(|_| Error::forbidden())?;
        if self.signal.origin().source().is_private_candidate() {
            runtime
                .media
                .store
                .try_private_initial_telephone_admission_read(origin)
        } else {
            runtime.media.store.try_initial_telephone_media_read(origin)
        }
    }
    pub fn participant(
        &self,
        scope: &ParticipantScope<'_>,
        revision: u64,
    ) -> std::result::Result<ParticipantAuthority, ParticipantControlError> {
        let source = self.signal.origin().source();
        if self.cancel.is_cancelled()
            || self
                .prepared
                .get()
                .is_some_and(|p| p.is_private_audio() != source.is_private_candidate())
            || tokio::time::Instant::now() >= self.deadline
            || scope.session.as_str() != source.session_id
            || scope.conversation.as_str() != source.conversation_id
            || scope.mutation.operation_id != self.signal.origin().operation_id()
            || scope.mutation.correlation_id != self.signal.origin().operation_id()
            || scope.mutation.expected_revision != revision
            || scope.mutation.participant.as_str() != source.target.participant_id
            || !matches!(&scope.mutation.change,ParticipantChange::Admit{connection,role_override} if connection==&self.connection() && role_override.as_ref().is_none_or(|r|r==self.target.member.default_role()))
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.target.principal,
            )
            || scope.connection_generation.is_some_and(|g| g != 1)
        {
            return Err(ParticipantControlError::Authority);
        }
        Ok(ParticipantAuthority {
            actor: self.target.member.clone(),
            target: self.target.member.clone(),
            endpoint: Some(self.target.endpoint.clone()),
            may_send_audio: true,
            may_receive_audio: true,
            confirmation_required: false,
            confirmed_generation: None,
        })
    }
    pub(crate) fn owner(
        &self,
        read: &InitialTelephoneMediaRead,
    ) -> std::result::Result<MediaOwner, StreamControlError> {
        self.runtime()?;
        let p = self.prepared.get().ok_or(StreamControlError::NotReady)?;
        let codec = self.codec.get().ok_or(StreamControlError::NotReady)?;
        if !read.current()
            || !p.is_current()
            || p.is_private_audio() != self.signal.origin().source().is_private_candidate()
            || read.is_private_admission() != p.is_private_audio()
            || read.source().target.membership_epoch != p.member().epoch()
            || read.source().endpoint_id != p.endpoint().endpoint().as_str()
            || read.endpoint_actor().identity_id != self.target.principal.subject
            || !self
                .context
                .get()
                .and_then(Weak::upgrade)
                .is_some_and(|c| c.negotiated_audio_codec().is_ok_and(|v| v == *codec))
        {
            return Err(StreamControlError::Authority);
        }
        let current = member(
            &self.target.principal,
            read.member(),
            read.conversation_revision(),
        )?;
        let presence = SessionAppearance::admit(
            &current,
            p.appearance().session().clone(),
            Some(crate::canonical_runtime::participant_role(read.role())),
        )
        .map_err(|_| StreamControlError::Authority)?;
        let format = MediaFormat::Encoded {
            name: match codec.name.to_ascii_lowercase().as_str() {
                "g.711-mu" => "pcmu".into(),
                "g.711-a" => "pcma".into(),
                _ => codec.name.clone(),
            },
            clock_rate_hz: codec.clock_rate_hz,
            channels: codec.channels,
            fmtp: codec.fmtp.clone(),
        };
        let caps = EndpointMediaCapabilities {
            send: vec![MediaCapability {
                medium: Medium::Audio,
                formats: vec![format.clone()],
            }],
            receive: vec![MediaCapability {
                medium: Medium::Audio,
                formats: vec![format],
            }],
            max_streams: 1,
            renegotiation: false,
        };
        let mut owner = MediaOwner {
            actor: current.clone(),
            member: current,
            presence,
            endpoint: self.target.endpoint.clone(),
            connection: p.binding().clone(),
            local: caps.clone(),
            remote: caps,
            send_media: if read.may_receive() {
                BTreeSet::from([Medium::Audio])
            } else {
                BTreeSet::new()
            },
            receive_media: if read.role() != "observer" {
                BTreeSet::from([Medium::Audio])
            } else {
                BTreeSet::new()
            },
            may_control: true,
            may_route_source: false,
        };
        if let Some(proof) = self
            .proof
            .try_lock()
            .map_err(|_| StreamControlError::NotReady)?
            .as_ref()
            .filter(|p| p.is_committed())
        {
            owner = proof.current(owner)?;
        }
        Ok(owner)
    }
    fn scope(&self, scope: &MediaScope<'_>) -> std::result::Result<(), StreamControlError> {
        let s = self.signal.origin().source();
        if scope.connection != &self.connection()
            || scope.conversation.as_str() != s.conversation_id
            || scope.session.as_str() != s.session_id
            || !reliable_data::same_credential(scope.principal, &self.target.principal)
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub fn desired() -> DesiredStream {
        DesiredStream {
            logical_id: StreamId::from_string("telephone-audio"),
            medium: Medium::Audio,
            direction: StreamDirection::Both,
        }
    }
}
impl DialInvocation {
    pub fn control(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> std::result::Result<ParticipantControl, ParticipantControlError> {
        self.current(scope)?;
        ParticipantControl::new(
            Box::new(self.clone()),
            self.leg.cancel.clone(),
            self.leg.deadline,
        )
    }
}
impl ParticipantControlLease for DialInvocation {
    fn current(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> std::result::Result<ParticipantAuthority, ParticipantControlError> {
        let runtime = self.leg.runtime().map_err(participant_error)?;
        if let Some(read) = &self.media {
            if !read.current()
                || read.source() != self.leg.signal.origin().source()
                || read.endpoint_actor().identity_id != self.leg.target.principal.subject
                || read.member().participant_id
                    != self.leg.signal.origin().source().target.participant_id
                || read.member().membership_epoch != self.leg.target.member.epoch()
            {
                return Err(ParticipantControlError::Authority);
            }
            let mut authority = self.leg.participant(scope, self.revision)?;
            let current = member(
                &self.leg.target.principal,
                read.member(),
                read.conversation_revision(),
            )
            .map_err(participant_error)?;
            authority.actor = current.clone();
            authority.target = current;
            authority.may_send_audio = read.role() != "observer";
            authority.may_receive_audio = read.may_receive();
            return Ok(authority);
        }
        // These are short metadata checks, not readers retained across ringing.
        let read = runtime
            .media
            .store
            .initial_telephone_signaling_read(&self.leg.signal)
            .map_err(participant_error)?;
        if !read.current()
            || read.runtime_connection_id() != Some(self.leg.signal.runtime_connection_id())
            || read.sip_call_id() != Some(self.leg.signal.sip_call_id())
        {
            return Err(ParticipantControlError::Authority);
        }
        self.leg.participant(scope, self.revision)
    }
}
impl MediaInvocation {
    pub fn control(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> std::result::Result<ParticipantControl, ParticipantControlError> {
        self.current(scope)?;
        ParticipantControl::new(
            Box::new(self.clone()),
            self.leg.cancel.clone(),
            self.leg.deadline,
        )
    }
}
impl ParticipantControlLease for MediaInvocation {
    fn current(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> std::result::Result<ParticipantAuthority, ParticipantControlError> {
        if !self.read.current() {
            return Err(ParticipantControlError::Authority);
        }
        let owner = self.leg.owner(&self.read).map_err(participant_error)?;
        let mut authority = self.leg.participant(scope, self.revision)?;
        authority.actor = owner.member.clone();
        authority.target = owner.member;
        authority.endpoint = Some(owner.endpoint);
        authority.may_send_audio = owner.receive_media.contains(&Medium::Audio);
        authority.may_receive_audio = owner.send_media.contains(&Medium::Audio);
        Ok(authority)
    }
}
struct Lease {
    leg: Weak<Leg>,
    owner: MediaOwner,
    read: Arc<InitialTelephoneMediaRead>,
}
impl MediaAuthorityLease for Lease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(
            self.leg
                .upgrade()
                .ok_or(StreamControlError::Cancelled)?
                .owner(&self.read)?,
        ))
    }
    fn control_prepared(
        &self,
        prepared: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        let leg = self.leg.upgrade().ok_or(StreamControlError::Cancelled)?;
        let owner = prepared.activate_for_commit(leg.owner(&self.read)?)?;
        *leg.proof
            .lock()
            .map_err(|_| StreamControlError::Authority)? = Some(prepared.clone());
        Ok(Box::new(owner))
    }
}
impl MediaPolicy for Leg {
    fn current_participant_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaParticipantSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.receive_native_participant(scope, source)
    }

    fn current(&self, scope: &MediaScope<'_>) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.scope(scope)?;
        let runtime = self.runtime()?;
        let started = std::time::Instant::now();
        let result = self.current_media_read();
        tracing::trace!(target: "connect_service::canonical_packet",
            elapsed_us = started.elapsed().as_micros() as u64,
            publishing = matches!(scope.purpose, MediaPurpose::Publish),
            problem = result.as_ref().err().map(|e| e.code.as_str()),
            "Current initial SIP packet authority");
        let read = Arc::new(result.map_err(crate::canonical_media::packet_authority_error)?);
        if !matches!(scope.purpose, MediaPurpose::Control) && !read.is_connected() {
            return Err(StreamControlError::NotReady);
        }
        if let MediaPurpose::Receive { source } = scope.purpose {
            let native = runtime.media.entry(&source.connection)?;
            let pair = runtime
                .media
                .store
                .try_initial_telephone_native_read(self.origin()?, native.origin.clone())
                .map_err(crate::canonical_media::packet_authority_error)?;
            if !pair.current()
                || !pair.phone().is_connected()
                || !pair.phone().may_receive()
                || pair.native().target().handoff_preparation
                || pair.native().target().publication_suspended
            {
                return Err(StreamControlError::Authority);
            }
            crate::canonical_media::source_matches(source, &native, pair.native().target())?;
            let mut owner = self.owner(pair.phone())?;
            owner.may_route_source = true;
            return Ok(Box::new(PairLease { owner, _read: pair }));
        }
        let owner = self.owner(&read)?;
        Ok(Box::new(Lease {
            leg: self.self_ref.clone(),
            owner,
            read,
        }))
    }
    fn current_control(
        &self,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ControlResult<MediaControl> {
        self.scope(scope)?;
        if let Some(i) =
            held.downcast_ref::<crate::canonical_media::consultation::AudienceInvocation>()
        {
            let runtime = self.runtime()?;
            if !std::ptr::eq(runtime.media.as_ref(), i.policy.as_ref()) {
                return Err(StreamControlError::Authority);
            }
            return i.initial_control(self, scope, intent);
        }
        let i = held
            .downcast_ref::<MediaInvocation>()
            .filter(|i| std::ptr::eq(self, Arc::as_ptr(&i.leg)))
            .ok_or(StreamControlError::Authority)?;
        let allowed = match intent {
            MediaControlIntent::Initialize => true,
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } => *expected == 0 && **desired == [Self::desired()] && replace.is_empty(),
            MediaControlIntent::Gate {
                expected,
                streams,
                action,
            } => {
                i.read.is_private_admission()
                    && *expected == 1
                    && **streams == BTreeSet::from([Self::desired().logical_id])
                    && *action == StreamGateAction::Mute
            }
            MediaControlIntent::PreparePrivateAudio {
                operation,
                expected,
            } => {
                i.read.is_private_admission()
                    && *expected == 2
                    && *operation == self.signal.origin().operation_id()
            }
            _ => false,
        };
        if !allowed || !i.read.current() {
            return Err(StreamControlError::Authority);
        }
        let owner = self.owner(&i.read)?;
        MediaControl::new(
            Box::new(Lease {
                leg: Arc::downgrade(&i.leg),
                owner,
                read: i.read.clone(),
            }),
            self.cancel.clone(),
            self.deadline,
        )
    }
}
pub(crate) struct Ready {
    pub leg: Arc<Leg>,
}
impl connect_store::InitialTelephoneRuntimeObservation for Ready {
    fn observe(&self) -> connect_protocol::Result<connect_store::InitialTelephoneReadyObservation> {
        self.observe_mode(false)
    }
}
impl connect_store::PrivateInitialTelephoneRuntimeObservation for Ready {
    fn observe_private(
        &self,
    ) -> connect_protocol::Result<connect_store::InitialTelephoneReadyObservation> {
        self.observe_mode(true)
    }
}
impl Ready {
    fn observe_mode(
        &self,
        private: bool,
    ) -> connect_protocol::Result<connect_store::InitialTelephoneReadyObservation> {
        let p = self.leg.prepared.get().ok_or_else(Error::forbidden)?;
        let admission = self.leg.admitted.get().ok_or_else(Error::forbidden)?;
        if p.is_private_audio() != private
            || self.leg.signal.origin().source().is_private_candidate() != private
            || admission.session != *p.appearance().session()
            || admission.participant != *p.member().participant()
            || admission.membership_epoch != p.member().epoch()
            || admission.operation_id != self.leg.signal.origin().operation_id()
            || admission.correlation_id != self.leg.signal.origin().operation_id()
            || admission.admitted != [self.leg.connection()]
            || !matches!(
                admission.kind,
                ParticipantObservationKind::Joined | ParticipantObservationKind::ConnectionAdded
            )
            || !p.is_current()
            || self.leg.cancel.is_cancelled()
            || !self
                .leg
                .proof
                .try_lock()
                .map_err(|_| Error::storage())?
                .as_ref()
                .is_some_and(|p| p.is_committed())
        {
            return Err(Error::forbidden());
        }
        let controller = self
            .leg
            .controller
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(Error::forbidden)?;
        if private {
            let ports = controller.active_bindings();
            let exact = self.leg.binding.get().is_some_and(|original| {
                ports.len() == 1
                    && ports.first().is_some_and(|current| {
                        current.negotiated == original.negotiated
                            && current.transport_id == original.transport_id
                            && current.stream_local_id == original.stream_local_id
                            && current.generation == original.generation
                            && current.connection_generation == original.connection_generation
                            && current.source_ready == original.source_ready
                            && current.sink_ready == original.sink_ready
                    })
            });
            if controller.revision() != 2 || !controller.is_bound_to_outbound(p) || !exact {
                return Err(Error::forbidden());
            }
        }
        Ok(connect_store::InitialTelephoneReadyObservation {
            runtime_connection_id: self.leg.signal.runtime_connection_id().into(),
            call_id: self.leg.signal.sip_call_id().into(),
            participant_id: p.member().participant().as_str().into(),
            membership_epoch: p.member().epoch(),
            generation: p.binding().generation(),
            stream_revision: controller.revision(),
            participant_revision: admission.revision,
            streams: vec![Leg::desired().logical_id.as_str().into()],
        })
    }
}
struct PairLease {
    owner: MediaOwner,
    _read: connect_store::InitialTelephoneNativeRead,
}
impl MediaAuthorityLease for PairLease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
impl Leg {
    pub(crate) fn validate_participant_source(
        &self,
        source: &MediaParticipantSource,
        read: &InitialTelephoneMediaRead,
    ) -> ControlResult<()> {
        let owner = self.owner(read)?;
        if source.conversation != *owner.member.conversation()
            || source.session != *owner.presence.session()
            || source.participant != *owner.member.participant()
            || source.membership_epoch != owner.member.epoch()
            || source.connection != self.connection()
            || source.identity != *owner.member.identity()
            || !reliable_data::same_credential(&source.principal, &self.target.principal)
            || !read.is_connected()
        {
            return Err(StreamControlError::Authority);
        }
        if source.medium != Medium::Audio {
            return Err(StreamControlError::Unsupported);
        }
        Ok(())
    }
    pub(crate) fn native_participant_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaParticipantSource,
        native: &Arc<crate::canonical_media::Connection>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let runtime = self.runtime()?;
        let read = runtime
            .media
            .store
            .try_initial_telephone_native_read(self.origin()?, native.origin.clone())
            .map_err(|_| StreamControlError::Authority)?;
        self.validate_participant_source(source, read.phone())?;
        if !read.current()
            || !read.native().permits_subscriptions()
            || read.native().target().handoff_preparation
            || read.native().target().publication_suspended
        {
            return Err(StreamControlError::Authority);
        }
        let owner = native.owner(read.native().target(), scope.principal, true)?;
        let owner = native
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        if !owner.send_media.contains(&Medium::Audio) {
            return Err(StreamControlError::Authority);
        }
        Ok(Box::new(PairLease { owner, _read: read }))
    }
    pub(crate) fn physical_source_matches(
        &self,
        source: &MediaSource,
        read: &InitialTelephoneMediaRead,
    ) -> ControlResult<()> {
        if source.connection != self.connection() || source.logical_id != Self::desired().logical_id
        {
            return Err(StreamControlError::Authority);
        }
        let p = self.prepared.get().ok_or(StreamControlError::NotReady)?;
        let binding = self.binding.get().ok_or(StreamControlError::NotReady)?;
        if source.transport_id != binding.transport_id
            || source.stream_generation != binding.generation
        {
            return Err(StreamControlError::Authority);
        }
        if source.participant != *p.member().participant()
            || source.membership_epoch != p.member().epoch()
            || source.connection_generation != p.binding().generation()
        {
            return Err(StreamControlError::Authority);
        }
        if !read.is_connected() {
            return Err(StreamControlError::NotReady);
        }
        let _ = self.owner(read)?;
        Ok(())
    }
    pub(crate) fn native_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaSource,
        native: &Arc<crate::canonical_media::Connection>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let runtime = self.runtime()?;
        let started = std::time::Instant::now();
        let result = runtime
            .media
            .store
            .try_initial_telephone_native_read(self.origin()?, native.origin.clone());
        tracing::trace!(target: "connect_service::canonical_packet",
            elapsed_us = started.elapsed().as_micros() as u64,
            problem = result.as_ref().err().map(|e| e.code.as_str()),
            "Current initial SIP to native packet authority");
        let mut stage = "store-read";
        let result = (|| -> ControlResult<Box<dyn MediaAuthorityLease>> {
            let read = result.map_err(crate::canonical_media::packet_authority_error)?;
            stage = "pair-current";
            if !read.current()
                || !read.phone().is_connected()
                || !read.native().permits_subscriptions()
                || read.native().target().handoff_preparation
                || read.native().target().publication_suspended
            {
                return Err(StreamControlError::Authority);
            }
            stage = "source-binding";
            self.physical_source_matches(source, read.phone())?;
            stage = "sip-owner";
            let _ = self.owner(read.phone())?;
            stage = "native-activation";
            let activation = native.activation()?.ok_or(StreamControlError::NotReady)?;
            stage = "native-owner";
            let owner = native.owner(read.native().target(), scope.principal, true)?;
            stage = "native-current";
            let owner = activation.current(owner)?;
            stage = "granted";
            Ok(Box::new(PairLease { owner, _read: read }))
        })();
        tracing::trace!(target: "connect_service::canonical_packet",
            connection = native.origin.connection_id(),
            elapsed_us = started.elapsed().as_micros() as u64,
            stage,
            problem = ?result.as_ref().err(),
            "Initial SIP to native packet grant");
        result
    }
    fn receive_native_participant(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaParticipantSource,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.scope(scope)?;
        let runtime = self.runtime()?;
        let native = runtime.media.entry(&source.connection)?;
        let read = runtime
            .media
            .store
            .try_initial_telephone_native_read(self.origin()?, native.origin.clone())
            .map_err(|_| StreamControlError::Authority)?;
        if !read.current()
            || !read.phone().is_connected()
            || !read.phone().may_receive()
            || read.native().target().handoff_preparation
            || read.native().target().publication_suspended
        {
            return Err(StreamControlError::Authority);
        }
        if source.conversation.as_str() != read.native().target().session.session.conversation_id
            || source.session.as_str() != read.native().target().session.session.id
            || source.participant.as_str() != read.native().target().connection.participant_id
            || source.membership_epoch != read.native().target().connection.membership_epoch
            || source.medium != Medium::Audio
        {
            return Err(StreamControlError::Authority);
        }
        let source_owner = native
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(native.owner(read.native().target(), &source.principal, false)?)?;
        if source.identity != *source_owner.member.identity()
            || !source_owner.receive_media.contains(&Medium::Audio)
        {
            return Err(StreamControlError::Authority);
        }
        let mut owner = self.owner(read.phone())?;
        owner.may_route_source = true;
        Ok(Box::new(PairLease { owner, _read: read }))
    }
}
