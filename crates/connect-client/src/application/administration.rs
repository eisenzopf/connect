//! Trusted administration over the documented, explicitly selected HTTP API.
use super::{provisioning::*, sanitized_error, ApplicationClient};
use crate::{ClientError, Result};
use tokio_util::sync::CancellationToken;

impl ApplicationClient {
    /// Execute one trusted administrative mutation with a caller-owned logical
    /// operation ID. Save and reuse the complete command after uncertainty.
    /// The server enforces current administrator and target authority on replay.
    pub async fn provision(&mut self, command: ProvisioningCommand) -> Result<ProvisioningOutcome> {
        self.provision_with_cancel(command, CancellationToken::new())
            .await
    }

    pub async fn provision_with_cancel(
        &mut self,
        mut command: ProvisioningCommand,
        cancellation: CancellationToken,
    ) -> Result<ProvisioningOutcome> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        command.normalize().map_err(sanitized_error)?;
        let path = route(&command)?;
        let body = serde_json::to_vec(&command).map_err(|_| ClientError::Protocol)?;
        self.transact(&path, body, 0, cancellation, |status, bytes| {
            let response: ProvisioningResponse =
                serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
            if response.profile != PROVISIONING_PROFILE {
                return Err(ClientError::Protocol);
            }
            let result = response.result.map_err(sanitized_error)?;
            if !status.is_success() || !matches_command(&command, &result) {
                return Err(ClientError::Protocol);
            }
            Ok(result)
        })
        .await
    }
}

// normalize() validates every interpolated identifier before route construction.
fn route(command: &ProvisioningCommand) -> Result<String> {
    let target = match command {
        ProvisioningCommand::Identity { .. } => None,
        ProvisioningCommand::Participant {
            conversation_id, ..
        } => Some(conversation_id),
        ProvisioningCommand::RecordReadRole { identity_id, .. }
        | ProvisioningCommand::Endpoint { identity_id, .. }
        | ProvisioningCommand::SigningKey { identity_id, .. }
        | ProvisioningCommand::RevokeIdentity { identity_id, .. } => Some(identity_id),
        ProvisioningCommand::VerifyEndpoint { endpoint_id, .. }
        | ProvisioningCommand::RevokeEndpoint { endpoint_id, .. } => Some(endpoint_id),
        ProvisioningCommand::RevokeSigningKey { key_id, .. } => Some(key_id),
        ProvisioningCommand::RevokeDevice { device_id, .. } => Some(device_id),
        ProvisioningCommand::RevokeCredential { credential_id, .. } => Some(credential_id),
    };
    // The shared identifier alphabet allows these values, but HTTP URL parsing
    // normalizes them as path segments. Never send a different route by accident.
    if target.is_some_and(|value| matches!(value.as_str(), "." | "..")) {
        return Err(ClientError::Application(connect_protocol::Error::new(
            "invalid-request",
            "Provisioning target cannot be represented as an HTTP route",
        )));
    }
    Ok(match command {
        ProvisioningCommand::Identity { .. } => "/v2/identities".into(),
        ProvisioningCommand::RecordReadRole { identity_id, .. } => {
            format!("/v2/identities/{identity_id}/record-read-role")
        }
        ProvisioningCommand::RevokeCredential { credential_id, .. } => {
            format!("/v2/credentials/{credential_id}/revoke")
        }
        ProvisioningCommand::Participant {
            conversation_id, ..
        } => format!("/v2/conversations/{conversation_id}/participants"),
        ProvisioningCommand::Endpoint { identity_id, .. } => {
            format!("/v2/identities/{identity_id}/endpoints")
        }
        ProvisioningCommand::VerifyEndpoint { endpoint_id, .. } => {
            format!("/v2/endpoints/{endpoint_id}/verify")
        }
        ProvisioningCommand::RevokeEndpoint { endpoint_id, .. } => {
            format!("/v2/endpoints/{endpoint_id}/revoke")
        }
        ProvisioningCommand::RevokeIdentity { identity_id, .. } => {
            format!("/v2/identities/{identity_id}/revoke")
        }
        ProvisioningCommand::SigningKey { identity_id, .. } => {
            format!("/v2/identities/{identity_id}/signing-keys")
        }
        ProvisioningCommand::RevokeSigningKey { key_id, .. } => {
            format!("/v2/signing-keys/{key_id}/revoke")
        }
        ProvisioningCommand::RevokeDevice { device_id, .. } => {
            format!("/v2/devices/{device_id}/revoke")
        }
    })
}

