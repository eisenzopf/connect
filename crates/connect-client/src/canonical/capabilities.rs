//! Endpoint declarations retain exact logical intent and never grant media.
use super::*;
impl CanonicalClient {
    /// Replace this actual physical peer's advertised descriptor. Preserve the
    /// original scope, descriptor, participant and trigger after uncertainty.
    /// Completion confirms server receipt, not renegotiation or peer delivery.
    pub async fn advertise_native_capabilities(
        &mut self,
        scope: CommandScope,
        by_participant: &str,
        capabilities: Value,
        trigger: &str,
    ) -> Result<payloads::OperationUpdated> {
        let operation = scope.operation_id.as_deref().ok_or_else(|| {
            super::calls::invalid("Capability advertisement needs an operation ID")
        })?;
        let payload =
            json!({"by_participant":by_participant,"capabilities":capabilities,"trigger":trigger});
        connect_protocol::native_streams::Intent {
            operation_id: operation.into(),
            conversation_id: scope.conversation_id.clone().ok_or(ClientError::Protocol)?,
            session_id: scope.session_id.clone().ok_or(ClientError::Protocol)?,
            connection_id: scope.connection_id.clone().ok_or(ClientError::Protocol)?,
            expected_revision: scope.expected_revision,
            by_participant: Some(by_participant.into()),
            command: connect_protocol::native_streams::Command::Advertise,
            payload: payload.clone(),
        }
        .validate()
        .map_err(|_| super::calls::invalid("Invalid native capability advertisement"))?;
        let batch = self
            .request_value("capability.advertise", payload, scope.clone())
            .await?;
        let outcome = super::calls::operation_reply(&batch, operation)?;
        if outcome.state == payloads::OperationState::Completed {
            let result = outcome.result.as_ref().ok_or(ClientError::Protocol)?;
            if outcome.problem.is_some()
                || outcome.record_id.is_none()
                || result["connection_id"].as_str() != scope.connection_id.as_deref()
                || result["connection_generation"]
                    .as_u64()
                    .is_none_or(|n| n == 0 || n > 9_007_199_254_740_991)
                || result["stream_revision"].as_u64() != scope.expected_revision
                || result["resources_changed"] != false
                || result["by_participant"] != by_participant
                || result["capabilities"] != capabilities
                || result["trigger"] != trigger
                || result["advertisement_record_id"].as_str()
                    != outcome.record_id.as_ref().map(|s| s.as_str())
            {
                return Err(ClientError::Protocol);
            }
        }
        Ok(outcome)
    }
}
