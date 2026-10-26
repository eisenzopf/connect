//! Trusted administrative provisioning. Public communication clients cannot
//! grant endpoint ownership or verification by supplying these payloads.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

pub const PROVISIONING_PROFILE: &str = "connect/2";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SigningAlgorithm {
    EdDSA,
    ES256,
}
impl SigningAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EdDSA => "EdDSA",
            Self::ES256 => "ES256",
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Human,
    Ai,
    System,
    External,
}
impl IdentityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Ai => "ai",
            Self::System => "system",
            Self::External => "external",
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointTransport {
    Quic,
    Webtransport,
    Webrtc,
    Sip,
    Telephone,
    Sms,
    VapiVoice,
    VapiChat,
}
impl EndpointTransport {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Quic => "quic",
            Self::Webtransport => "webtransport",
            Self::Webrtc => "webrtc",
            Self::Sip => "sip",
            Self::Telephone => "telephone",
            Self::Sms => "sms",
            Self::VapiVoice => "vapi-voice",
            Self::VapiChat => "vapi-chat",
        }
    }
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    pub id: String,
    pub kind: String,
    pub platform: String,
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProvisioningCommand {
    RecordReadRole {
        operation_id: String,
        identity_id: String,
        role: String,
        /// Explicit tenant policy: include sources restricted to Participants.
        /// Credential recipient ceilings still apply independently.
        private_sources: bool,
        active: bool,
        expected_revision: Option<u64>,
    },
    Identity {
        operation_id: String,
        alias: String,
        kind: IdentityKind,
        display_name: Option<String>,
    },
    Participant {
        operation_id: String,
        conversation_id: String,
        identity_id: String,
        role: String,
    },
    Endpoint {
        operation_id: String,
        identity_id: String,
        device: DeviceSpec,
        transport: EndpointTransport,
        address: String,
        capabilities: Vec<String>,
        /// A server-owned secret reference, never credential plaintext.
        secret_ref: Option<String>,
    },
    VerifyEndpoint {
        operation_id: String,
        endpoint_id: String,
        expected_revision: u64,
        /// Attributed proof reference supplied by the trusted backend.
        evidence_ref: String,
    },
    RevokeEndpoint {
        operation_id: String,
        endpoint_id: String,
        expected_revision: u64,
    },
    RevokeIdentity {
        operation_id: String,
        identity_id: String,
        expected_revision: u64,
    },
    SigningKey {
        operation_id: String,
        identity_id: String,
        credential_id: String,
        device_id: String,
        key_id: String,
        algorithm: SigningAlgorithm,
        /// Canonical base64url without padding; public verification material only.
        public_key: String,
        not_before: i64,
        expires_at: i64,
        /// None creates a key; binding another credential requires its current revision.
        expected_revision: Option<u64>,
    },
    RevokeSigningKey {
        operation_id: String,
        key_id: String,
        expected_revision: u64,
    },
    RevokeDevice {
        operation_id: String,
        device_id: String,
        expected_revision: u64,
    },
    /// Credential IDs are immutable; revocation is terminal and needs no revision.
    RevokeCredential {
        operation_id: String,
        credential_id: String,
    },
}
impl std::fmt::Debug for ProvisioningCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.command_type())
    }
}
impl ProvisioningCommand {
    /// Normalize routing identity before hashing or reserving an address.
    pub fn normalize(&mut self) -> Result<()> {
        self.validate()?;
        if let Self::Endpoint {
            address,
            transport,
            capabilities,
            ..
        } = self
        {
            *address = canonical_address(transport, address)?;
            capabilities.sort();
            capabilities.dedup();
        }
        Ok(())
    }
    pub fn operation_id(&self) -> &str {
        match self {
            Self::RecordReadRole { operation_id, .. } => operation_id,
            Self::Identity { operation_id, .. }
            | Self::Participant { operation_id, .. }
            | Self::Endpoint { operation_id, .. }
            | Self::VerifyEndpoint { operation_id, .. }
            | Self::RevokeEndpoint { operation_id, .. }
            | Self::SigningKey { operation_id, .. }
            | Self::RevokeSigningKey { operation_id, .. }
            | Self::RevokeDevice { operation_id, .. }
            | Self::RevokeCredential { operation_id, .. }
            | Self::RevokeIdentity { operation_id, .. } => operation_id,
        }
    }
    pub fn command_type(&self) -> &'static str {
        match self {
            Self::RecordReadRole { .. } => "record-read-role.configure",
            Self::Identity { .. } => "identity.provision",
            Self::Participant { .. } => "participant.provision",
            Self::Endpoint { .. } => "endpoint.provision",
            Self::VerifyEndpoint { .. } => "endpoint.verify",
            Self::RevokeEndpoint { .. } => "endpoint.revoke",
            Self::RevokeIdentity { .. } => "identity.revoke",
            Self::SigningKey { .. } => "signing-key.enroll",
            Self::RevokeSigningKey { .. } => "signing-key.revoke",
            Self::RevokeDevice { .. } => "device.revoke",
            Self::RevokeCredential { .. } => "credential.revoke",
        }
    }
    pub fn validate(&self) -> Result<()> {
        identifier(self.operation_id())?;
        match self {
            Self::RecordReadRole {
                identity_id,
                role,
                expected_revision,
                ..
            } => {
                identifier(identity_id)?;
                identifier(role)?;
                if role.len() > 64 {
                    return Err(invalid());
                }
                if let Some(value) = expected_revision {
                    revision(*value)?;
                }
            }
            Self::RevokeCredential { credential_id, .. } => identifier(credential_id)?,
            Self::SigningKey {
                identity_id,
                credential_id,
                device_id,
                key_id,
                public_key,
                algorithm,
                not_before,
                expires_at,
                expected_revision,
                ..
            } => {
                use base64::Engine;
                for value in [identity_id, credential_id, device_id, key_id] {
                    identifier(value)?;
                }
                let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(public_key)
                    .map_err(|_| invalid())?;
                if public_key.len() > 128
                    || base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
                        != *public_key
                    || bytes.len()
                        != match algorithm {
                            SigningAlgorithm::EdDSA => 32,
                            SigningAlgorithm::ES256 => 65,
                        }
                    || *not_before < 0
                    || expires_at <= not_before
                    || *expires_at > 9_007_199_254_740_991
                {
                    return Err(invalid());
                }
                if let Some(value) = expected_revision {
                    revision(*value)?;
                }
            }
            Self::RevokeSigningKey {
                key_id,
                expected_revision,
                ..
            } => {
                identifier(key_id)?;
                revision(*expected_revision)?;
            }
            Self::RevokeDevice {
                device_id,
                expected_revision,
                ..
            } => {
                identifier(device_id)?;
                revision(*expected_revision)?;
            }
            Self::Identity {
                alias,
                display_name,
                ..
            } => {
                if alias.is_empty()
                    || alias.len() > 64
                    || !alias
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                {
                    return Err(invalid());
                }
                if display_name.as_ref().is_some_and(|s| {
                    s.is_empty() || s.len() > 256 || s.chars().any(char::is_control)
                }) {
                    return Err(invalid());
                }
            }
            Self::Participant {
                conversation_id,
                identity_id,
                role,
                ..
            } => {
                identifier(conversation_id)?;
                identifier(identity_id)?;
                identifier(role)?;
                if role.len() > 64 {
                    return Err(invalid());
                }
            }
            Self::Endpoint {
                identity_id,
                device,
                address,
                capabilities,
                secret_ref,
                transport,
                ..
            } => {
                identifier(identity_id)?;
                identifier(&device.id)?;
                identifier(&device.kind)?;
                if device.platform.is_empty()
                    || device.platform.len() > 128
                    || device.platform.chars().any(char::is_control)
                {
                    return Err(invalid());
                }
                if address.is_empty()
                    || address.len() > 2048
                    || !address.is_ascii()
                    || address
                        .bytes()
                        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
                {
                    return Err(invalid());
                }
                if capabilities.is_empty() || capabilities.len() > 32 {
                    return Err(invalid());
                }
                for capability in capabilities {
                    identifier(capability)?;
                }
                if let Some(reference) = secret_ref {
                    identifier(reference)?;
                }
                let valid = match transport {
                    EndpointTransport::Telephone | EndpointTransport::Sms => {
                        address.starts_with('+')
                            && (9..=16).contains(&address.len())
                            && address.as_bytes()[1] != b'0'
                            && address[1..].bytes().all(|b| b.is_ascii_digit())
                    }
                    EndpointTransport::Sip => {
                        (address.starts_with("sip:") || address.starts_with("sips:"))
                            && address.split('@').count() == 2
                            && !address.ends_with('@')
                            && !address.contains('?')
                    }
                    EndpointTransport::Quic => {
                        address.starts_with("quic://")
                            && address.len() > 7
                            && !address.contains('@')
                    }
                    EndpointTransport::Webtransport | EndpointTransport::Webrtc => {
                        address.starts_with("https://")
                            && address.len() > 8
                            && !address.contains('@')
                    }
                    EndpointTransport::VapiVoice | EndpointTransport::VapiChat => {
                        identifier(address)?;
                        true
                    }
                };
                if !valid {
                    return Err(invalid());
                }
                canonical_address(transport, address)?;
                if matches!(
                    transport,
                    EndpointTransport::Sip
                        | EndpointTransport::Telephone
                        | EndpointTransport::Sms
                        | EndpointTransport::VapiVoice
                        | EndpointTransport::VapiChat
                ) && secret_ref.is_none()
                {
                    return Err(invalid());
                }
            }
            Self::VerifyEndpoint {
                endpoint_id,
                expected_revision,
                evidence_ref,
                ..
            } => {
                identifier(endpoint_id)?;
                revision(*expected_revision)?;
                identifier(evidence_ref)?;
            }
            Self::RevokeEndpoint {
                endpoint_id,
                expected_revision,
                ..
            } => {
                identifier(endpoint_id)?;
                revision(*expected_revision)?;
            }
            Self::RevokeIdentity {
                identity_id,
                expected_revision,
                ..
            } => {
                identifier(identity_id)?;
                revision(*expected_revision)?;
            }
        }
        Ok(())
    }
}
fn canonical_address(transport: &EndpointTransport, address: &str) -> Result<String> {
    match transport {
        EndpointTransport::Quic | EndpointTransport::Webtransport | EndpointTransport::Webrtc => {
            let value = url::Url::parse(address).map_err(|_| invalid())?;
            if value.host_str().is_none()
                || !value.username().is_empty()
                || value.password().is_some()
                || value.query().is_some()
                || value.fragment().is_some()
                || (matches!(transport, EndpointTransport::Quic) && value.port().is_none())
            {
                return Err(invalid());
            }
            Ok(value.to_string())
        }
        EndpointTransport::Sip => {
            match crate::session_control::Target::parse(address).map_err(|_| invalid())? {
                crate::session_control::Target::Sip { address, .. } => Ok(address),
                _ => Err(invalid()),
            }
        }
        _ => Ok(address.to_owned()),
    }
}
fn invalid() -> Error {
    Error::new("invalid-request", "Invalid trusted provisioning request")
}
fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._~-".contains(&b))
    {
        Err(invalid())
    } else {
        Ok(())
    }
}
fn revision(value: u64) -> Result<()> {
    if value >= 9_007_199_254_740_991 {
        Err(invalid())
    } else {
        Ok(())
    }
}

