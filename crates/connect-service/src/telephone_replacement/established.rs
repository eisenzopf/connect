//! Current established SIP owners and mixed native/telephone packet grants.
use super::*;
use connect_store::{TelephoneMediaOrigin, TelephoneMediaRead, TelephoneNativeMediaRead};
use rvoip_core::{ids::*, relationships::*, staged_streams::*, stream_lifecycle::Medium};
pub(super) struct Established {
    pub origin: Arc<TelephoneMediaOrigin>,
    pub owner: MediaOwner,
}
impl media::Policy {
    /// Actual active replacement binding only; Store named signal authority is
    /// acquired separately for each write and never converted into RTP rights.
    pub(crate) fn established_signal_current(&self) -> bool {
        self.established.get().is_some_and(|established| {
            established
                .owner
                .connection
                .same_incarnation(self.pending.binding())
                && established.owner.connection.state() == BindingState::Active
        }) && self
            .runtime
            .upgrade()
            .is_some_and(|runtime| !runtime.stop.is_cancelled())
            && self.physical_current()
    }

    pub(crate) fn established_origin(&self) -> Option<Arc<TelephoneMediaOrigin>> {
        self.established.get().map(|e| e.origin.clone())
    }
    pub(super) fn establish(
        &self,
        origin: Arc<TelephoneMediaOrigin>,
        owner: MediaOwner,
    ) -> ControlResult<()> {
        if self.root.upgrade().is_none_or(|r| !r.handle.is_committed())
            || !owner.connection.same_incarnation(self.pending.binding())
            || owner.connection.state() != BindingState::Active
            || origin.snapshot().runtime_connection_id != owner.connection.connection().as_str()
            || origin.snapshot().generation != owner.connection.generation()
        {
            return Err(StreamControlError::Authority);
        }
        self.established
            .set(Arc::new(Established { origin, owner }))
            .map_err(|_| StreamControlError::Authority)
    }
    pub(crate) fn established_owner(&self, read: &TelephoneMediaRead) -> ControlResult<MediaOwner> {
        let established = self.established.get().ok_or(StreamControlError::NotReady)?;
        let s = read.snapshot();
        if !read.current()
            || established.origin.record_id()
                != s.expected_movement["operation_id"].as_str().unwrap_or("")
            || !self.physical_current()
            || s.runtime_connection_id != self.pending.intent().replacement.as_str()
            || s.generation != self.pending.generation()
        {
            return Err(StreamControlError::Authority);
        }
        let principal = self.principal()?;
        if principal.subject != read.route_actor().identity_id {
            return Err(StreamControlError::Authority);
        }
        let id = IdentityBinding::from_authenticated(
            &principal,
            IdentityId::from_string(read.member().identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(|_| StreamControlError::Authority)?;
        let member = Membership::from_current(
            id.clone(),
            ConversationId::from_string(s.conversation_id.clone()),
            ParticipantId::from_string(s.participant_id.clone()),
            match read.member().kind {
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
            crate::canonical_runtime::participant_role(&read.member().role),
            s.membership_epoch,
            read.conversation_revision(),
            read.member().active,
        )
        .map_err(|_| StreamControlError::Authority)?;
        let mut endpoint = EndpointBinding::provision(
            id.clone(),
            EndpointId::from_string(s.endpoint_id.clone()),
            DeviceId::from_string(s.device_id.clone()),
            rvoip_core::Transport::Sip,
            s.destination.clone(),
        )
        .map_err(|_| StreamControlError::Authority)?;
        endpoint
            .verify(
                &id,
                "connect-store-telephone-route".into(),
                chrono::Utc::now(),
            )
            .map_err(|_| StreamControlError::Authority)?;
        let presence = SessionAppearance::admit(
            &member,
            SessionId::from_string(s.session_id.clone()),
            Some(crate::canonical_runtime::participant_role(read.role())),
        )
        .map_err(|_| StreamControlError::Authority)?;
        let mut owner = established.owner.clone();
        owner.actor = member.clone();
        owner.member = member;
        owner.endpoint = endpoint;
        owner.presence = presence;
        owner.may_route_source = false;
        // A durable Observer change must stop publication immediately, even
        // before the disposable mixer role catches up with the journal.
        if read.role() == "observer" {
            owner.receive_media.remove(&Medium::Audio);
        }
        self.proof
            .try_lock()
            .map_err(|_| StreamControlError::NotReady)?
            .as_ref()
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
    pub(crate) fn source_matches(&self, source: &MediaSource) -> ControlResult<()> {
        let b = self.binding.get().ok_or(StreamControlError::NotReady)?;
        let leg = self.leg.upgrade().ok_or(StreamControlError::Cancelled)?;
        if source.connection != self.pending.intent().replacement
            || source.session != *self.pending.appearance().session()
            || source.conversation != *leg.target.member.conversation()
            || source.participant != *self.pending.appearance().participant()
            || source.membership_epoch != self.pending.appearance().membership_epoch()
            || source.connection_generation != b.connection_generation
            || source.logical_id != b.negotiated.requested.logical_id
            || source.transport_id != b.transport_id
            || source.stream_generation != b.generation
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub(crate) fn participant_source_matches(
        &self,
        source: &MediaParticipantSource,
        read: &TelephoneMediaRead,
    ) -> ControlResult<()> {
        let owner = self.established_owner(read)?;
        if &source.conversation != owner.member.conversation()
            || &source.session != owner.presence.session()
            || &source.connection != owner.connection.connection()
            || &source.participant != owner.member.participant()
            || source.membership_epoch != owner.member.epoch()
            || &source.identity != owner.member.identity()
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &source.principal,
                &self.principal()?,
            )
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub(super) fn established_scope(&self, scope: &MediaScope<'_>) -> ControlResult<()> {
        let origin = self
            .established_origin()
            .ok_or(StreamControlError::NotReady)?;
        let s = origin.snapshot();
        if scope.connection.as_str() != s.runtime_connection_id
            || scope.session.as_str() != s.session_id
            || scope.conversation.as_str() != s.conversation_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.principal()?,
            )
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    pub(super) fn established_current(
        &self,
        scope: &MediaScope<'_>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        self.established_scope(scope)?;
        let base = self.base()?;
        let origin = self
            .established_origin()
            .ok_or(StreamControlError::NotReady)?;
        if let MediaPurpose::Receive { source } = scope.purpose {
            let entry = base.entry(&source.connection)?;
            let read = base
                .store
                .try_telephone_native_media_read(origin, entry.origin.clone())
                .map_err(|_| StreamControlError::Authority)?;
            if !read.permits_telephone_receive() {
                return Err(StreamControlError::Authority);
            }
            crate::canonical_media::source_matches(source, &entry, read.native().target())?;
            let mut owner = self.established_owner(read.telephone())?;
            owner.may_route_source = true;
            return Ok(Box::new(FreshPair { owner, _read: read }));
        }
        let read = base
            .store
            .try_telephone_media_read(origin)
            .map_err(|_| StreamControlError::Authority)?;
        let owner = self.established_owner(&read)?;
        Ok(Box::new(Fresh { owner, _read: read }))
    }
    pub(crate) fn native_receive(
        &self,
        scope: &MediaScope<'_>,
        source: &MediaSource,
        native: &Arc<crate::canonical_media::Connection>,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        let base = self.base()?;
        let origin = self
            .established_origin()
            .ok_or(StreamControlError::NotReady)?;
        let read = base
            .store
            .try_telephone_native_media_read(origin, native.origin.clone())
            .map_err(|_| StreamControlError::Authority)?;
        if !read.permits_native_receive() {
            return Err(StreamControlError::Authority);
        }
        self.source_matches(source)?;
        let _source = self.established_owner(read.telephone())?;
        let owner = native.owner(read.native().target(), scope.principal, true)?;
        let owner = native
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)?;
        Ok(Box::new(FreshPair { owner, _read: read }))
    }
}
struct Fresh {
    owner: MediaOwner,
    _read: TelephoneMediaRead,
}
impl MediaAuthorityLease for Fresh {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
struct FreshPair {
    owner: MediaOwner,
    _read: TelephoneNativeMediaRead,
}
impl MediaAuthorityLease for FreshPair {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
}
