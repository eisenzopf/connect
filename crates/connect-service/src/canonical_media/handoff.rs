//! Exact managed handoff approval under current caller and physical peer facts.
use super::*;
use connect_store::NativeHandoffRead;
use rvoip_core::orchestrator::participant_control::{
    ControlResult as ParticipantResult, ManagedAudioHandoffPlan, ParticipantControlError,
    ParticipantHandoffAuthority, ParticipantHandoffControl, ParticipantHandoffLease,
    ParticipantHandoffScope,
};
#[derive(Clone)]
pub(crate) struct HandoffInvocation {
    pub policy: Arc<Policy>,
    pub plan: ManagedAudioHandoffPlan,
    pub read: Arc<NativeHandoffRead>,
    pub principal: AuthenticatedPrincipal,
    pub cancel: tokio_util::sync::CancellationToken,
}
impl ParticipantHandoffLease for HandoffInvocation {
    fn current(
        &self,
        scope: &ParticipantHandoffScope<'_>,
    ) -> ParticipantResult<ParticipantHandoffAuthority> {
        if !self.read.current()
            || self.cancel.is_cancelled()
            || !self.plan.same_intent(scope.plan)
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                &self.principal,
                scope.principal,
            )
            || scope.conversation.as_str() != self.read.context().conversation_id
            || scope.session.as_str() != self.read.context().session_id
        {
            return Err(ParticipantControlError::Authority);
        }
        let request = self.plan.media().request();
        if request.operation_id != self.read.context().operation_record_id {
            return Err(ParticipantControlError::Authority);
        }
        let outgoing = self
            .policy
            .entry(&request.old)
            .map_err(|_| ParticipantControlError::Authority)?;
        let incoming = self
            .policy
            .entry(&request.admitted)
            .map_err(|_| ParticipantControlError::Authority)?;
        let resolve = |entry: &Connection| -> ParticipantResult<MediaOwner> {
            let read = self
                .read
                .media(entry.origin.connection_id(), None)
                .ok_or(ParticipantControlError::Authority)?;
            let owner = entry
                .owner(read.target(), &entry.principal, false)
                .map_err(|_| ParticipantControlError::Authority)?;
            entry
                .activation()
                .map_err(|_| ParticipantControlError::Authority)?
                .ok_or(ParticipantControlError::Authority)?
                .current(owner)
                .map_err(|_| ParticipantControlError::Authority)
        };
        let old = resolve(&outgoing)?;
        let new = resolve(&incoming)?;
        if old.member.participant().as_str() != self.read.context().source_participant
            || new.member.participant().as_str() != self.read.context().target_participant
        {
            return Err(ParticipantControlError::Authority);
        }
        let may_observe = self.read.context().outgoing
            == connect_protocol::transitions::OutgoingDisposition::Observe;
        Ok(ParticipantHandoffAuthority {
            actor: current_member(&self.principal, self.read.caller())
                .map_err(|_| ParticipantControlError::Authority)?,
            outgoing: old.member,
            incoming: new.member,
            incoming_may_send_audio: new.receive_media.contains(&Medium::Audio),
            outgoing_may_receive_audio: may_observe && old.send_media.contains(&Medium::Audio),
        })
    }
}
impl Policy {
    pub(super) fn handoff_control(
        &self,
        scope: &ParticipantHandoffScope<'_>,
        held: &(dyn Any + Send + Sync),
    ) -> ParticipantResult<ParticipantHandoffControl> {
        let i = held
            .downcast_ref::<HandoffInvocation>()
            .ok_or(ParticipantControlError::Authority)?;
        if !std::ptr::eq(self, Arc::as_ptr(&i.policy)) {
            return Err(ParticipantControlError::Authority);
        }
        let _ = i.current(scope)?;
        let remaining = (i.read.expires_at() - chrono::Utc::now().timestamp()).max(0) as u64;
        ParticipantHandoffControl::new(
            Box::new(i.clone()),
            i.cancel.clone(),
            tokio::time::Instant::now() + Duration::from_secs(remaining),
        )
    }
}
