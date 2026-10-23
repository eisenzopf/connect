//! Explicit current local erasure; retain the complete request on uncertainty.
use super::{sanitized_error, ApplicationClient};
use crate::{ClientError, Result};
use connect_protocol::record_mutations::{Action, Outcome, Request, Response};
use tokio_util::sync::CancellationToken;
impl ApplicationClient {
    pub async fn mutate_record(&mut self, request: Request) -> Result<Outcome> {
        self.mutate_record_with_cancel(request, CancellationToken::new())
            .await
    }
    pub async fn mutate_record_with_cancel(
        &mut self,
        request: Request,
        cancel: CancellationToken,
    ) -> Result<Outcome> {
        if cancel.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if self.task_profile {
            return Err(ClientError::Configuration);
        }
        request.validate().map_err(sanitized_error)?;
        let path = format!("/v2/records/{}/mutations", request.uuid);
        let body = serde_json::to_vec(&request).map_err(|_| ClientError::Protocol)?;
        self.transact(&path, body, 0, cancel, |status, bytes| {
            let response: Response =
                serde_json::from_slice(bytes).map_err(|_| ClientError::Protocol)?;
            if response.profile != "connect/2" {
                return Err(ClientError::Protocol);
            }
            let result = response.result.map_err(sanitized_error)?;
            let action = match request.action {
                Action::Redact { .. } => "redact",
                Action::Delete => "delete",
            };
            if !status.is_success()
                || result.operation_id != request.operation_id
                || result.uuid != request.uuid
                || result.revision != request.expected_revision + 1
                || result.action != action
            {
                return Err(ClientError::Protocol);
            }
            Ok(result)
        })
        .await
    }
}
