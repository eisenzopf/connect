use super::{sanitized_error, ApplicationClient};
use crate::{ClientError, Result};
use connect_protocol::credential_issuance::*;
use tokio_util::sync::CancellationToken;
impl ApplicationClient {
    /// Persist the complete original intent before IO. Explicit replay with the
    /// same current issuer recovers the same secret; no automatic retry occurs.
    pub async fn issue_scoped_credential(
        &mut self,
        request: CredentialIssueRequest,
    ) -> Result<CredentialIssueOutcome> {
        self.issue_scoped_credential_with_cancel(request, CancellationToken::new())
            .await
    }
    pub async fn issue_scoped_credential_with_cancel(
        &mut self,
        mut request: CredentialIssueRequest,
        cancel: CancellationToken,
    ) -> Result<CredentialIssueOutcome> {
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.task_profile {
            return Err(ClientError::Configuration);
        }
        request.normalize().map_err(sanitized_error)?;
        let body = serde_json::to_vec(&request).map_err(|_| ClientError::Protocol)?;
        self.transact("/v2/credentials", body, 0, cancel, |status, bytes| {
            let response: CredentialIssueResponse =
                serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
            if response.profile != PROFILE {
                return Err(ClientError::Protocol);
            }
            let result = response.result.map_err(sanitized_error)?;
            if status != reqwest::StatusCode::OK
                || result.operation_id != request.operation_id
                || result.identity_id != request.identity_id
                || result.operations != request.operations
                || result.recipients != request.recipients
                || result.expires_at != request.expires_at
                || result.issued_at <= 0
                || result.issued_at > chrono::Utc::now().timestamp()
                || result.issued_at >= result.expires_at
                || result.expires_at <= chrono::Utc::now().timestamp()
                || result.credential.len() != 64
                || !result
                    .credential
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || [
                    &result.credential_id,
                    &result.issuer_identity_id,
                    &result.issuer_credential_id,
                    &result.installation_id,
                ]
                .iter()
                .any(|id| uuid::Uuid::parse_str(id).is_err())
            {
                return Err(ClientError::Protocol);
            }
            Ok(result)
        })
        .await
    }
}
