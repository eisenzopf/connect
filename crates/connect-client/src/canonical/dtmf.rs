//! Native signal requests retain one logical ID through local uncertainty.
use super::*;
impl CanonicalClient {
    /// Submit a bounded signal on this physical native source. Preparation is
    /// durable acceptance; reconcile the operation for each local submission
    /// outcome. Neither completion nor dtmf.received proves remote playback.
    pub async fn send_native_dtmf(
        &mut self,
        scope: CommandScope,
        digits: &str,
        duration_ms: u32,
        method: &str,
    ) -> Result<payloads::OperationUpdated> {
        let operation = scope
            .operation_id
            .as_deref()
            .ok_or_else(|| super::calls::invalid("Native DTMF needs an operation ID"))?;
        let payload = json!({"digits":digits,"duration_ms":duration_ms,"method":method});
        connect_protocol::native_streams::Intent {
            operation_id: operation.into(),
            conversation_id: scope.conversation_id.clone().ok_or(ClientError::Protocol)?,
            session_id: scope.session_id.clone().ok_or(ClientError::Protocol)?,
            connection_id: scope.connection_id.clone().ok_or(ClientError::Protocol)?,
            expected_revision: scope.expected_revision,
            by_participant: None,
            command: connect_protocol::native_streams::Command::Dtmf,
            payload: payload.clone(),
        }
        .validate()
        .map_err(|_| super::calls::invalid("Invalid native DTMF intent"))?;
        let batch = self
            .request_value("dtmf.send", payload, scope.clone())
            .await?;
        let outcome = super::calls::operation_reply(&batch, operation)?;
        if outcome.state == payloads::OperationState::Completed {
            let result = outcome.result.as_ref().ok_or(ClientError::Protocol)?;
            if result["connection_id"].as_str() != scope.connection_id.as_deref()
                || result["local_submission_only"] != true
                || result["local_work_complete"] != true
                || result["resources_changed"] != false
                || result["signal_source"] != "uctp-native"
                || result["source_observation_id"].as_str().is_none()
                || result["recipients"].as_array().is_none_or(|r| {
                    r.is_empty()
                        || r.iter().any(|r| {
                            r["disposition"] != "local_submitted"
                                || r["frame_id"].as_str().is_none()
                        })
                })
            {
                return Err(ClientError::Protocol);
            }
        }
        Ok(outcome)
    }
}
