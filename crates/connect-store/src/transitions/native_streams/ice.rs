//! Exact private ICE negotiation custody. This module never calls an engine.
use super::*;
use connect_protocol::{provisioning::EndpointTransport, webrtc_ice_restart as wire};

pub struct NativeIceRestart {
    expected: u64,
    previous: String,
    proposed: String,
    local: Option<String>,
    answer: Option<String>,
}
impl std::fmt::Debug for NativeIceRestart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeIceRestart(redacted)")
    }
}
impl NativeIceRestart {
    pub fn expected_generation(&self) -> u64 {
        self.expected
    }
    pub fn previous_remote_offer(&self) -> &str {
        &self.previous
    }
    pub fn proposed_remote_offer(&self) -> &str {
        &self.proposed
    }
    pub fn previous_local_answer(&self) -> Option<&str> {
        self.local.as_deref()
    }
    pub fn answer(&self) -> Option<&str> {
        self.answer.as_deref()
    }
}
impl NativeStreamLease {
    pub fn ice_restart(&self) -> Option<&NativeIceRestart> {
        self.ice_restart.as_ref()
    }
}
pub(super) fn load(c: &Connection, record: &str) -> Result<NativeIceRestart> {
    c.query_row("SELECT expected_ice_generation,previous_remote_offer,proposed_remote_offer,previous_local_answer,answer_sdp FROM native_webrtc_ice_operations WHERE operation_record_id=?1", [record], |r| Ok(NativeIceRestart { expected:r.get(0)?, previous:r.get(1)?, proposed:r.get(2)?, local:r.get(3)?, answer:r.get(4)? })).map_err(|_|Error::storage())
}
pub(super) fn accept(
    c: &Connection,
    intent: &Intent,
    record: &str,
    source: &crate::NativeStreamProjection,
) -> Result<()> {
    if source.connection.endpoint_transport != EndpointTransport::Webrtc {
        return Err(Error::new(
            "unsupported-media-profile",
            "ICE restart requires the original WebRTC peer",
        ));
    }
    let update = wire::parse_update(&intent.payload)?;
    let original = source.connection.offer["substrate_setup"]["sdp"]
        .as_str()
        .ok_or_else(conflict)?;
    c.execute("INSERT INTO native_webrtc_ice_state(parent_record_id,connection_id,connection_generation,ice_generation,remote_offer) SELECT ?1,?2,?3,0,?4 WHERE NOT EXISTS(SELECT 1 FROM native_webrtc_ice_state WHERE parent_record_id=?1)", params![source.parent_record_id,intent.connection_id,source.connection.connection.generation,original]).map_err(|_|Error::storage())?;
    let (generation, previous): (u64,String) = c.query_row("SELECT ice_generation,remote_offer FROM native_webrtc_ice_state WHERE parent_record_id=?1 AND connection_id=?2 AND connection_generation=?3", params![source.parent_record_id,intent.connection_id,source.connection.connection.generation], |r| Ok((r.get(0)?,r.get(1)?))).map_err(|_|Error::storage())?;
    if generation != update.details.expected_ice_generation {
        return Err(conflict());
    }
    wire::validate_offer_change(&previous, &update.details.setup.sdp)?;
    c.execute("INSERT INTO native_webrtc_ice_operations(operation_record_id,parent_record_id,expected_ice_generation,previous_remote_offer,proposed_remote_offer) VALUES(?1,?2,?3,?4,?5)", params![record,source.parent_record_id,generation,previous,update.details.setup.sdp]).map_err(|_|Error::storage())?;
    Ok(())
}
fn sdp(value: &str) -> Result<()> {
    if !value.starts_with("v=0\r\n") || value.len() > wire::MAX_SDP_BYTES || value.contains('\0') {
        return Err(invalid());
    }
    Ok(())
}
impl Store {
    /// Trusted host capture of the actual original peer's descriptions before
    /// restart IO. This saves historical metadata, never a physical IO grant.
    pub fn capture_native_ice_baseline(
        &self,
        lease: &mut NativeStreamLease,
        local: &str,
        remote: &str,
    ) -> Result<()> {
        sdp(local)?;
        sdp(remote)?;
        let ice = lease.ice_restart.as_ref().ok_or_else(conflict)?;
        if remote != ice.previous || ice.local.as_deref().is_some_and(|v| v != local) {
            return Err(conflict());
        }
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = outcomes::exact(&tx, self, lease)?;
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        let previous:Option<String>=tx.query_row("SELECT i.answer_sdp FROM native_webrtc_ice_state s LEFT JOIN native_webrtc_ice_operations i ON i.operation_record_id=s.last_record_id WHERE s.parent_record_id=?1",[&lease.parent],|r|r.get(0)).map_err(|_|Error::storage())?;
        if previous.as_deref().is_some_and(|v| v != local) {
            return Err(conflict());
        }
        tx.execute("UPDATE native_webrtc_ice_operations SET previous_local_answer=?2 WHERE operation_record_id=?1 AND (previous_local_answer IS NULL OR previous_local_answer=?2)",params![lease.record,local]).map_err(|_|Error::storage())?;
        let saved = load(&tx, &lease.record)?;
        if saved.previous != remote || saved.local.as_deref() != Some(local) {
            return Err(conflict());
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        lease.ice_restart = Some(saved);
        Ok(())
    }
    /// Trusted host capture of the actual gathered answer. Call before reply
    /// delivery; a lost observer can read this exact answer without new IO.
    /// It is not evidence of nominated ICE, media readiness or completion.
    pub fn capture_native_ice_answer(
        &self,
        lease: &mut NativeStreamLease,
        answer: &str,
    ) -> Result<()> {
        sdp(answer)?;
        let ice = lease.ice_restart.as_ref().ok_or_else(conflict)?;
        wire::validate_offer_change(ice.local.as_deref().ok_or_else(conflict)?, answer)?;
        if ice.answer.as_deref().is_some_and(|v| v != answer) {
            return Err(conflict());
        }
        let owner = self.require_runtime_owner()?;
        let mut c = self.connection()?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| Error::storage())?;
        let row = outcomes::exact(&tx, self, lease)?;
        if !matches!(
            row.record.state,
            OperationState::Preparing | OperationState::Unknown
        ) {
            return Err(conflict());
        }
        tx.execute("UPDATE native_webrtc_ice_operations SET answer_sdp=?2 WHERE operation_record_id=?1 AND (answer_sdp IS NULL OR answer_sdp=?2)",params![lease.record,answer]).map_err(|_|Error::storage())?;
        let saved = load(&tx, &lease.record)?;
        if saved.answer.as_deref() != Some(answer) || saved.local != ice.local {
            return Err(conflict());
        }
        if !owner.current() {
            return Err(Error::forbidden());
        }
        tx.commit().map_err(|_| Error::storage())?;
        lease.ice_restart = Some(saved);
        Ok(())
    }
    /// Reauthorize the original request and its current audience before reading
    /// private negotiation metadata. No source/attempt is created on replay.
    pub fn native_ice_answer_with_peer(
        &self,
        credential: &str,
        proof: &PeerAuthorityGuard,
    ) -> Result<Option<String>> {
        if !intent(proof)?.is_ice_restart() {
            return Err(Error::forbidden());
        }
        let c = self.connection()?;
        let row = checked_row(&c, self, credential, proof)?;
        let saved = load(&c, &row.record.record_id)?;
        self.require_peer_source(&c, proof, credential)?;
        Ok(saved.answer)
    }
}
pub(super) fn complete(c: &Connection, lease: &NativeStreamLease, generation: u64) -> Result<()> {
    let ice = lease.ice_restart.as_ref().ok_or_else(conflict)?;
    let saved = load(c, &lease.record)?;
    if generation != ice.expected + 1
        || saved.expected != ice.expected
        || saved.previous != ice.previous
        || saved.proposed != ice.proposed
        || saved.local != ice.local
        || saved.answer != ice.answer
        || saved.answer.is_none()
    {
        return Err(conflict());
    }
    c.execute("UPDATE native_webrtc_ice_operations SET committed=1 WHERE operation_record_id=?1 AND committed=0",[&lease.record]).map_err(|_|Error::storage())?;
    let changed=c.execute("UPDATE native_webrtc_ice_state SET ice_generation=?2,remote_offer=?3,last_record_id=?4 WHERE parent_record_id=?1 AND ice_generation=?5 AND remote_offer=?6",params![lease.parent,generation,ice.proposed,lease.record,ice.expected,ice.previous]).map_err(|_|Error::storage())?;
    if changed != 1 {
        return Err(conflict());
    }
    Ok(())
}
