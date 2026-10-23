//! Explicit completion of one signed challenge on the original physical peer.
use super::*;

pub(super) struct Challenge {
    id: String,
    request_id: String,
    scope: CommandScope,
    required: String,
    deadline: tokio::time::Instant,
    assigned_conversation: bool,
}
impl Challenge {
    pub(super) fn new(
        frame: &Envelope,
        request_id: &str,
        scope: CommandScope,
        deadline: tokio::time::Instant,
        assigned_conversation: bool,
    ) -> Result<Self> {
        let Payload::IdentityStepUpRequest(request) = frame.payload() else {
            return Err(ClientError::Protocol);
        };
        if frame.scope(Scope::Reply) != Some(request_id)
            || !matches!(
                request.required.as_str(),
                "identified" | "task-scoped" | "user-authorized"
            )
            || request
                .allowed_methods
                .as_ref()
                .is_none_or(|methods| !methods.iter().any(|method| method == "bearer"))
        {
            return Err(ClientError::Authentication);
        }
        Ok(Self {
            id: frame.id().to_owned(),
            request_id: request_id.to_owned(),
            scope,
            required: request.required.clone(),
            deadline,
            assigned_conversation,
        })
    }
}

/// Authentication transition plus replies to the original, resumed command.
/// Inspect `replies.ack()` and operation outcomes separately from authentication.
pub struct StepUpCompletion {
    pub authentication: CanonicalAuthentication,
    pub replies: ReplyBatch,
}

impl CanonicalClient {
    /// Required assurance for the original suspended command. The caller must
    /// supply explicit current credentials; the SDK never issues or inherits one.
    pub fn step_up_required(&self) -> Option<&str> {
        self.step_up
            .as_ref()
            .map(|challenge| challenge.required.as_str())
    }

    /// Complete the challenge returned by this peer's most recent request.
    /// The caller supplies a successor bearer; no credential is issued, selected
    /// or inherited. The held original command is resumed by the server without
    /// sending it again. The original command deadline still applies.
    /// Owned native audio retains its single verified reader during this
    /// explicitly correlated transition; no successor credential is inferred.
    pub async fn complete_step_up(&mut self, credential: String) -> Result<StepUpCompletion> {
        self.complete_step_up_with_cancel(credential, tokio_util::sync::CancellationToken::new())
            .await
    }

