//! Fresh remote-client enrollment intent and device-key proof context.
//!
//! Validation here establishes bounded syntax and immutable claim bindings only.
//! Store admission must separately verify the signature, challenge, current
//! issuer/recipient authority and one-effect replay before issuing any credential.
use crate::{
    credential_issuance::CredentialIssueRequest,
    provisioning::{DeviceSpec, EndpointTransport, ProvisioningCommand, SigningAlgorithm},
    Error, Result,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PROFILE: &str = "connect-enrollment/1";
pub const CHALLENGE_TTL_SECONDS: i64 = 600;
pub const BROWSER_CREDENTIAL_TTL_SECONDS: i64 = 3600;
pub const NATIVE_CREDENTIAL_TTL_SECONDS: i64 = 86400;
const MAX_TIMESTAMP: i64 = 9_007_199_254_740_991;
const PROOF_DOMAIN: &[u8] = b"connect-enrollment-proof/1\0";

fn invalid() -> Error {
    Error::new("invalid-request", "Invalid bounded enrollment intent")
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EnrollmentTransport {
    Quic,
    Webtransport,
    WssWebrtc,
}
impl EnrollmentTransport {
    pub fn endpoint_transport(self) -> EndpointTransport {
        match self {
            Self::Quic => EndpointTransport::Quic,
            Self::Webtransport => EndpointTransport::Webtransport,
            Self::WssWebrtc => EndpointTransport::Webrtc,
        }
    }
}

/// Backend-approved original intent. No operator/provider secret is accepted.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRequest {
    pub operation_id: String,
    /// Client persists this logical completion ID before requesting approval.
    pub completion_operation_id: String,
    pub identity_id: String,
    pub device: DeviceSpec,
    pub key_id: String,
    pub algorithm: SigningAlgorithm,
    pub public_key: String,
    pub transport: EnrollmentTransport,
    pub endpoint_address: String,
    pub capabilities: Vec<String>,
    /// Exact serialized HTTPS origin for browser profiles; absent for QUIC.
    pub browser_origin: Option<String>,
    pub operations: Vec<String>,
    pub recipients: Vec<String>,
    pub credential_expires_at: i64,
    pub challenge_expires_at: i64,
}
impl EnrollmentRequest {
    /// Stable normalization is also usable for original-intent recovery.
    /// It does not assert that an expired challenge can create a new grant.
    pub fn normalize(&mut self) -> Result<()> {
        crate::transitions::id(&self.completion_operation_id)?;
        crate::transitions::id(&self.key_id)?;
        if self.operation_id == self.completion_operation_id
            || self.challenge_expires_at <= 0
            || self.challenge_expires_at > self.credential_expires_at
            || self.credential_expires_at > MAX_TIMESTAMP
        {
            return Err(invalid());
        }
        let mut scope = CredentialIssueRequest {
            operation_id: self.operation_id.clone(),
            identity_id: self.identity_id.clone(),
            operations: self.operations.clone(),
            recipients: self.recipients.clone(),
            expires_at: self.credential_expires_at,
        };
        scope.normalize()?;
        let mut endpoint = ProvisioningCommand::Endpoint {
            operation_id: self.operation_id.clone(),
            identity_id: self.identity_id.clone(),
            device: self.device.clone(),
            transport: self.transport.endpoint_transport(),
            address: self.endpoint_address.clone(),
            capabilities: self.capabilities.clone(),
            secret_ref: None,
        };
        endpoint.normalize()?;
        let ProvisioningCommand::Endpoint {
            address,
            capabilities,
            ..
        } = endpoint
        else {
            return Err(invalid());
        };
        let expected = match self.algorithm {
            SigningAlgorithm::EdDSA => 32,
            SigningAlgorithm::ES256 => 65,
        };
        let key = canonical_bytes(&self.public_key, expected)?;
        if self.algorithm == SigningAlgorithm::ES256 && key.first() != Some(&4) {
            return Err(invalid());
        }
        match (&self.transport, &self.browser_origin) {
            (EnrollmentTransport::Quic, None) => (),
            (EnrollmentTransport::Webtransport | EnrollmentTransport::WssWebrtc, Some(origin)) => {
                exact_origin(origin)?;
            }
            _ => return Err(invalid()),
        }
        self.operations = scope.operations;
        self.recipients = scope.recipients;
        self.endpoint_address = address;
        self.capabilities = capabilities;
        Ok(())
    }

    /// Temporal admission for a new challenge only. Exact completed recovery
    /// remains a Store decision under current issuer, credential and key state.
    pub fn validate_new_at(&self, now: i64) -> Result<()> {
        let mut normalized = self.clone();
        normalized.normalize()?;
        let maximum = if self.transport == EnrollmentTransport::Quic {
            NATIVE_CREDENTIAL_TTL_SECONDS
        } else {
            BROWSER_CREDENTIAL_TTL_SECONDS
        };
        if now <= 0
            || now > MAX_TIMESTAMP
            || self.challenge_expires_at <= now
            || self.challenge_expires_at - now > CHALLENGE_TTL_SECONDS
            || self.credential_expires_at <= now
            || self.credential_expires_at - now > maximum
        {
            return Err(invalid());
        }
        Ok(())
    }
}

/// Protected backend offer. Its challenge alone never proves device possession.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentOffer {
    pub profile: String,
    pub enrollment_id: String,
    pub endpoint_id: String,
    pub installation_id: String,
    pub issuer_identity_id: String,
    pub request: EnrollmentRequest,
    pub challenge: String,
}
impl EnrollmentOffer {
    pub fn normalize(&mut self) -> Result<()> {
        if self.profile != PROFILE || !challenge(&self.challenge) {
            return Err(invalid());
        }
        for id in [
            &self.enrollment_id,
            &self.endpoint_id,
            &self.installation_id,
            &self.issuer_identity_id,
        ] {
            crate::transitions::id(id)?;
        }
        self.request.normalize()
    }

