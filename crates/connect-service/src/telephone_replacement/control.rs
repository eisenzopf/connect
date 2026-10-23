//! Allocation owns one bounded read. Start retains metadata and reacquires on each check.
use super::*;
use crate::{canonical_replacement::PreparedReplacement, canonical_runtime::current_member};
use connect_store::{
    NativeReplacementPreparationRead, TelephoneReplacementAttempt, TelephoneReplacementRead,
};
use rvoip_core::orchestrator::participant_control::{
    ControlResult, ParticipantControlError, ParticipantReplacementAuthority,
    ParticipantReplacementControl, ParticipantReplacementLease, ParticipantReplacementScope,
    ReplacementPhase,
};
#[derive(Clone)]
pub(crate) enum Phase {
    Allocate(Arc<TelephoneReplacementRead>),
    Start(Arc<TelephoneReplacementAttempt>),
}
#[derive(Clone)]
pub(crate) struct Invocation {
    pub runtime: Arc<Runtime>,
    pub root: Arc<PreparedReplacement>,
    pub route: Arc<TelephoneReplacementRoute>,
    pub target: Arc<authority::Target>,
    pub phase: Phase,
}
impl Invocation {
    pub fn control(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ControlResult<ParticipantReplacementControl> {
        self.current(scope)?;
        ParticipantReplacementControl::new(
            Box::new(self.clone()),
            self.root.handle.cancellation(),
            self.root.deadline,
        )
    }
    fn authority(
        &self,
        scope: &ParticipantReplacementScope<'_>,
        read: &NativeReplacementPreparationRead,
        destination: &str,
    ) -> ControlResult<ParticipantReplacementAuthority> {
        let deny = || ParticipantControlError::Authority;
        if self.runtime.stop.is_cancelled()
            || self.root.handle.cancellation().is_cancelled()
            || self.root.handle.is_committed()
            || tokio::time::Instant::now() >= self.root.deadline
            || scope.plan.is_some()
            || scope.intent != self.root.handle.intent()
            || scope.conversation.as_str() != read.context().conversation_id
            || scope.session.as_str() != read.context().session_id
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &self.root.operator,
            )
        {
            return Err(deny());
        }
        let original = self.runtime.media.replacement_source_owner(read)?;
        // Allocation calls current() again under Core's connection registry.
        // Its already-fenced actor/endpoint facts were resolved BEFORE entering
        // Core; never take the Store SQL mutex under that registry.
        let target = &self.target;
        let ctx = read.context();
        if target.principal.subject != ctx.target_route_owner_id
            || target.principal.is_expired()
            || target.member.identity().as_str() != ctx.identity_id
            || target.member.participant() != original.member.participant()
            || target.member.epoch() != original.member.epoch()
            || target.endpoint.endpoint().as_str() != ctx.target_endpoint_id
            || target.endpoint.device().as_str() != ctx.target_device_id
            || target.endpoint.address() != destination
            || ctx.target_secret_ref.as_deref() != Some(self.route.policy.route_reference.as_str())
        {
            return Err(deny());
        }
        Ok(ParticipantReplacementAuthority {
            actor: current_member(&self.root.operator, read.caller()).map_err(|_| deny())?,
            target: target.member.clone(),
            endpoint: target.endpoint.clone(),
            may_send_audio: read.context().role != "observer"
                && original
                    .receive_media
                    .contains(&rvoip_core::stream_lifecycle::Medium::Audio),
            may_receive_audio: original
                .send_media
                .contains(&rvoip_core::stream_lifecycle::Medium::Audio),
        })
    }
}
impl ParticipantReplacementLease for Invocation {
    fn current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
    ) -> ControlResult<ParticipantReplacementAuthority> {
        match (&self.phase, scope.phase) {
            (Phase::Allocate(read), ReplacementPhase::Allocate) => self.authority(
                scope,
                read.source(),
                &read
                    .destination()
                    .map_err(|_| ParticipantControlError::Authority)?,
            ),
            (Phase::Start(attempt), ReplacementPhase::Start) => {
                // This read is dropped before returning. Core may retain the
                // control object while waiting for answer; it retains NO reader.
                let read = self
                    .runtime
                    .media
                    .store
                    .telephone_replacement_submission_control_read(attempt.clone())
                    .map_err(|error| {
                        tracing::debug!(target:"connect_service::telephone_replacement",code=%error.code,"Current telephone start check refused");
                        ParticipantControlError::Authority
                    })?;
                self.authority(
                    scope,
                    read.source(),
                    &read
                        .destination()
                        .map_err(|_| ParticipantControlError::Authority)?,
                )
            }
            _ => Err(ParticipantControlError::Authority),
        }
    }
}