    pub async fn complete_step_up_with_cancel(
        &mut self,
        credential: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<StepUpCompletion> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        if credential.is_empty()
            || credential.len() > 65_536
            || !credential.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(ClientError::Configuration);
        }
        if self.step_up.is_none() {
            return Err(ClientError::Protocol);
        }
        let mut pending = self.pending()?;
        let challenge = self.step_up.take().ok_or(ClientError::Protocol)?;
        let remaining = (self.authenticated_until.min(self.server_key_expires_at)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
        let deadline = challenge
            .deadline
            .min(tokio::time::Instant::now() + self.deadline.min(remaining));
        let mut response = envelope(
            "identity.step-up-response",
            json!({"method":"bearer","credential":credential}),
        );
        response["in_reply_to"] = json!(challenge.id);
        for field in ["cid", "sid", "connid", "operation_id"] {
            let value = match field {
                "cid" => &challenge.scope.conversation_id,
                "sid" => &challenge.scope.session_id,
                "connid" => &challenge.scope.connection_id,
                _ => &challenge.scope.operation_id,
            };
            if let Some(value) = value {
                response[field] = json!(value);
            }
        }
        if let Some(revision) = challenge.scope.expected_revision {
            response["expected_revision"] = json!(revision);
        }
        let response_id = response["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        if tokio::time::Instant::now() >= deadline {
            return Err(ClientError::Timeout);
        }
        let mut owned_transition = self
            .owned_signaling
            .as_ref()
            .map(|signaling| {
                signaling.expect_step_up(
                    &response_id,
                    &self.assurance,
                    &challenge.required,
                    challenge.scope.clone(),
                    deadline,
                )
            })
            .transpose()?;
        let mut settled = false;
        let work = async {
            if tokio::time::Instant::now() >= deadline {
                return Err(ClientError::Timeout);
            }
            self.write(response, true).await?;
            let mut session: Option<payloads::AuthSession> = None;
            let mut authentication = None;
            let mut original_refused = false;
            let mut frames = Vec::new();
            let mut bytes = 0usize;
            loop {
                let frame = self.receive_wire_transition(Some(&response_id)).await?;
                if cancellation.is_cancelled() {
                    return Err(ClientError::Cancelled);
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ClientError::Timeout);
                }
                if frame.kind() == "auth.bye" {
                    self.queue_event(frame)?;
                    return Err(ClientError::Transport);
                }
                if frame.scope(Scope::Reply) == Some(response_id.as_str()) {
                    check_reply_scope(&frame, &challenge.scope, false)?;
                    match frame.payload() {
                        Payload::Error(error) if session.is_none() => {
                            settled = true;
                            return Err(remote_error(error.code));
                        }
                        Payload::AuthSession(next)
                            if session.is_none()
                                && authentication.is_none()
                                && !original_refused =>
                        {
                            if next.identity_id.as_str() != self.identity_id
                                || next.assurance != challenge.required
                                || frame.original()["payload"]["application_profile"] != "connect/2"
                                || next.expires_at <= chrono::Utc::now()
                                || next.session_token.is_empty()
                                || next.session_token == self.session_token
                                || next.session_token.len() > 4096
                                || !next
                                    .session_token
                                    .bytes()
                                    .all(|byte| byte.is_ascii_graphic())
                            {
                                return Err(ClientError::Authentication);
                            }
                            session = Some(next.clone());
                        }
                        Payload::IdentityAssuranceChanged(change) if authentication.is_none() => {
                            let next = session.as_ref().ok_or(ClientError::Protocol)?;
                            if change.previous != self.assurance
                                || change.current != next.assurance
                                || change.reason != "verified-step-up"
                            {
                                return Err(ClientError::Authentication);
                            }
                            let expires_at = next.expires_at.min(self.server_key_expires_at);
                            if let Some(permit) = &mut owned_transition {
                                permit.verified()?;
                            }
                            if let Some(signaling) = &self.owned_signaling {
                                signaling.refresh_until(expires_at)?;
                            }
                            self.session_token = next.session_token.clone();
                            self.assurance = next.assurance.clone();
                            self.authenticated_until = expires_at;
                            self.record_download.credential = credential.clone();
                            authentication = Some(CanonicalAuthentication {
                                identity_id: self.identity_id.clone(),
                                assurance: self.assurance.clone(),
                                expires_at,
                                application_profile: "connect/2".into(),
                            });
                        }
                        _ => return Err(ClientError::Protocol),
                    }
                    continue;
                }
                // Any uncorrelated assurance transition remains terminal.
                if frame.kind() == "identity.assurance-changed" {
                    self.queue_event(frame)?;
                    return Err(ClientError::Transport);
                }
                if frame.scope(Scope::Reply) != Some(challenge.request_id.as_str()) {
                    self.queue_event(frame)?;
                    continue;
                }
                if authentication.is_none() {
                    // The canonical peer drains validation/interruption
                    // failures in original-command, then response order. Keep
                    // the original durable/unknown problem available and wait
                    // for the response's separate definitive auth result.
                    if session.is_none()
                        && !original_refused
                        && matches!(frame.payload(), Payload::Error(_))
                    {
                        check_reply_scope(
                            &frame,
                            &challenge.scope,
                            challenge.assigned_conversation,
                        )?;
                        original_refused = true;
                        self.queue_event(frame)?;
                        continue;
                    }
                    return Err(ClientError::Protocol);
                }
                check_reply_scope(&frame, &challenge.scope, challenge.assigned_conversation)?;
                bytes = bytes
                    .checked_add(encoded_size(&frame)?)
                    .ok_or(ClientError::Protocol)?;
                if frames.len() >= MAX_BATCH_FRAMES || bytes > MAX_BATCH_BYTES {
                    return Err(ClientError::Protocol);
                }
                let end = matches!(frame.kind(), "ack" | "error");
                frames.push(frame);
                if end {
                    settled = true;
                    return Ok(StepUpCompletion {
                        authentication: authentication.ok_or(ClientError::Protocol)?,
                        replies: ReplyBatch {
                            request_id: challenge.request_id,
                            frames,
                        },
                    });
                }
                tokio::task::consume_budget().await;
            }
        };
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout_at(deadline, work) => result.unwrap_or(Err(ClientError::Timeout)),
        };
        pending.complete = settled;
        result
    }
}
