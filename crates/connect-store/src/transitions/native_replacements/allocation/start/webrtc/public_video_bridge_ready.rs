//! Exact signed pre-F B receiver observation. No allocation, F or IO grant.
use super::*;
use crate::transitions::native_replacements::public_video::bridge;
use connect_protocol::webrtc_public_video_bridge as wire;
fn normalize(value: &mut serde_json::Value) -> Result<wire::ReadyMarker> {
    let marker = &mut value["replacement_preparation"];
    let ready: wire::ReadyMarker =
        serde_json::from_value(marker.clone()).map_err(|_| Error::forbidden())?;
    ready.validate_shape()?;
    marker["profile"] = serde_json::json!("connect-replacement-media-v1");
    marker
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("bridge_readiness");
    Ok(ready)
}
impl Store {
    /// SAME signed key/device/physical offer and source read. The complete
    /// unmodified original target descriptor must survive normalization.
    pub fn record_native_replacement_public_video_bridge_ready_with_peer(
        &self,
        started: &NativeReplacementStarted,
        reservation: &crate::NativeReplacementPublicVideoReservation,
        credential: &str,
        proof: &PeerAuthorityGuard,
        observation: &dyn crate::NativeReplacementPublicVideoBridgeObservation,
    ) -> Result<crate::PublicVideoOperationSnapshot> {
        let a = &started.attempt;
        let p = &a.allocated.attempt.prepared;
        if !bridge::same_prepared(reservation, p) {
            return Err(Error::forbidden());
        }
        let mut c = self.try_connection()?;
        let saved = allocated_current(self, &c, &a.allocated)?;
        let (member, mut descriptor) =
            target_facts_for_profile(self, &c, p, credential, proof, &saved, wire::READY_PROFILE)?;
        let marker = normalize(&mut descriptor["intent"]["offer"])?;
        if crate::provisioning::canonical_json(&descriptor)? != a.target {
            return Err(conflict());
        }
        let source = p.selection.read_with_connection(
            self,
            &c,
            Self::peer_scopes(proof).clone(),
            proof.expires_at(),
        )?;
        let expiry = bridge::validate_signed_ready(self, &c, p, &marker, &source)?;
        let target = projection(&source, member, &saved, &descriptor)?;
        let mut read = NativeReplacementStartRead {
            source,
            target,
            descriptor,
        };
        read.limit(expiry)?;
        if !proof.current() || !read.current() {
            return Err(Error::forbidden());
        }
        // Immediate SQL on the SAME supplied Read; no nested Read acquisition,
        // queued Writer wait, native callback or new operation/attempt.
        c.bind_available_mutation_authority(self, Self::peer_scopes(proof))?;
        let snapshot = bridge::record_in(
            self,
            &mut c,
            Self::peer_scopes(proof),
            reservation,
            Some(&marker),
            observation,
        )?;
        if !proof.current() || !read.current() {
            return Err(Error::forbidden());
        }
        self.require_peer_source(&c, proof, credential)?;
        Ok(snapshot)
    }
}

impl Store {
    pub fn replay_native_replacement_public_video_bridge_ready_with_peer(
        &self,
        started: &NativeReplacementStarted,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<OperationRecord> {
        let a = &started.attempt;
        let p = &a.allocated.attempt.prepared;
        local_owner(self, &p.owner, &p.runtime)?;
        let c = self.try_connection()?;
        self.require_peer_source(&c, proof, credential)?;
        actor_from_connection(&c, credential)?.require("connection.offer")?;
        let PeerCommand::NativeConnectionOffer(intent) = &proof.request.command else {
            return Err(Error::forbidden());
        };
        let mut normalized = intent.clone();
        let marker = normalize(&mut normalized.offer)?;
        let frozen: serde_json::Value =
            serde_json::from_str(&a.target).map_err(|_| Error::storage())?;
        if frozen["intent"] != serde_json::to_value(normalized).map_err(|_| Error::storage())?
            || frozen["credential_id"] != credential
            || frozen["key_id"] != proof.key_id()
            || frozen["device_id"] != proof.key.device_id
            || frozen["public_key"] != serde_json::json!(proof.key.public_key)
            || frozen["algorithm"]
                != serde_json::to_value(&proof.request.algorithm).map_err(|_| Error::storage())?
        {
            return Err(Error::forbidden());
        }
        let row = rows::by_record(&c, &p.record)?;
        if row.tombstoned {
            return Err(Error::forbidden());
        }
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        let observation = bridge::project(&c, &row.record)?.ok_or_else(Error::forbidden)?;
        if observation.decoded_ready.as_ref() != Some(&marker) || !proof.current() {
            return Err(Error::forbidden());
        }
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
}
