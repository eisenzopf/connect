//! Fresh original-attempt RTC control; no originating handler readers.
use super::*;
use rvoip_core::orchestrator::participant_control::{
    ParticipantAuthority, ParticipantControl, ParticipantControlError, ParticipantControlLease,
    ParticipantScope,
};
use rvoip_core::participant_transitions::ParticipantChange;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
#[derive(Clone)]
pub(crate) struct RtcInvocation {
    pub policy: Arc<Policy>,
    pub connection: Arc<Connection>,
    pub proof: Arc<connect_store::PeerAuthorityGuard>,
    pub peer: Arc<rvoip_core::peer_admission::PeerAdmission>,
    pub read: Arc<NativeMediaRead>,
    pub operation: String,
    pub correlation: String,
    pub participant_revision: u64,
    pub cancel: CancellationToken,
    pub deadline: Instant,
}
impl RtcInvocation {
    pub(crate) fn check(&self, policy: &Policy) -> ControlResult<()> {
        let id =
            ConnectionId::from_string(self.connection.origin.runtime_connection_id().to_owned());
        let current = policy.entry(&id)?;
        if !Arc::ptr_eq(&current, &self.connection)
            || !std::ptr::eq(policy, Arc::as_ptr(&self.policy))
            || !self
                .connection
                .domain
                .upgrade()
                .is_some_and(|p| Arc::ptr_eq(&p, &self.policy))
            || self
                .peer
                .validate_principal(&self.connection.principal)
                .is_err()
            || !self.proof.current()
            || !self.read.current()
            || self.cancel.is_cancelled()
            || Instant::now() >= self.deadline
            || self.read.target().endpoint_transport
                != connect_protocol::provisioning::EndpointTransport::Webrtc
            || self.read.target().private_consultation.is_some()
            || self.read.target().handoff_preparation
            || self.connection.origin.runtime_connection_id()
                != self.read.target().runtime_connection_id
        {
            return Err(StreamControlError::Authority);
        }
        Ok(())
    }
    fn base(&self) -> ControlResult<MediaOwner> {
        self.check(&self.policy)?;
        self.connection
            .owner(self.read.target(), &self.connection.principal, false)
    }
    pub fn media_control(
        &self,
        policy: &Policy,
        scope: &MediaScope<'_>,
        intent: &MediaControlIntent<'_>,
    ) -> ControlResult<MediaControl> {
        self.check(policy)?;
        policy.scope(scope, &self.connection)?;
        match intent {
            MediaControlIntent::Change {
                expected,
                desired,
                replace,
            } if *expected == 0
                && replace.is_empty()
                && desired.iter().eq(self
                    .connection
                    .plan
                    .selected()
                    .iter()
                    .map(|s| &s.requested)) => {}
            _ => return Err(StreamControlError::Authority),
        }
        MediaControl::new(
            Box::new(MediaLease {
                invocation: self.clone(),
                owner: self.base()?,
            }),
            self.cancel.child_token(),
            self.deadline,
        )
    }
    pub fn role_control(
        &self,
        policy: &Policy,
        scope: &ParticipantScope<'_>,
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<ParticipantControl> {
        self.check(policy)
            .map_err(|_| ParticipantControlError::Authority)?;
        let lease = RoleLease(self.clone());
        lease.current(scope)?;
        ParticipantControl::new(Box::new(lease), self.cancel.child_token(), self.deadline)
    }
}
struct MediaLease {
    invocation: RtcInvocation,
    owner: MediaOwner,
}
impl MediaAuthorityLease for MediaLease {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        chrono::DateTime::from_timestamp(self.invocation.read.expires_at(), 0)
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        let base = self.invocation.base()?;
        match self
            .invocation
            .connection
            .activation()?
            .filter(|a| a.confirmed_generation().is_some())
        {
            Some(a) => Ok(Box::new(a.current(base)?)),
            None => Ok(Box::new(base)),
        }
    }
    fn control_prepared(
        &self,
        p: &Arc<PreparedMediaObservation>,
    ) -> ControlResult<Box<MediaOwner>> {
        let owner = self.invocation.base()?;
        Ok(Box::new(
            self.invocation
                .connection
                .activation()?
                .ok_or(StreamControlError::NotReady)?
                .prepare(p, owner)?,
        ))
    }
}
struct RoleLease(RtcInvocation);
impl ParticipantControlLease for RoleLease {
    fn current(
        &self,
        scope: &ParticipantScope<'_>,
    ) -> rvoip_core::orchestrator::participant_control::ControlResult<ParticipantAuthority> {
        let i = &self.0;
        i.check(&i.policy)
            .map_err(|_| ParticipantControlError::Authority)?;
        if scope.mutation.expected_revision != i.participant_revision
            || scope.mutation.operation_id != i.operation
            || scope.mutation.correlation_id != i.correlation
            || !matches!(&scope.mutation.change, ParticipantChange::Admit { .. })
            || !rvoip_core::staged_streams::reliable_data::same_credential(
                scope.principal,
                &i.connection.principal,
            )
        {
            return Err(ParticipantControlError::Authority);
        }
        // Existing helper checks exact conversation/session/participant/role,
        // endpoint, member epoch and committed media generation under this read.
        super::participants::authority(&i.connection, scope, &i.read)
    }
}
