//! Bounded correlation shared by serial callers and the owned SDK driver.
use super::*;

pub(crate) struct Exchange {
    pub(crate) id: String,
    pub(crate) deadline: tokio::time::Instant,
    scope: CommandScope,
    assigned_conversation: bool,
    frames: Vec<Envelope>,
    bytes: usize,
}
impl Exchange {
    pub(crate) fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }
    pub(crate) fn continuation(&self, deadline: tokio::time::Instant) -> Self {
        Self {
            id: self.id.clone(),
            deadline,
            scope: self.scope.clone(),
            assigned_conversation: self.assigned_conversation,
            frames: Vec::new(),
            bytes: 0,
        }
    }
    pub(crate) fn finish(self) -> ReplyBatch {
        ReplyBatch {
            request_id: self.id,
            frames: self.frames,
        }
    }
}
impl CanonicalClient {
    pub(super) fn prepare_exchange(
        &self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
        invitation_reference: Option<&str>,
    ) -> Result<(Value, Exchange)> {
        if self.step_up.is_some() {
            return Err(ClientError::Protocol);
        }
        let mut value = command_value(kind, payload, scope.clone())?;
        if let Some(reference) = invitation_reference {
            if kind != "session.cancel" || Identifier::try_from(reference.to_owned()).is_err() {
                return Err(ClientError::Protocol);
            }
            value["in_reply_to"] = json!(reference);
        }
        let id = value["id"]
            .as_str()
            .ok_or(ClientError::Protocol)?
            .to_owned();
        let remaining = (self.authenticated_until.min(self.server_key_expires_at)
            - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
        Ok((
            value,
            Exchange {
                id,
                deadline: tokio::time::Instant::now() + self.deadline.min(remaining),
                scope,
                assigned_conversation: matches!(kind, "conversation.create" | "conversation.list"),
                frames: Vec::new(),
                bytes: 0,
            },
        ))
    }
    pub(crate) fn with_owned_signaling_cause(&self, error: ClientError) -> ClientError {
        self.owned_signaling
            .as_ref()
            .and_then(|reader| reader.terminal_error())
            .unwrap_or(error)
    }
    /// The SDK owns this peer and the one signature-verifying reader. A dropped
    /// wait cannot consume a partial frame or abandon another request's reply.
    pub(crate) async fn receive_owned_frame(&self) -> Result<Envelope> {
        self.owned_signaling
            .as_ref()
            .ok_or(ClientError::Protocol)?
            .receive()
            .await
    }
    pub(crate) async fn begin_exchange(
        &mut self,
        kind: &str,
        payload: Value,
        scope: CommandScope,
    ) -> Result<Exchange> {
        let (value, exchange) = self.prepare_exchange(kind, payload, scope, None)?;
        tokio::time::timeout_at(exchange.deadline, self.write(value, true))
            .await
            .map_err(|_| ClientError::Timeout)??;
        Ok(exchange)
    }
    pub(crate) fn accept_exchange_frame(
        &mut self,
        exchange: &mut Exchange,
        frame: Envelope,
    ) -> Result<bool> {
        if tokio::time::Instant::now() >= exchange.deadline {
            return Err(ClientError::Timeout);
        }
        if frame.scope(Scope::Reply) != Some(exchange.id.as_str()) {
            return Err(ClientError::Protocol);
        }
        check_reply_scope(&frame, &exchange.scope, exchange.assigned_conversation)?;
        if frame.kind() == "identity.step-up-request" {
            if self.step_up.is_some() {
                return Err(ClientError::Protocol);
            }
            self.step_up = Some(step_up::Challenge::new(
                &frame,
                &exchange.id,
                exchange.scope.clone(),
                exchange.deadline,
                exchange.assigned_conversation,
            )?);
        }
        exchange.bytes = exchange
            .bytes
            .checked_add(encoded_size(&frame)?)
            .ok_or(ClientError::Protocol)?;
        if exchange.frames.len() >= MAX_BATCH_FRAMES || exchange.bytes > MAX_BATCH_BYTES {
            return Err(ClientError::Protocol);
        }
        let end = matches!(frame.kind(), "ack" | "error" | "identity.step-up-request");
        exchange.frames.push(frame);
        Ok(end)
    }
    pub(crate) fn retain_unmatched_frame(&mut self, frame: Envelope) -> Result<()> {
        self.queue_event(frame)
    }
}
