//! Sealed installation metadata for a reserved replacement commit, never a grant.
use super::*;

/// An inactive ordinary entry can be allocated before the public completion
/// transaction. Ordinary origin admission remains unavailable until that exact
/// attempt is durably confirmed and completed. The separate continuation read
/// requires this same original started attempt and actual current Core owner;
/// captured metadata alone never grants a packet.
pub struct NativeReplacementActivation {
    origin: Arc<NativeMediaOrigin>,
    target: PeerConnectionProjection,
    pub(super) original: crate::NativeReplacementOrigin,
    // Exact pending-phase namespace; no authority reader or runtime owner.
    pub(super) metadata: snapshot::MetadataToken,
}
impl std::fmt::Debug for NativeReplacementActivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeReplacementActivation(redacted)")
    }
}
impl NativeReplacementActivation {
    #[cfg(test)]
    pub(crate) fn continuation_memo_counts_for_test(&self) -> [u64; 3] {
        snapshot::continuation_memo_counts(&self.metadata)
    }

    /// Immutable attribution only; this is not a NativeMediaRead.
    pub fn origin(&self) -> &Arc<NativeMediaOrigin> {
        &self.origin
    }
    /// Facts captured under the original commit reservation. Do not use this
    /// projection as current permission or as a public readiness observation.
    pub fn target(&self) -> &PeerConnectionProjection {
        &self.target
    }
}

