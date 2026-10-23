//! Quality configuration is a logical operation, independent of media bindings.
use super::*;
impl CanonicalClient {
    /// Configure this physical peer's exact native Connection. Preserve the
    /// original scope, revision, interval and operation ID after uncertainty.
    /// Completion confirms local configuration, never remote receipt or playback.
    pub async fn configure_native_quality(
        &mut self,
        scope: CommandScope,
        interval_ms: u64,
    ) -> Result<payloads::OperationUpdated> {
        let operation = scope
            .operation_id
            .as_deref()
            .ok_or_else(|| super::calls::invalid("Quality configuration needs an operation ID"))?;
        if !(100..=60_000).contains(&interval_ms)
            || scope.expected_revision.is_none()
            || scope.conversation_id.is_none()
            || scope.session_id.is_none()
            || scope.connection_id.is_none()
        {
            return Err(super::calls::invalid(
                "Invalid native quality configuration scope or interval",
            ));
        }
        let batch = self
            .request_value(
                "connection.update",
                json!({"action":"quality","quality_interval_ms":interval_ms}),
                scope.clone(),
            )
            .await?;
        let outcome = super::calls::operation_reply(&batch, operation)?;
        if outcome.state == payloads::OperationState::Completed {
            let result = outcome.result.as_ref().ok_or(ClientError::Protocol)?;
            if outcome.problem.is_some()
                || result["connection_generation"]
                    .as_u64()
                    .is_none_or(|n| n == 0 || n > 9_007_199_254_740_991)
                || result["connection_id"].as_str() != scope.connection_id.as_deref()
                || result["stream_revision"].as_u64() != scope.expected_revision
                || result["quality_interval_ms"] != interval_ms
                || result["resources_changed"] != false
            {
                return Err(ClientError::Protocol);
            }
        }
        Ok(outcome)
    }
}
