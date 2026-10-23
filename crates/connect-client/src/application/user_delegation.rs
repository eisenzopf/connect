use super::{sanitized_error, ApplicationClient};
use crate::{ClientError, Result};
use connect_protocol::user_delegation::*;
use tokio_util::sync::CancellationToken;
impl ApplicationClient {
    /// Explicit human consent. Retain the complete request on uncertainty;
    /// retries recover the same secret and never extend consent's lifetime.
    pub async fn issue_user_delegation(
        &mut self,
        expected_user_id: &str,
        request: UserDelegationRequest,
    ) -> Result<UserDelegationOutcome> {
        self.issue_user_delegation_with_cancel(expected_user_id, request, CancellationToken::new())
            .await
    }
    pub async fn issue_user_delegation_with_cancel(
        &mut self,
        expected_user_id: &str,
        mut request: UserDelegationRequest,
        cancel: CancellationToken,
    ) -> Result<UserDelegationOutcome> {
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.task_profile || expected_user_id.is_empty() || expected_user_id.len() > 128 {
            return Err(ClientError::Configuration);
        }
        request.normalize().map_err(sanitized_error)?;
        let body = serde_json::to_vec(&request).map_err(|_| ClientError::Protocol)?;
        self.transact("/v2/user-delegations", body, 0, cancel, |status, bytes| {
            let response: UserDelegationResponse =
                serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
            if response.profile != "connect/2" {
                return Err(ClientError::Protocol);
            }
            let result = response.result.map_err(sanitized_error)?;
            if !status.is_success()
                || result.operation_id != request.operation_id
                || result.identity_id != request.identity_id
                || result.user_id != expected_user_id
                || result.operations != request.operations
                || result.recipients != request.recipients
                || result.expires_at != request.expires_at
                || result.credential.len() != 64
                || !result
                    .credential
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                || uuid::Uuid::parse_str(&result.credential_id).is_err()
            {
                return Err(ClientError::Protocol);
            }
            Ok(result)
        })
        .await
    }
}