/// Holds the exact installed Service entry against removal through publication.
/// This is local resource custody, never permission to read or write media.
pub trait NativeReplacementInstallationGuard {
    /// Recheck and order native retirement around the final synchronous commit.
    /// The callback is already prepared local SQL work, never native IO or an
    /// authorization acquisition. Failure before it runs must leave no Ready.
    fn publish(&self, commit: Box<dyn FnOnce() -> Result<()> + '_>) -> Result<()>;
}

/// Trusted runtime observation of the exact inactive ordinary entry and its
/// observer installation. Acquisition must be nonblocking local metadata only:
/// never call Store, await, or perform native IO. A successful guard must prevent
/// entry removal until dropped; an instantaneous boolean check is insufficient.
pub trait NativeReplacementInstallationObservation: Send + Sync {
    fn hold_installed(
        &self,
        expected: &NativeReplacementActivation,
    ) -> Result<Option<Box<dyn NativeReplacementInstallationGuard + '_>>>;
}

pub(crate) struct ReplacementActivationInput {
    pub record: String,
    pub attempt: String,
    pub effect: String,
    pub runtime: String,
    pub target: PeerConnectionProjection,
    pub binding: PeerAuthorityRequest,
    pub descriptor: serde_json::Value,
    pub expires_at: i64,
    pub original: crate::NativeReplacementOrigin,
}
impl Store {
    /// Called only while reserving the original commit under its complete
    /// current scene read. No public factory accepts wire-supplied metadata.
    pub(crate) fn replacement_activation_metadata(
        &self,
        input: ReplacementActivationInput,
    ) -> Result<Arc<NativeReplacementActivation>> {
        let ReplacementActivationInput {
            record,
            attempt,
            effect,
            runtime,
            target,
            mut binding,
            descriptor,
            expires_at,
            original,
        } = input;
        let mut intent: OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
        let credential = descriptor["credential_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        if original.record_id() != record
            || original.runtime != runtime
            || original.request().conversation_id != target.connection.conversation_id
            || original.request().session_id != target.connection.session_id
            || runtime != self.runtime_epoch()
            || binding.credentials.last().map(String::as_str) != Some(credential)
            || descriptor["key_id"].as_str() != Some(binding.key_id.as_str())
            || binding.device.id != target.device_id
            || intent.connection_id != target.connection.connection_id
            || intent.session_id != target.connection.session_id
            || intent.conversation_id != target.connection.conversation_id
        {
            return Err(Error::forbidden());
        }
        binding.credentials = vec![credential.to_owned()];
        binding.command = PeerCommand::Binding;
        intent
            .offer
            .as_object_mut()
            .ok_or_else(Error::storage)?
            .remove("replacement_preparation");
        let snapshot_hints = native_snapshot_hints(
            &intent.conversation_id,
            &target.session.actor.identity_id,
            &intent.connection_id,
        )?;
        let origin = Arc::new(NativeMediaOrigin {
            snapshot_hints,
            attempt: NativeMediaAdmission::Replacement {
                record,
                attempt,
                effect,
                runtime,
                connection: target.connection.connection_id.clone(),
                core: target.runtime_connection_id.clone(),
                generation: target.connection.generation,
            },
            owner: Arc::downgrade(&self.require_runtime_owner()?),
            binding,
            intent,
            endpoint_id: target.connection.endpoint_id.clone(),
            participant_id: target.connection.participant_id.clone(),
            membership_epoch: target.connection.membership_epoch,
            generation: target.connection.generation,
            runtime_connection_id: target.runtime_connection_id.clone(),
            public_key: serde_json::from_value(descriptor["public_key"].clone())
                .map_err(|_| Error::storage())?,
            expires_at,
            credential_epoch: 0,
            metadata: snapshot::new_metadata(),
        });
        Ok(Arc::new(NativeReplacementActivation {
            origin,
            target,
            original,
            metadata: snapshot::new_metadata(),
        }))
    }

    /// Final activation uses current target authority, unlike historical stopped
    /// projection. The caller owns the complete writer scope and SQL transaction.
    pub(crate) fn require_native_replacement_activation(
        &self,
        c: &Connection,
        activation: &NativeReplacementActivation,
    ) -> Result<()> {
        current_owner(self, activation.origin())?;
        if !pending_current(self, c, activation.origin())? {
            return Err(Error::forbidden());
        }
        Ok(())
    }

    /// Allocate exact historical writer attribution before activation. This
    /// cannot reserve a write or authorize IO: both existing writer paths still
    /// validate the exact durable attempt and current/historical lifetime.
    pub fn pending_native_replacement_data_writer(
        &self,
        activation: &NativeReplacementActivation,
    ) -> Result<NativeDataWriter> {
        current_owner(self, &activation.origin)?;
        Ok(NativeDataWriter::from_origin(&activation.origin))
    }
}

/// Classify only a durably captured exact move awaiting its final projection.
/// This returns no grant. Credentials, membership, session and endpoint are
/// checked anew; revocation and wrong/unknown attempts stay permanent refusals.
pub(super) fn pending_current(
    store: &Store,
    c: &Connection,
    origin: &NativeMediaOrigin,
) -> Result<bool> {
    let NativeMediaAdmission::Replacement {
        record,
        attempt,
        effect,
        runtime,
        connection,
        core,
        generation,
    } = &origin.attempt
    else {
        return Ok(false);
    };
    let pending: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM native_replacements r JOIN operations o ON o.record_id=r.operation_record_id JOIN native_replacement_steps s ON s.operation_record_id=o.record_id AND s.phase='commit' JOIN operation_effects e ON e.id=s.effect_id AND e.state='confirmed' WHERE o.record_id=?1 AND s.attempt_id=?2 AND s.effect_id=?3 AND r.runtime_epoch=?4 AND r.candidate_connection_id=?5 AND r.candidate_runtime_id=?6 AND s.started=1 AND s.observation_json IS NOT NULL AND json_extract(s.observation_json,'$.movement.generation')=?7 AND o.state IN ('preparing','unknown') AND NOT EXISTS(SELECT 1 FROM native_replacement_lifetimes l WHERE l.operation_record_id=o.record_id))", params![record,attempt,effect,runtime,connection,core,generation], |r|r.get(0)).map_err(|_|Error::storage())?;
    if !pending {
        return Ok(false);
    }
    let (meta, key, _, _) = checked(store, c, &origin.binding)?;
    if key.public_key != origin.public_key {
        return Err(Error::forbidden());
    }
    meta.actor.require("connection.offer")?;
    let recipients = crate::connection_control::intent_resources(c, &meta.actor, &origin.intent)?;
    if recipients
        .iter()
        .any(|id| !meta.actor.permits_recipient(id))
    {
        return Err(Error::forbidden());
    }
    let member =
        crate::audience::member(c, &meta.actor.identity_id, origin.conversation_id(), true)?;
    let session = crate::session_control::view(c, origin.conversation_id(), origin.session_id())?;
    if member.participant_id != origin.participant_id
        || member.membership_epoch != origin.membership_epoch
        || !matches!(session.state.as_str(), "accepted" | "ready" | "active")
    {
        return Err(Error::forbidden());
    }
    let present: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM endpoints e JOIN devices d ON d.id=e.device_id AND d.identity_id=e.identity_id AND d.tenant_id=e.tenant_id AND d.revoked=0 JOIN session_participants sp ON sp.session_id=?1 AND sp.participant_id=?2 AND sp.membership_epoch=?3 AND sp.state='present' WHERE e.id=?4 AND e.device_id=?5 AND e.identity_id=?6 AND e.verification_state='verified' AND e.transport IN ('quic','webtransport','webrtc'))",params![origin.session_id(),origin.participant_id(),origin.membership_epoch(),origin.endpoint_id,origin.binding.device.id,meta.actor.identity_id],|r|r.get(0)).map_err(|_|Error::storage())?;
    if !present {
        return Err(Error::forbidden());
    }
    Ok(true)
}
