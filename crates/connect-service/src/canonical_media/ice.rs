//! Fresh exact ICE invocation. Never retained by a transport custodian at idle.
use super::*;
use connect_store::{NativeStreamLease, PeerAuthorityGuard};
use rvoip_core::peer_admission::PeerAdmission;
use tokio::time::Instant;

#[derive(Clone)]
pub(crate) struct Invocation {
    policy: Arc<Policy>,
    connection: Arc<Connection>,
    proof: Proof,
    read: Arc<NativeMediaRead>,
    peer: Arc<PeerAdmission>,
    expected: u64,
    deadline: Instant,
}
#[derive(Clone)]
enum Proof {
    Fresh(Arc<PeerAuthorityGuard>),
    Held(Arc<StoreInvocationAuthority>),
}
impl Proof {
    fn guard(&self) -> connect_protocol::Result<&PeerAuthorityGuard> {
        match self {
            Self::Fresh(p) => Ok(p),
            Self::Held(p) => p
                .peer_guard()
                .ok_or_else(connect_protocol::Error::forbidden),
        }
    }
}
impl Policy {
    pub(crate) fn ice_invocation(
        self: &Arc<Self>,
        proof: Arc<PeerAuthorityGuard>,
        lease: &NativeStreamLease,
        peer: Arc<PeerAdmission>,
        deadline: Instant,
    ) -> connect_protocol::Result<Invocation> {
        self.ice_invocation_from(Proof::Fresh(proof), lease, peer, deadline)
    }
    pub(crate) fn held_ice_invocation(
        self: &Arc<Self>,
        authority: &StoreInvocationAuthority,
        lease: &NativeStreamLease,
        peer: Arc<PeerAdmission>,
        deadline: Instant,
    ) -> connect_protocol::Result<Invocation> {
        self.ice_invocation_from(
            Proof::Held(Arc::new(authority.retained())),
            lease,
            peer,
            deadline,
        )
    }
    fn ice_invocation_from(
        self: &Arc<Self>,
        proof: Proof,
        lease: &NativeStreamLease,
        peer: Arc<PeerAdmission>,
        deadline: Instant,
    ) -> connect_protocol::Result<Invocation> {
        if lease.ice_restart().is_none() {
            return Err(connect_protocol::Error::forbidden());
        }
        let connection = self
            .entry(&ConnectionId::from_string(
                lease.runtime_connection_id().to_owned(),
            ))
            .map_err(|error| match error {
                StreamControlError::NotReady | StreamControlError::Backpressure => {
                    connect_protocol::Error::new(
                        "authority-busy",
                        "ICE invocation metadata is busy",
                    )
                }
                _ => connect_protocol::Error::forbidden(),
            })?;
        let (read, _) = self.store.native_stream_media_reads_with_peer(
            &connection.origin,
            &[],
            proof.guard()?,
            lease,
        )?;
        Ok(Invocation {
            policy: self.clone(),
            connection,
            proof,
            read: Arc::new(read),
            peer,
            expected: lease.expected_revision(),
            deadline,
        })
    }
}
impl Invocation {
    fn owner(&self, policy: &Policy) -> ControlResult<MediaOwner> {
        let id =
            ConnectionId::from_string(self.connection.origin.runtime_connection_id().to_owned());
        let registered = policy
            .entries
            .lock()
            .map_err(|_| StreamControlError::Authority)?
            .get(&id)
            .is_some_and(|c| Arc::ptr_eq(c, &self.connection));
        if !registered
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
            || !self.proof.guard().is_ok_and(|p| p.current())
            || !self.read.current()
            || Instant::now() >= self.deadline
            || self.read.target().endpoint_transport
                != connect_protocol::provisioning::EndpointTransport::Webrtc
        {
            return Err(StreamControlError::Authority);
        }
        let owner = self
            .connection
            .owner(self.read.target(), &self.connection.principal, false)?;
        self.connection
            .activation()?
            .ok_or(StreamControlError::NotReady)?
            .current(owner)
    }
    pub(super) fn current(
        &self,
        policy: &Policy,
        scope: &MediaScope<'_>,
        expected: u64,
    ) -> ControlResult<Box<dyn MediaAuthorityLease>> {
        if expected != self.expected {
            return Err(StreamControlError::Authority);
        }
        policy.scope(scope, &self.connection)?;
        Ok(Box::new(Held {
            owner: self.owner(policy)?,
            invocation: self.clone(),
        }))
    }
}
struct Held {
    owner: MediaOwner,
    invocation: Invocation,
}
impl MediaAuthorityLease for Held {
    fn authority(&self) -> &MediaOwner {
        &self.owner
    }
    fn control_owner(&self) -> ControlResult<Box<MediaOwner>> {
        Ok(Box::new(self.invocation.owner(&self.invocation.policy)?))
    }
}
