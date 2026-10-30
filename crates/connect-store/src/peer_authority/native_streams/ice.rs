//! Attribution for the one owned ICE attempt after its answer handler exits.
use super::*;
use crate::NativeStreamLease;
use connect_protocol::provisioning::EndpointTransport;
use std::sync::Weak;

/// Immutable Store-issued attribution, never a retained authority reader. It
/// cannot be built from wire IDs or used to create another effect or attempt.
pub struct NativeIceOrigin {
    owner: Weak<InstallationOwnership>,
    request: PeerAuthorityRequest,
    record: String,
    effect: String,
    runtime: String,
    core_connection: String,
    connection: String,
    generation: u64,
    endpoint: String,
    participant: String,
    membership_epoch: u64,
    public_key: Vec<u8>,
    expires_at: i64,
}
impl std::fmt::Debug for NativeIceOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeIceOrigin(redacted)")
    }
}
impl Store {
    /// Bind only the exact current signed command and its already-created local
    /// lease. The full credential chain and command remain immutable.
    pub fn native_ice_origin_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
        lease: &NativeStreamLease,
    ) -> Result<Arc<NativeIceOrigin>> {
        let source = self.validate_native_stream_attempt(credential, proof, lease)?;
        if lease.ice_restart().is_none()
            || source.connection.endpoint_transport != EndpointTransport::Webrtc
            || proof.request.credentials.last().map(String::as_str) != Some(credential)
            || !proof.current()
        {
            return Err(Error::forbidden());
        }
        Ok(Arc::new(NativeIceOrigin {
            owner: Arc::downgrade(&proof._owner),
            request: proof.request.clone(),
            record: lease.record_id().into(),
            effect: lease.effect_id().into(),
            runtime: self.runtime_epoch().into(),
            core_connection: lease.runtime_connection_id().into(),
            connection: lease.connection_id().into(),
            generation: lease.generation(),
            endpoint: source.connection.connection.endpoint_id,
            participant: source.connection.connection.participant_id,
            membership_epoch: source.connection.connection.membership_epoch,
            public_key: proof.key.public_key.clone(),
            expires_at: proof.expires_at().min(lease.expires_at()),
        }))
    }

    /// Acquire fresh, bounded current authority for a live observation of the
    /// original attempt only. Drop the returned guard before waiting for the
    /// remote endpoint. Unknown/status recovery never obtains a new IO lease.
    pub fn resume_native_ice_authority(
        &self,
        credential: &str,
        origin: &NativeIceOrigin,
        lease: &NativeStreamLease,
        wait: &AuthorityWait,
    ) -> Result<PeerAuthorityGuard> {
        let owner = self.require_runtime_owner()?;
        if !owner.current()
            || !origin
                .owner
                .upgrade()
                .is_some_and(|o| Arc::ptr_eq(&owner, &o))
            || self.runtime_epoch() != origin.runtime
            || lease.ice_restart().is_none()
            || origin.record != lease.record_id()
            || origin.effect != lease.effect_id()
            || origin.connection != lease.connection_id()
            || origin.core_connection != lease.runtime_connection_id()
            || origin.generation != lease.generation()
            || origin.request.credentials.last().map(String::as_str) != Some(credential)
            || crate::now() >= origin.expires_at.min(lease.expires_at())
        {
            return Err(Error::forbidden());
        }
        let expires_at = origin.expires_at.min(lease.expires_at());
        let remaining = chrono::DateTime::from_timestamp(expires_at, 0)
            .and_then(|expiry| (expiry - chrono::Utc::now()).to_std().ok())
            .ok_or_else(Error::forbidden)?;
        let mut proof = self.admit_peer_authority(&origin.request, &wait.bounded(remaining))?;
        proof.limit_expiry(expires_at)?;
        let source = self.validate_native_stream_attempt(credential, &proof, lease)?;
        if proof.key.public_key != origin.public_key
            || proof.native_member.as_ref()
                != Some(&(origin.participant.clone(), origin.membership_epoch))
            || source.connection.endpoint_transport != EndpointTransport::Webrtc
            || source.connection.connection.endpoint_id != origin.endpoint
            || source.connection.connection.participant_id != origin.participant
            || source.connection.connection.membership_epoch != origin.membership_epoch
            || !proof.current()
        {
            return Err(Error::forbidden());
        }
        Ok(proof)
    }
}
