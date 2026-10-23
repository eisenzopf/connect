//! Fresh operator and original physical grants for exact managed preparation.
use super::*;
use connect_store::NativeReplacementPreparationRead;
use rvoip_core::{
    orchestrator::participant_control::{
        ControlResult as ParticipantResult, ParticipantControlError,
        ParticipantReplacementAuthority, ParticipantReplacementControl,
        ParticipantReplacementLease, ParticipantReplacementScope, ReplacementPhase,
    },
    participant_transitions::ParticipantReplacementMutation,
};

#[derive(Clone)]
pub(crate) struct ReplacementPreparationInvocation {
    pub policy: Arc<Policy>,
    pub read: Arc<NativeReplacementPreparationRead>,
    pub principal: AuthenticatedPrincipal,
    pub intent: ParticipantReplacementMutation,
    pub deadline: tokio::time::Instant,
    pub cancel: tokio_util::sync::CancellationToken,
    pub telephone: Option<Arc<crate::telephone_replacement::authority::Target>>,
}
impl ParticipantReplacementLease for ReplacementPreparationInvocation {
    fn current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ParticipantResult<ParticipantReplacementAuthority> {
        let deny = || ParticipantControlError::Authority;
        let ctx = self.read.context();
        if scope.phase != ReplacementPhase::Prepare
            || scope.plan.is_some()
            || !self.read.current()
            || self.cancel.is_cancelled()
            || scope.intent != &self.intent
            || scope.conversation.as_str() != ctx.conversation_id
            || scope.session.as_str() != ctx.session_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &self.principal,
                scope.principal,
            )
            || scope.intent.operation_id != ctx.operation_record_id
            || scope.intent.correlation_id != ctx.operation_record_id
            || scope.intent.source.as_str() != ctx.source_runtime_connection_id
            || scope.intent.retained.as_str() != ctx.retained_runtime_connection_id
            || scope.intent.participant.as_str() != ctx.participant_id
            || scope.intent.endpoint.as_str() != ctx.target_endpoint_id
            || scope.intent.device.as_str() != ctx.target_device_id
        {
            return Err(deny());
        }
        let old = self.policy.replacement_source_owner(&self.read)?;
        if let Some(target) = &self.telephone {
            if ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Telephone
                || target.principal.subject != ctx.target_route_owner_id
                || target.member.identity().as_str() != ctx.identity_id
                || target.member.participant() != old.member.participant()
                || target.member.epoch() != old.member.epoch()
                || target.endpoint.transport() != Transport::Sip
                || target.endpoint.endpoint().as_str() != ctx.target_endpoint_id
                || target.endpoint.device().as_str() != ctx.target_device_id
            {
                return Err(deny());
            }
            return Ok(ParticipantReplacementAuthority {
                actor: current_member(&self.principal, self.read.caller()).map_err(|_| deny())?,
                target: target.member.clone(),
                endpoint: target.endpoint.clone(),
                may_send_audio: ctx.role != "observer"
                    && old.receive_media.contains(&Medium::Audio),
                may_receive_audio: old.send_media.contains(&Medium::Audio),
            });
        }
        if ctx.target_transport != connect_protocol::provisioning::EndpointTransport::Quic {
            return Err(ParticipantControlError::Unsupported);
        }
        let principal = self.policy.replacement_source_principal(&self.read)?;
        let binding = IdentityBinding::from_authenticated(
            &principal,
            IdentityId::from_string(ctx.identity_id.clone()),
            chrono::Utc::now(),
        )
        .map_err(|_| deny())?;
        let mut endpoint = EndpointBinding::provision(
            binding.clone(),
            EndpointId::from_string(ctx.target_endpoint_id.clone()),
            DeviceId::from_string(ctx.target_device_id.clone()),
            Transport::Quic,
            ctx.target_address.clone(),
        )
        .map_err(|_| deny())?;
        endpoint
            .verify(&binding, "connect-store".into(), chrono::Utc::now())
            .map_err(|_| deny())?;
        Ok(ParticipantReplacementAuthority {
            actor: current_member(&self.principal, self.read.caller()).map_err(|_| deny())?,
            target: old.member,
            endpoint,
            may_send_audio: ctx.role != "observer" && old.receive_media.contains(&Medium::Audio),
            may_receive_audio: old.send_media.contains(&Medium::Audio),
        })
    }
}
impl Policy {
    pub(crate) fn replacement_telephone_source(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<Option<Arc<crate::telephone_replacement::media::Policy>>> {
        let deny = || ParticipantControlError::Authority;
        let Some(phone) = read.source().telephone() else {
            return Ok(None);
        };
        if !read.current() {
            return Err(deny());
        }
        let id = ConnectionId::from_string(read.context().source_runtime_connection_id.clone());
        let source = self
            .telephone_targets
            .lock()
            .map_err(|_| deny())?
            .get(&id)
            .and_then(Weak::upgrade)
            .ok_or_else(deny)?;
        let _ = source.established_owner(phone).map_err(|_| deny())?;
        Ok(Some(source))
    }
    pub(crate) fn replacement_source_principal(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<AuthenticatedPrincipal> {
        let deny = || ParticipantControlError::Authority;
        if !read.current() {
            return Err(deny());
        }
        let id = ConnectionId::from_string(read.context().source_runtime_connection_id.clone());
        match read.source() {
            connect_store::NativeReplacementSourceRead::Native(_) => {
                Ok(self.entry(&id).map_err(|_| deny())?.principal.clone())
            }
            connect_store::NativeReplacementSourceRead::Telephone(phone, _) => {
                let source = self
                    .telephone_targets
                    .lock()
                    .map_err(|_| deny())?
                    .get(&id)
                    .and_then(Weak::upgrade)
                    .ok_or_else(deny)?;
                let _ = source.established_owner(phone).map_err(|_| deny())?;
                source.principal().map_err(|_| deny())
            }
        }
    }
    pub(crate) fn replacement_source_owner(
        &self,
        read: &NativeReplacementPreparationRead,
    ) -> ParticipantResult<MediaOwner> {
        let deny = || ParticipantControlError::Authority;
        let ctx = read.context();
        if !read.current() {
            return Err(deny());
        }
        let retained = self
            .entry(&ConnectionId::from_string(
                ctx.retained_runtime_connection_id.clone(),
            ))
            .map_err(|_| deny())?;
        let old = match read.source() {
            connect_store::NativeReplacementSourceRead::Native(source_read) => {
                let source = self
                    .entry(&ConnectionId::from_string(
                        ctx.source_runtime_connection_id.clone(),
                    ))
                    .map_err(|_| deny())?;
                let owner = source
                    .owner(source_read.target(), &source.principal, false)
                    .map_err(|_| deny())?;
                source
                    .activation()
                    .map_err(|_| deny())?
                    .ok_or_else(deny)?
                    .current(owner)
                    .map_err(|_| deny())?
            }
            connect_store::NativeReplacementSourceRead::Telephone(phone, _) => {
                let source = self
                    .telephone_targets
                    .lock()
                    .map_err(|_| deny())?
                    .get(&ConnectionId::from_string(
                        ctx.source_runtime_connection_id.clone(),
                    ))
                    .and_then(Weak::upgrade)
                    .ok_or_else(deny)?;
                source.established_owner(phone).map_err(|_| deny())?
            }
        };
        let remote = retained
            .owner(read.retained().target(), &retained.principal, false)
            .map_err(|_| deny())?;
        retained
            .activation()
            .map_err(|_| deny())?
            .ok_or_else(deny)?
            .current(remote)
            .map_err(|_| deny())?;
        if old.member.participant().as_str() != ctx.participant_id
            || old.member.epoch() != ctx.membership_epoch
        {
            return Err(deny());
        }
        Ok(old)
    }
}
impl Policy {
    pub(super) fn replacement_preparation_control(
        &self,
        scope: &ParticipantReplacementScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<ParticipantReplacementControl> {
        let invocation = held
            .downcast_ref::<ReplacementPreparationInvocation>()
            .ok_or(ParticipantControlError::Authority)?;
        if !std::ptr::eq(self, Arc::as_ptr(&invocation.policy)) {
            return Err(ParticipantControlError::Authority);
        }
        let _ = invocation.current(scope)?;
        // Current() still enforces the five-second reader. Its upper deadline
        // bounds the owned resource ticket without retaining that reader.
        ParticipantReplacementControl::new(
            Box::new(invocation.clone()),
            invocation.cancel.clone(),
            invocation.deadline,
        )
    }
}

impl super::Policy {
    pub(crate) fn adopt_replacement(
        self: &Arc<Self>,
        target: &Arc<crate::canonical_replacement::TargetPolicy>,
        origin: Arc<connect_store::NativeMediaOrigin>,
        binding: rvoip_core::relationships::ConnectionBinding,
        read: &connect_store::NativeMediaRead,
    ) -> ControlResult<Arc<super::Connection>> {
        if !std::ptr::eq(self.as_ref(), Arc::as_ptr(&target.base))
            || target
                .root
                .upgrade()
                .is_none_or(|r| !r.handle.is_committed())
            || !binding.same_incarnation(target.pending.binding())
            || binding.state() != rvoip_core::relationships::BindingState::Active
            || origin.generation() != binding.generation()
            || origin.runtime_connection_id() != binding.connection().as_str()
            || read.target().runtime_connection_id != origin.runtime_connection_id()
            || !read.current()
        {
            return Err(StreamControlError::Authority);
        }
        let activation = target
            .activation
            .get()
            .cloned()
            .ok_or(StreamControlError::NotReady)?;
        let entry = Arc::new(super::Connection {
            domain: Arc::downgrade(self),
            origin,
            principal: target.principal.clone(),
            profile: target.profile.clone(),
            plan: target.plan.clone(),
            negotiated_remote: std::sync::Mutex::new(target.plan.remote_capabilities().clone()),
            activation: std::sync::Mutex::new(Some(activation.clone())),
            binding: std::sync::Mutex::new(Some(binding)),
            preparation: Default::default(),
        });
        activation.current(entry.owner(read.target(), &entry.principal, false)?)?;
        let id = entry.origin.runtime_connection_id().to_owned();
        let mut entries = self.entries.lock().map_err(super::denied)?;
        if let Some(existing) = entries.get(&ConnectionId::from_string(id.clone())) {
            if existing.origin.offer_record_id() != entry.origin.offer_record_id() {
                return Err(StreamControlError::Authority);
            }
            return Ok(existing.clone());
        }
        // The actual committed candidate inherits the old route's slot.
        entries.remove(&target.pending.intent().source);
        if entries.len() >= self.capacity {
            return Err(StreamControlError::Authority);
        }
        let mut provisional = self.replacement_targets.lock().map_err(super::denied)?;
        if provisional
            .get(target.pending.binding().connection())
            .and_then(std::sync::Weak::upgrade)
            .is_none_or(|p| !Arc::ptr_eq(&p, target))
        {
            return Err(StreamControlError::Authority);
        }
        entries.insert(ConnectionId::from_string(id), entry.clone());
        provisional.remove(target.pending.binding().connection());
        Ok(entry)
    }
}