    /// Sign these exact bytes using Ed25519 or ES256 (SHA-256, raw r||s).
    /// Domain separation plus the canonical offer digest commits every approved
    /// field and challenge without returning plaintext challenge bytes here.
    pub fn proof_message(&self) -> Result<Vec<u8>> {
        let mut value = self.clone();
        value.normalize()?;
        let json = serde_jcs::to_vec(&value).map_err(|_| invalid())?;
        if json.len() > 65536 {
            return Err(invalid());
        }
        let mut message = PROOF_DOMAIN.to_vec();
        message.extend_from_slice(&Sha256::digest(json));
        Ok(message)
    }
}

/// Unauthenticated claim syntax. Binding checks are not signature verification.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentClaim {
    pub operation_id: String,
    pub enrollment_id: String,
    pub identity_id: String,
    pub device_id: String,
    pub endpoint_id: String,
    pub key_id: String,
    pub transport: EnrollmentTransport,
    pub browser_origin: Option<String>,
    pub challenge: String,
    /// Canonical unpadded base64url; Ed25519/ES256 signatures are exactly64 bytes.
    pub signature: String,
}
impl EnrollmentClaim {
    /// Compare with the saved offer and actual trusted HTTP Origin context.
    /// The caller must still cryptographically verify the returned signature
    /// against offer.proof_message() and the approved key before any effect.
    pub fn check_binding(&self, offer: &EnrollmentOffer, origin: Option<&str>) -> Result<Vec<u8>> {
        let mut normalized = offer.clone();
        normalized.normalize()?;
        if self.operation_id != offer.request.completion_operation_id
            || self.enrollment_id != offer.enrollment_id
            || self.identity_id != offer.request.identity_id
            || self.device_id != offer.request.device.id
            || self.endpoint_id != offer.endpoint_id
            || self.key_id != offer.request.key_id
            || self.transport != offer.request.transport
            || self.browser_origin != offer.request.browser_origin
            || self.browser_origin.as_deref() != origin
            || !challenge(&self.challenge)
            || self
                .challenge
                .as_bytes()
                .iter()
                .zip(offer.challenge.as_bytes())
                .fold(0u8, |difference, (a, b)| difference | (a ^ b))
                != 0
        {
            return Err(Error::forbidden());
        }
        canonical_bytes(&self.signature, 64)
    }
}

/// Protected client response after verified admission; never a public journal receipt.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentCredential {
    pub enrollment_id: String,
    pub endpoint_id: String,
    pub device_id: String,
    pub key_id: String,
    pub transport: EnrollmentTransport,
    pub browser_origin: Option<String>,
    pub credential: crate::credential_issuance::CredentialIssueOutcome,
}
impl std::fmt::Debug for EnrollmentCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnrollmentCredential(redacted)")
    }
}

/// Non-secret current revocation projection for a trusted backend.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRevocation {
    pub enrollment_id: String,
    pub credential_id: Option<String>,
    pub revoked: bool,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentRevokeRequest {
    pub operation_id: String,
}

/// Protected HTTP result. Successful offers and credentials must stay private.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentResponse<T> {
    pub profile: String,
    pub result: Result<T>,
}
impl<T> std::fmt::Debug for EnrollmentResponse<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnrollmentResponse(redacted)")
    }
}

fn canonical_bytes(value: &str, expected: usize) -> Result<Vec<u8>> {
    if value.len() > 128 {
        return Err(invalid());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
    if bytes.len() != expected || URL_SAFE_NO_PAD.encode(&bytes) != value {
        return Err(invalid());
    }
    Ok(bytes)
}
fn challenge(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn exact_origin(value: &str) -> Result<()> {
    if value.len() > 2048 || !value.is_ascii() || value.contains('*') {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || url.origin().ascii_serialization() != value
    {
        return Err(invalid());
    }
    Ok(())
}
macro_rules! redacted {
    ($($name:ty),*) => {$ (
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(redacted)"))
            }
        }
    )*};
}
redacted!(EnrollmentRequest, EnrollmentOffer, EnrollmentClaim);