/// Administrative responses omit endpoint addresses and secret references.
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProvisioningOutcome {
    RecordReadRole {
        identity_id: String,
        tenant_id: String,
        role: String,
        private_sources: bool,
        active: bool,
        revision: u64,
    },
    Credential {
        credential_id: String,
        identity_id: String,
        expires_at: i64,
        revoked: bool,
    },
    SigningKey {
        key_id: String,
        identity_id: String,
        device_id: String,
        algorithm: SigningAlgorithm,
        not_before: i64,
        expires_at: i64,
        revision: u64,
        revoked: bool,
    },
    Device {
        device_id: String,
        identity_id: String,
        revision: u64,
        revoked: bool,
    },
    Identity {
        identity_id: String,
        alias: String,
        identity_kind: IdentityKind,
        revision: u64,
        revoked: bool,
    },
    Participant {
        participant_id: String,
        conversation_id: String,
        identity_id: String,
        membership_epoch: u64,
    },
    Endpoint {
        endpoint_id: String,
        identity_id: String,
        device_id: String,
        transport: EndpointTransport,
        capabilities: Vec<String>,
        verification_state: String,
        revision: u64,
    },
}

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Serialize, Deserialize)]
pub struct ProvisioningResponse {
    pub profile: String,
    pub result: Result<ProvisioningOutcome>,
}
