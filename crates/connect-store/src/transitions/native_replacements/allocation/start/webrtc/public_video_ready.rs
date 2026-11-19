//! Signed public-B ready custody. No phase, native readiness or IO is issued.
use super::*;
use crate::transitions::native_replacements::public_video::validate_public_ready_marker;
const PROFILE: &str = "connect-replacement-public-video-ready-v1";
fn normalize_public_ready(value: &mut serde_json::Value) -> Result<serde_json::Value> {
    let marker = &mut value["replacement_preparation"];
    if marker["profile"] != PROFILE
        || marker.as_object().map(|value| value.len()) != Some(3)
        || marker.get("public_video_readiness").is_none()
    {
        return Err(Error::forbidden());
    }
    let original = marker.clone();
    marker["profile"] = serde_json::json!("connect-replacement-media-v1");
    marker
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("public_video_readiness");
    Ok(original)
}
impl Store {
    /// Fresh capture under this SAME signed target/source/retained Read. It
    /// neither persists readiness nor admits B media. Original expiry is clamped.
    pub fn native_replacement_public_video_ready_read_with_peer(
        &self,
        started: &NativeReplacementStarted,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<NativeReplacementStartRead> {
        let a = &started.attempt;
        let p = &a.allocated.attempt.prepared;
        let (member, saved, descriptor, marker) = {
            let c = self.try_connection()?;
            let saved = allocated_current(self, &c, &a.allocated)?;
            let (member, mut descriptor) =
                target_facts_for_profile(self, &c, p, credential, proof, &saved, PROFILE)?;
            let marker = normalize_public_ready(&mut descriptor["intent"]["offer"])?;
            if crate::provisioning::canonical_json(&descriptor)? != a.target {
                return Err(conflict());
            }
            (member, saved, descriptor, marker)
        };
        let source = p.selection.read_with_peer(self, proof)?;
        let expires = {
            let c = self.try_connection()?;
            validate_public_ready_marker(self, &c, p, &marker, Some(&source))?
        };
        let target = projection(&source, member, &saved, &descriptor)?;
        let mut read = NativeReplacementStartRead {
            source,
            target,
            descriptor,
        };
        read.limit(expires)?;
        if !proof.current() || !read.current() {
            return Err(Error::forbidden());
        }
        Ok(read)
    }
    /// Observe the original result after Host has matched the WHOLE same
    /// captured ReadyMarker, including decoded counts and nullable key count.
    /// Store validates the original challenge/tuple and current signed target
    /// only. This is not an identical-capture/native readiness issuer, source
    /// reauthorization, preparation renewal or a repeated Core transition.
    pub fn replay_native_replacement_public_video_ready_with_peer(
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
        let marker = normalize_public_ready(&mut normalized.offer)?;
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
        validate_public_ready_marker(self, &c, p, &marker, None)?;
        let row = rows::by_record(&c, &p.record)?;
        admission::require_row(self, Self::peer_scopes(proof), &row)?;
        if !proof.current() {
            return Err(Error::forbidden());
        }
        let mut record = row.record;
        record.result = None;
        record.problem = None;
        record.effects.clear();
        Ok(record)
    }
}
