//! Initial managed admission uses the same exact native invocation and Store fence.
use super::*;
use rvoip_core::{
    orchestrator::participant_control::{
        ControlResult as ParticipantResult, ParticipantAuthority, ParticipantAuthorityLease,
        ParticipantControl, ParticipantControlError, ParticipantControlLease, ParticipantPolicy,
        ParticipantScope,
    },
    participant_transitions::ParticipantChange,
};
fn authority(
    entry: &Connection,
    scope: &ParticipantScope<'_>,
    read: &NativeMediaRead,
) -> ParticipantResult<ParticipantAuthority> {
    let ParticipantChange::Admit {
        connection,
        role_override,
    } = &scope.mutation.change
    else {
        return Err(ParticipantControlError::Unsupported);
    };
    let view = read.target();
    let requested_role = role_override
        .clone()
        .unwrap_or_else(|| crate::canonical_runtime::participant_role(&view.session.actor.role));
    if !read.current()
        || connection.as_str() != entry.origin.runtime_connection_id()
        || scope.conversation.as_str() != entry.origin.conversation_id()
        || scope.session.as_str() != entry.origin.session_id()
        || scope.mutation.participant.as_str() != view.connection.participant_id
        || requested_role != crate::canonical_runtime::participant_role(&view.presence_role)
        || scope
            .connection_generation
            .is_some_and(|g| g != view.connection.generation)
    {
        return Err(ParticipantControlError::Authority);
    }
    let owner = entry
        .owner(view, scope.principal, false)
        .map_err(|_| ParticipantControlError::Authority)?;
    let activation = entry
        .activation()
        .map_err(|_| ParticipantControlError::Authority)?
        .ok_or(ParticipantControlError::Authority)?;
    let owner = activation
        .current(owner)
        .map_err(|_| ParticipantControlError::Authority)?;
    let generation = activation
        .confirmed_generation()
        .filter(|g| *g == view.connection.generation)
        .ok_or(ParticipantControlError::Authority)?;
    Ok(ParticipantAuthority {
        actor: owner.actor.clone(),
        target: owner.member,
        endpoint: Some(owner.endpoint),
        may_send_audio: owner.receive_media.contains(&Medium::Audio),
        may_receive_audio: owner.send_media.contains(&Medium::Audio),
        confirmation_required: true,
        confirmed_generation: Some(generation),
    })
}
struct Current {
    authority: ParticipantAuthority,
    _read: NativeMediaRead,
}
impl ParticipantAuthorityLease for Current {
    fn authority(&self) -> &ParticipantAuthority {
        &self.authority
    }
}
struct Held(Arc<Invocation>);
impl ParticipantControlLease for Held {
    fn current(&self, scope: &ParticipantScope<'_>) -> ParticipantResult<ParticipantAuthority> {
        let i = &self.0;
        let a = &i.authorized;
        if scope.mutation.expected_revision != i.participant_revision
            || if i.read.target().private_consultation.is_some()
                || i.read.target().handoff_preparation
            {
                i.connection.origin.offer_record_id() != scope.mutation.operation_id
            } else {
                a.envelope().scope(Scope::Operation) != Some(scope.mutation.operation_id.as_str())
            }
            || a.envelope().id() != scope.mutation.correlation_id
        {
            return Err(ParticipantControlError::Authority);
        }
        let read = i
            .policy
            .held_read(i, None)
            .map_err(|_| ParticipantControlError::Authority)?;
        authority(&i.connection, scope, &read)
    }
}
impl ParticipantPolicy for Policy {
    fn current_replacement_control(
        &self,
        scope: &rvoip_core::orchestrator::participant_control::ParticipantReplacementScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<
        rvoip_core::orchestrator::participant_control::ParticipantReplacementControl,
    > {
        if let Some(i) =
            held.downcast_ref::<crate::telephone_replacement::continuation::Invocation>()
        {
            if !std::ptr::eq(self, Arc::as_ptr(&i.runtime.media)) {
                return Err(ParticipantControlError::Authority);
            }
            return i.role_control(scope);
        }
        if let Some(i) =
            held.downcast_ref::<crate::canonical_replacement::continuation::Invocation>()
        {
            if !std::ptr::eq(self, Arc::as_ptr(&i.policy.base)) {
                return Err(ParticipantControlError::Authority);
            }
            return i.role_control(scope);
        }
        if let Some(i) = held.downcast_ref::<crate::telephone_replacement::control::Invocation>() {
            if !std::ptr::eq(self, Arc::as_ptr(&i.runtime.media)) {
                return Err(ParticipantControlError::Authority);
            }
            return i.control(scope);
        }
        self.replacement_preparation_control(scope, held)
    }
    fn current_handoff_control(
        &self,
        scope: &rvoip_core::orchestrator::participant_control::ParticipantHandoffScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<rvoip_core::orchestrator::participant_control::ParticipantHandoffControl>
    {
        self.handoff_control(scope, held)
    }
    fn current(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> ParticipantResult<Box<dyn ParticipantAuthorityLease + '_>> {
        let ParticipantChange::Admit { connection, .. } = &scope.mutation.change else {
            return Err(ParticipantControlError::Unsupported);
        };
        let entry = self
            .entry(connection)
            .map_err(|_| ParticipantControlError::Authority)?;
        let read = self
            .store
            .try_native_media_read(&entry.origin, None)
            .map_err(|_| ParticipantControlError::Authority)?;
        let authority = authority(&entry, scope, &read)?;
        Ok(Box::new(Current {
            authority,
            _read: read,
        }))
    }
    fn current_control(
        &self,
        scope: &ParticipantScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<ParticipantControl> {
        if let Some(i) = held.downcast_ref::<crate::initial_telephone::policy::DialInvocation>() {
            if !std::ptr::eq(
                self,
                Arc::as_ptr(
                    &i.leg
                        .runtime()
                        .map_err(|_| ParticipantControlError::Authority)?
                        .media,
                ),
            ) {
                return Err(ParticipantControlError::Authority);
            }
            return i.control(scope);
        }
        if let Some(i) = held.downcast_ref::<crate::initial_telephone::policy::MediaInvocation>() {
            if !std::ptr::eq(
                self,
                Arc::as_ptr(
                    &i.leg
                        .runtime()
                        .map_err(|_| ParticipantControlError::Authority)?
                        .media,
                ),
            ) {
                return Err(ParticipantControlError::Authority);
            }
            return i.control(scope);
        }
        if let Some(role) = held.downcast_ref::<super::roles::Projection>() {
            return role.control(self, scope);
        }
        let i = held
            .downcast_ref::<Invocation>()
            .ok_or(ParticipantControlError::Authority)?;
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy)) {
            return Err(ParticipantControlError::Authority);
        }
        let lease = Held(Arc::new(i.clone()));
        let _ = lease.current(scope)?;
        ParticipantControl::new(
            Box::new(lease),
            i.authorized.invocation_cancellation(),
            tokio::time::Instant::now() + Duration::from_secs(30),
        )
    }
}
