//! Fresh native enrollment. Save the generated key before requesting backend
//! approval; save the approved offer before claiming. Explicit retry and restart
//! preserve the offer, private key and original completion operation ID.
use crate::{canonical::CanonicalClientConfig, ClientError, NativeNetworkProfile, Result};
pub use connect_protocol::enrollment::*;
pub use connect_protocol::provisioning::{DeviceSpec, SigningAlgorithm};
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

mod http;
mod key;
mod state;
#[cfg(test)]
mod tests;
pub use http::{EnrollmentClient, EnrollmentClientConfig};
pub use key::NativeEnrollmentKey;

/// Explicit server trust and contact, supplied independently of an offer. No
/// administrator or provider credential is accepted by this installation API.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentServer {
    pub network_profile: NativeNetworkProfile,
    pub endpoint: SocketAddr,
    pub server_name: String,
    pub trust_certificate: PathBuf,
    pub server_key_id: String,
    pub server_algorithm: String,
    pub server_public_key: PathBuf,
    pub server_key_not_before: chrono::DateTime<chrono::Utc>,
    pub server_key_expires_at: chrono::DateTime<chrono::Utc>,
    pub timeout_ms: u64,
}

/// Private resumable state. Debug never exposes the offer, proof or key.
pub struct NativeEnrollment {
    directory: PathBuf,
    offer: EnrollmentOffer,
    key: NativeEnrollmentKey,
}
impl std::fmt::Debug for NativeEnrollment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeEnrollment(redacted)")
    }
}
impl NativeEnrollment {
    /// The complete expected request must be retained before asking the backend
    /// for approval. A broader or differently bound offer is refused locally.
    /// Creates a new private directory; never replaces an existing enrollment.
    pub fn create(
        directory: &Path,
        mut offer: EnrollmentOffer,
        mut expected: EnrollmentRequest,
        key: NativeEnrollmentKey,
    ) -> Result<Self> {
        expected
            .normalize()
            .map_err(|_| ClientError::Configuration)?;
        offer.normalize().map_err(|_| ClientError::Configuration)?;
        if expected != offer.request {
            return Err(ClientError::Configuration);
        }
        Self::check_key(&offer, &key)?;
        state::create_directory(directory)?;
        // The offer is written last: incomplete crash state cannot be loaded.
        key.save(&directory.join("client.pk8"))?;
        state::write_private(
            &directory.join("offer.json"),
            &serde_json::to_vec(&offer).map_err(|_| ClientError::Configuration)?,
        )?;
        Self::load(directory)
    }
    /// Loads the exact saved offer and key. Challenge freshness is a server
    /// decision: completed original recovery may outlive the challenge.
    pub fn load(directory: &Path) -> Result<Self> {
        state::check_directory(directory)?;
        let directory = directory
            .canonicalize()
            .map_err(|_| ClientError::Configuration)?;
        let mut offer: EnrollmentOffer =
            serde_json::from_slice(&state::read_private(&directory.join("offer.json"), 65_536)?)
                .map_err(|_| ClientError::Configuration)?;
        offer.normalize().map_err(|_| ClientError::Configuration)?;
        let key =
            NativeEnrollmentKey::load(&directory.join("client.pk8"), offer.request.algorithm)?;
        Self::check_key(&offer, &key)?;
        Ok(Self {
            directory,
            offer,
            key,
        })
    }
    fn check_key(offer: &EnrollmentOffer, key: &NativeEnrollmentKey) -> Result<()> {
        if offer.request.transport != EnrollmentTransport::Quic
            || offer.request.browser_origin.is_some()
            || offer.request.algorithm != key.algorithm()
            || offer.request.public_key != key.public_key()
            || [".", ".."].contains(&offer.enrollment_id.as_str())
        {
            return Err(ClientError::Configuration);
        }
        Ok(())
    }
    /// Protected original offer, for deliberate reconciliation only.
    pub fn offer(&self) -> &EnrollmentOffer {
        &self.offer
    }
    pub fn claim(&self) -> Result<EnrollmentClaim> {
        let r = &self.offer.request;
        Ok(EnrollmentClaim {
            operation_id: r.completion_operation_id.clone(),
            enrollment_id: self.offer.enrollment_id.clone(),
            identity_id: r.identity_id.clone(),
            device_id: r.device.id.clone(),
            endpoint_id: self.offer.endpoint_id.clone(),
            key_id: r.key_id.clone(),
            transport: r.transport,
            browser_origin: None,
            challenge: self.offer.challenge.clone(),
            signature: self.key.sign(
                &self
                    .offer
                    .proof_message()
                    .map_err(|_| ClientError::Configuration)?,
            )?,
        })
    }
    pub(super) fn check_credential(&self, value: &EnrollmentCredential) -> Result<()> {
        let o = &self.offer;
        let r = &o.request;
        let c = &value.credential;
        let now = chrono::Utc::now().timestamp();
        if value.enrollment_id != o.enrollment_id
            || value.endpoint_id != o.endpoint_id
            || value.device_id != r.device.id
            || value.key_id != r.key_id
            || value.transport != r.transport
            || value.browser_origin != r.browser_origin
            || c.operation_id != r.completion_operation_id
            || c.identity_id != r.identity_id
            || c.issuer_identity_id != o.issuer_identity_id
            || c.installation_id != o.installation_id
            || c.operations != r.operations
            || c.recipients != r.recipients
            || c.expires_at != r.credential_expires_at
            || c.expires_at <= now
            || c.issued_at <= 0
            || c.issued_at > now
            || c.issued_at >= c.expires_at
            || c.credential.len() != 64
            || !c
                .credential
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || [&c.credential_id, &c.issuer_credential_id]
                .iter()
                .any(|s| uuid::Uuid::parse_str(s).is_err())
        {
            return Err(ClientError::Protocol);
        }
        Ok(())
    }
    /// Writes private client.toml beside the saved original offer/key. A matching
    /// replay is idempotent; a different installation never overwrites it.
    pub fn install(
        &self,
        value: &EnrollmentCredential,
        server: EnrollmentServer,
    ) -> Result<PathBuf> {
        self.check_credential(value)?;
        let r = &self.offer.request;
        let config = CanonicalClientConfig {
            network_profile: server.network_profile,
            endpoint: server.endpoint,
            server_name: server.server_name,
            trust_certificate: server
                .trust_certificate
                .canonicalize()
                .map_err(|_| ClientError::Configuration)?,
            credential: value.credential.credential.clone(),
            identity_id: r.identity_id.clone(),
            device_id: r.device.id.clone(),
            device_kind: r.device.kind.clone(),
            device_platform: r.device.platform.clone(),
            signing_key_id: r.key_id.clone(),
            signing_algorithm: match r.algorithm {
                SigningAlgorithm::EdDSA => "EdDSA",
                SigningAlgorithm::ES256 => "ES256",
            }
            .into(),
            signing_private_key: self.directory.join("client.pk8"),
            server_key_id: server.server_key_id,
            server_algorithm: server.server_algorithm,
            server_public_key: server
                .server_public_key
                .canonicalize()
                .map_err(|_| ClientError::Configuration)?,
            server_key_not_before: server.server_key_not_before,
            server_key_expires_at: server.server_key_expires_at,
            timeout_ms: server.timeout_ms,
        };
        config.validate()?;
        // Check persisted material again before making the config usable.
        let loaded = Self::load(&self.directory)?;
        if loaded.offer != self.offer || loaded.key.public_key() != self.key.public_key() {
            return Err(ClientError::Configuration);
        }
        let bytes = toml::to_string(&config)
            .map_err(|_| ClientError::Configuration)?
            .into_bytes();
        let path = self.directory.join("client.toml");
        if path.try_exists().map_err(|_| ClientError::Configuration)? {
            if state::read_private(&path, 65_536)? != bytes {
                return Err(ClientError::Configuration);
            }
        } else {
            state::write_private(&path, &bytes)?;
        }
        Ok(path)
    }
}
