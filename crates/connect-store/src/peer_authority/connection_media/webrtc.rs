//! Fresh current control for the original RTC offer after its SDP handler exits.
use super::*;
impl Store {
    /// Fresh bounded control for the exact original WebRTC offer after its SDP
    /// handler has released all readers. This creates neither an attempt nor
    /// an effect. The Store-issued origin is attribution, never a saved grant.
    pub fn resume_webrtc_offer_authority(
        &self,
        credential: &str,
        origin: &NativeMediaOrigin,
        attempt: &crate::NativeOfferLease,
        wait: &AuthorityWait,
    ) -> Result<PeerAuthorityGuard> {
        current_owner(self, origin)?;
        if !matches!(&origin.attempt, NativeMediaAdmission::Offer(_))
            || origin.intent.endpoint_transport()?
                != connect_protocol::provisioning::EndpointTransport::Webrtc
            || origin.binding.credentials.last().map(String::as_str) != Some(credential)
            || origin.offer_record_id() != attempt.record_id()
            || origin.connection_id() != attempt.connection_id()
            || origin.runtime_connection_id() != attempt.runtime_connection_id()
            || origin.generation() != attempt.generation()
            || origin.intent.operation_id != attempt.operation_id()
            || crate::now()
                >= origin
                    .expires_at
                    .min(attempt.expires_at())
                    .min(attempt.source_expires_at())
        {
            return Err(Error::forbidden());
        }
        let expires_at = origin
            .expires_at
            .min(attempt.expires_at())
            .min(attempt.source_expires_at());
        let remaining = chrono::DateTime::from_timestamp(expires_at, 0)
            .and_then(|expiry| (expiry - chrono::Utc::now()).to_std().ok())
            .ok_or_else(Error::forbidden)?;
        let wait = wait.bounded(remaining);
        let mut request = origin.binding.clone();
        request.command = PeerCommand::NativeConnectionOffer(origin.intent.clone());
        let mut proof = self.admit_peer_authority(&request, &wait)?;
        proof.limit_expiry(expires_at)?;
        self.validate_native_offer_attempt(credential, attempt, &proof)?;
        if proof.key.public_key != origin.public_key
            || proof.native_member.as_ref()
                != Some(&(origin.participant_id.clone(), origin.membership_epoch))
            || !proof.current()
        {
            return Err(Error::forbidden());
        }
        // This derives the current original endpoint/acceptance/membership/key
        // under this SAME newly acquired proof, without reader reentry.
        let read = self.native_media_read_with_peer(origin, None, &proof)?;
        let view = read.target();
        if !read.current()
            || view.endpoint_transport != connect_protocol::provisioning::EndpointTransport::Webrtc
            || view.connection.endpoint_id != origin.endpoint_id
            || view.connection.participant_id != origin.participant_id
            || view.connection.membership_epoch != origin.membership_epoch
            || view.connection.generation != origin.generation
            || view.runtime_connection_id != origin.runtime_connection_id
        {
            return Err(Error::forbidden());
        }
        drop(read);
        current_owner(self, origin)?;
        Ok(proof)
    }
}
