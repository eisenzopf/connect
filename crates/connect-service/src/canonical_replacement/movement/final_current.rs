//! Pure currentness of the exact Store scene proof and physical resource bank.
use super::super::continuation::{Invocation, Phase};
use super::*;
use rvoip_core::orchestrator::participant_control::{
    ManagedAttemptBinding, ParticipantReplacementScope, ReplacementFinalCurrent,
};
use rvoip_core::staged_streams::{
    MediaControlIntent, MediaOwner, MediaParticipantSource, MediaPurpose, MediaScope, MediaSource,
    StreamController,
};
use std::collections::BTreeMap;

pub(super) struct Current<'a> {
    invocation: &'a Invocation,
    attempt: (&'a str, &'a str, &'a str),
    resources: &'a crate::canonical_media::FinalResourceBank,
    sip: Option<&'a rvoip_sip::adapter::SipCurrentMediaRoute<'a>>,
    owners: BTreeMap<ConnectionId, MediaOwner>,
    principals: BTreeMap<ConnectionId, rvoip_core::AuthenticatedPrincipal>,
}
impl<'a> Current<'a> {
    /// Derive actual owners/principals before Core acquires any final guard.
    /// Every temporary callback lease remains in this one borrowed final turn.
    pub(super) fn capture(
        invocation: &'a Invocation,
        attempt: (&'a str, &'a str, &'a str),
        resources: &'a crate::canonical_media::FinalResourceBank,
        sip: Option<&'a rvoip_sip::adapter::SipCurrentMediaRoute<'a>>,
    ) -> rvoip_core::staged_streams::ControlResult<Self> {
        let mut owners = BTreeMap::new();
        let mut principals = BTreeMap::new();
        for connection in resources.connections() {
            owners.insert(connection.clone(), invocation.owner(connection)?);
            principals.insert(connection.clone(), invocation.principal_for(connection)?);
        }
        Ok(Self {
            invocation,
            attempt,
            resources,
            sip,
            owners,
            principals,
        })
    }
    fn current(&self, attempt: ManagedAttemptBinding<'_>) -> bool {
        self.attempt == (attempt.record, attempt.attempt_id, attempt.effect_id)
            && attempt.record == self.invocation.root.origin.record_id()
            && self.invocation.final_turn
            && self.invocation.check().is_ok()
            && (self.invocation.source_phone.is_none() || self.sip.is_some())
            && self.sip.is_none_or(|proof| {
                proof.scope().attempt_binding() == self.attempt && proof.is_current()
            })
    }
    fn intent(&self, intent: &MediaControlIntent<'_>) -> bool {
        matches!((&self.invocation.phase,intent),(Phase::Commit(original),MediaControlIntent::DeviceReplacement{plan})if original.media().audience().same_intent(plan))
    }
    fn owner_matches(&self, source: &MediaSource) -> bool {
        self.owners.get(&source.connection).is_some_and(|owner| {
            owner.member.conversation() == &source.conversation
                && owner.presence.session() == &source.session
                && owner.member.participant() == &source.participant
                && owner.member.epoch() == source.membership_epoch
                && owner.connection.connection() == &source.connection
                && owner.connection.generation() == source.connection_generation
        })
    }
}
impl ReplacementFinalCurrent for Current<'_> {
    fn is_current(
        &self,
        scope: &ParticipantReplacementScope<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt) && self.invocation.final_role_scope_current(scope).is_ok()
    }
    fn media_is_current(
        &self,
        controller: &Arc<StreamController>,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && matches!(scope.purpose, MediaPurpose::Control)
            && self.owners.get(scope.connection).is_some_and(|owner| {
                scope.conversation == owner.member.conversation()
                    && scope.session == owner.presence.session()
            })
            && self
                .principals
                .get(scope.connection)
                .is_some_and(|principal| {
                    rvoip_core::staged_streams::reliable_data::same_credential(
                        scope.principal,
                        principal,
                    )
                })
            && self
                .resources
                .control_current(scope.connection, controller, self.sip)
    }
    fn source_is_current(
        &self,
        controller: &Arc<StreamController>,
        source: &MediaSource,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && self.owner_matches(source)
            && self.resources.source_current(controller, source, self.sip)
    }
    fn participant_source_is_current(
        &self,
        controller: &Arc<StreamController>,
        source: &MediaParticipantSource,
        intent: &MediaControlIntent<'_>,
        attempt: ManagedAttemptBinding<'_>,
    ) -> bool {
        self.current(attempt)
            && self.intent(intent)
            && source.medium == rvoip_core::stream_lifecycle::Medium::Audio
            && self.owners.get(&source.connection).is_some_and(|owner| {
                owner.member.conversation() == &source.conversation
                    && owner.presence.session() == &source.session
                    && owner.member.participant() == &source.participant
                    && owner.member.identity() == &source.identity
                    && owner.member.epoch() == source.membership_epoch
                    && owner.connection.connection() == &source.connection
                    && owner
                        .member
                        .revalidate_principal(&source.principal, chrono::Utc::now())
                        .is_ok()
            })
            && self
                .resources
                .participant_current(controller, source, self.sip)
    }
}
