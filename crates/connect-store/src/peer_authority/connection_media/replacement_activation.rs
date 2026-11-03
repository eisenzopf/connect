//! Sealed installation metadata for a reserved replacement commit, never a grant.
use super::*;

/// An inactive ordinary entry can be allocated before the public completion
/// transaction. The enclosed origin is deliberately unusable until that exact
/// attempt is durably confirmed and completed. Each eventual packet still
/// requires the normal current authorization and lifetime checks.
pub struct NativeReplacementActivation {
    origin: Arc<NativeMediaOrigin>,
    target: PeerConnectionProjection,
}
impl std::fmt::Debug for NativeReplacementActivation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeReplacementActivation(redacted)")
    }
}
impl NativeReplacementActivation {
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

pub(crate) struct ReplacementActivationInput {
    pub record: String,
    pub attempt: String,
    pub effect: String,
    pub runtime: String,
    pub target: PeerConnectionProjection,
    pub binding: PeerAuthorityRequest,
    pub descriptor: serde_json::Value,
    pub expires_at: i64,
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
        } = input;
        let mut intent: OfferIntent =
            serde_json::from_value(descriptor["intent"].clone()).map_err(|_| Error::storage())?;
        let credential = descriptor["credential_id"]
            .as_str()
            .ok_or_else(Error::storage)?;
        if runtime != self.runtime_epoch()
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
        let origin = Arc::new(NativeMediaOrigin {
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
        Ok(Arc::new(NativeReplacementActivation { origin, target }))
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
