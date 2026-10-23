//! Replies to received server-owned invitations are observations, not commands.
use super::*;
#[derive(Clone)]
pub enum SessionInvitationAnswer {
    Accept { capabilities: Value },
    Reject { reason_code: u64, reason: String },
}
impl std::fmt::Debug for SessionInvitationAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionInvitationAnswer(redacted)")
    }
}
impl CanonicalClient {
    /// Cancel this physical caller's original invitation. Keep the same logical
    /// cancellation operation across uncertainty; a replayed invitation supplies
    /// a new current wire reference after reconnect. A reference grants no rights.
    pub async fn cancel_session_invitation(
        &mut self,
        original_request_id: &str,
        mut request: connect_protocol::session_control::Request,
    ) -> Result<connect_protocol::session_decisions::DecisionOutcome> {
        request.normalize()?;
        let connect_protocol::session_control::Action::Cancel {
            reason_code,
            reason,
        } = &request.action
        else {
            return Err(ClientError::Protocol);
        };
        let mut payload = json!({"reason_code":reason_code,"reason":reason});
        if let Some(by) = &request.by_participant {
            payload["by"] = json!(by);
        }
        let batch = self
            .request_correlated_value(
                "session.cancel",
                payload,
                CommandScope {
                    conversation_id: Some(request.conversation_id.clone()),
                    session_id: Some(request.session_id.clone()),
                    operation_id: Some(request.operation_id.clone()),
                    expected_revision: request.expected_revision,
                    ..Default::default()
                },
                tokio_util::sync::CancellationToken::new(),
                Some(original_request_id),
            )
            .await?;
        let update = calls::operation_reply(&batch, &request.operation_id)?;
        if update.state != payloads::OperationState::Completed || update.problem.is_some() {
            return Err(ClientError::Protocol);
        }
        let result: connect_protocol::Outcome =
            serde_json::from_value(update.result.ok_or(ClientError::Protocol)?)
                .map_err(|_| ClientError::Protocol)?;
        let connect_protocol::Outcome::SessionDecision(result) = result else {
            return Err(ClientError::Protocol);
        };
        if result.operation_id != request.operation_id
            || result.conversation_id != request.conversation_id
            || result.session_id != request.session_id
            || result.state != "cancelled"
            || result.reason_code != *reason_code
            || result.reason != *reason
            || result.revision > 9_007_199_254_740_991
            || request
                .expected_revision
                .is_some_and(|v| v.checked_add(1) != Some(result.revision))
        {
            return Err(ClientError::Protocol);
        }
        Ok(result)
    }
    /// Receive a signed server-owned invitation. Other facts/replies remain in
    /// the shared bounded inbox for their normal receive/request APIs.
    pub async fn receive_session_invitation(&mut self) -> Result<Envelope> {
        if let Some(index) = self
            .events
            .iter()
            .position(|event| event.kind() == "session.invite")
        {
            let event = self.events.remove(index).ok_or(ClientError::Protocol)?;
            self.event_bytes = self.event_bytes.saturating_sub(encoded_size(&event)?);
            return Ok(event);
        }
        loop {
            let event = self.receive_wire().await?;
            if event.kind() == "session.invite" {
                return Ok(event);
            }
            self.queue_event(event)?;
        }
    }
    /// Answer a previously received, signed server invitation on this physical
    /// peer. Returns only the fresh transport envelope ID, not a durable outcome.
    /// Observe the authorized journal/snapshot for the actual Session decision.
    /// No caller command/operation identity is allocated for this observation.
    pub async fn answer_session_invitation(
        &mut self,
        invitation: &Envelope,
        answer: SessionInvitationAnswer,
    ) -> Result<String> {
        let Payload::SessionInvite(request) = invitation.payload() else {
            return Err(ClientError::Protocol);
        };
        if invitation.scope(Scope::Reply).is_some()
            || invitation.scope(Scope::Connection).is_some()
            || invitation.scope(Scope::Conversation).is_none()
            || invitation.scope(Scope::Session).is_none()
            || invitation.scope(Scope::Operation).is_none()
            || request.r#from.is_none()
            || request.to.len() != 1
            || invitation.original().get("signature").is_none()
        {
            return Err(ClientError::Protocol);
        }
        // receive() already verified and consumed the server signature's replay
        // ID. Re-verifying here would reject that valid received observation.
        let by = &request.to[0];
        let (kind, payload) = match answer {
            SessionInvitationAnswer::Accept { capabilities } => {
                connect_protocol::session_control::validate_capabilities(&capabilities)
                    .map_err(|_| ClientError::Protocol)?;
                (
                    "session.accept",
                    json!({"by":by,"capabilities_answer":capabilities}),
                )
            }
            SessionInvitationAnswer::Reject {
                reason_code,
                reason,
            } => {
                if reason_code > 65535 || reason.len() > 256 || reason.chars().any(char::is_control)
                {
                    return Err(ClientError::Protocol);
                }
                (
                    "session.reject",
                    json!({"by":by,"reason_code":reason_code,"reason":reason}),
                )
            }
        };
        let mut value = envelope(kind, payload);
        value["in_reply_to"] = json!(invitation.id());
        for field in ["cid", "sid", "operation_id"] {
            value[field] = invitation.original()[field].clone();
        }
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        self.write(value, true).await?;
        Ok(id)
    }
}