// A replay projects current state, so revisions may advance and enrollment may
// subsequently be revoked. Correlate immutable targets without reviving state.
fn matches_command(command: &ProvisioningCommand, outcome: &ProvisioningOutcome) -> bool {
    match (command, outcome) {
        (
            ProvisioningCommand::RecordReadRole {
                identity_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::RecordReadRole {
                identity_id: actual,
                revision,
                ..
            },
        ) => identity_id == actual && *revision > expected_revision.unwrap_or(0),
        (
            ProvisioningCommand::RevokeCredential { credential_id, .. },
            ProvisioningOutcome::Credential {
                credential_id: actual,
                revoked,
                ..
            },
        ) => credential_id == actual && *revoked,
        (
            ProvisioningCommand::Identity { alias, kind, .. },
            ProvisioningOutcome::Identity {
                alias: actual,
                identity_kind,
                ..
            },
        ) => alias == actual && kind == identity_kind,
        (
            ProvisioningCommand::Participant {
                conversation_id,
                identity_id,
                ..
            },
            ProvisioningOutcome::Participant {
                conversation_id: cid,
                identity_id: owner,
                ..
            },
        ) => conversation_id == cid && identity_id == owner,
        (
            ProvisioningCommand::Endpoint {
                identity_id,
                device,
                transport,
                ..
            },
            ProvisioningOutcome::Endpoint {
                identity_id: owner,
                device_id,
                transport: actual,
                ..
            },
        ) => identity_id == owner && device.id == *device_id && transport == actual,
        (
            ProvisioningCommand::VerifyEndpoint {
                endpoint_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::Endpoint {
                endpoint_id: actual,
                revision,
                ..
            },
        ) => endpoint_id == actual && revision > expected_revision,
        (
            ProvisioningCommand::RevokeEndpoint {
                endpoint_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::Endpoint {
                endpoint_id: actual,
                revision,
                verification_state,
                ..
            },
        ) => {
            endpoint_id == actual && revision > expected_revision && verification_state == "revoked"
        }
        (
            ProvisioningCommand::RevokeIdentity {
                identity_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::Identity {
                identity_id: actual,
                revision,
                revoked,
                ..
            },
        ) => identity_id == actual && revision > expected_revision && *revoked,
        (
            ProvisioningCommand::SigningKey {
                key_id,
                identity_id,
                device_id,
                algorithm,
                not_before,
                expires_at,
                ..
            },
            ProvisioningOutcome::SigningKey {
                key_id: actual,
                identity_id: owner,
                device_id: device,
                algorithm: alg,
                not_before: start,
                expires_at: end,
                ..
            },
        ) => {
            key_id == actual
                && identity_id == owner
                && device_id == device
                && algorithm == alg
                && not_before == start
                && expires_at == end
        }
        (
            ProvisioningCommand::RevokeSigningKey {
                key_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::SigningKey {
                key_id: actual,
                revision,
                revoked,
                ..
            },
        ) => key_id == actual && revision > expected_revision && *revoked,
        (
            ProvisioningCommand::RevokeDevice {
                device_id,
                expected_revision,
                ..
            },
            ProvisioningOutcome::Device {
                device_id: actual,
                revision,
                revoked,
                ..
            },
        ) => device_id == actual && revision > expected_revision && *revoked,
        _ => false,
    }
}
