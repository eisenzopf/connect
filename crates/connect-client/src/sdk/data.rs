//! Typed reliable data on this peer's negotiated native Stream.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::json;
pub(super) const MAX_BYTES: usize = 64 * 1024;
#[derive(Clone, PartialEq)]
pub enum DataPayload {
    Json(Value),
    Binary(Vec<u8>),
}
impl std::fmt::Debug for DataPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(_) => f.write_str("DataPayload::Json(redacted)"),
            Self::Binary(b) => f
                .debug_struct("DataPayload::Binary")
                .field("bytes", &b.len())
                .finish(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ReceivedData {
    pub message_id: MessageId,
    pub operation_id: OperationId,
    pub from: ParticipantId,
    pub payload: DataPayload,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataAcceptanceBoundary {
    CoreQueueAdmission,
}
#[derive(Clone, Debug)]
pub struct DataAcceptance {
    pub message_id: MessageId,
    pub connection_id: ConnectionId,
    pub boundary: DataAcceptanceBoundary,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DataStats {
    pub received_frames: u64,
    pub accepted_messages: u64,
    pub malformed_frames: u64,
    pub unowned_frames: u64,
    pub overflow_drops: u64,
    pub binding_change_drops: u64,
    pub queued_messages: usize,
    pub queued_bytes: usize,
}
impl DataPayload {
    fn wire(self) -> Result<(String, &'static str, &'static str)> {
        let bad = || Error::local(ErrorKind::Validation);
        match self {
            Self::Json(v) => {
                let body = serde_json::to_string(&v).map_err(|_| bad())?;
                if body.len() > MAX_BYTES {
                    return Err(bad());
                }
                Ok((body, "application/json", "utf8"))
            }
            Self::Binary(bytes) => {
                if bytes.len() > MAX_BYTES {
                    return Err(bad());
                }
                Ok((STANDARD.encode(bytes), "application/octet-stream", "base64"))
            }
        }
    }
}
pub(super) fn body(payload: &Value, codec: &str) -> Result<(DataPayload, usize)> {
    let bad = || Error::local(ErrorKind::Validation);
    let bytes = payload["body"].as_str().ok_or_else(bad)?;
    let encoding = payload
        .get("body_encoding")
        .and_then(Value::as_str)
        .unwrap_or("utf8");
    match codec {
        "json"
            if encoding == "utf8"
                && payload["content_type"] == "application/json"
                && bytes.len() <= MAX_BYTES =>
        {
            let value = serde_json::from_str(bytes).map_err(|_| bad())?;
            Ok((DataPayload::Json(value), bytes.len()))
        }
        "binary"
            if encoding == "base64"
                && payload["content_type"] == "application/octet-stream"
                && bytes.len() <= MAX_BYTES.div_ceil(3) * 4 =>
        {
            let decoded = STANDARD.decode(bytes).map_err(|_| bad())?;
            if decoded.len() > MAX_BYTES || STANDARD.encode(&decoded) != bytes {
                return Err(bad());
            }
            let size = decoded.len();
            Ok((DataPayload::Binary(decoded), size))
        }
        _ => Err(bad()),
    }
}
pub(super) fn decode(value: Value, _client: &Client, key: &OperationKey) -> Result<DataAcceptance> {
    let bad = || Error::request(ErrorKind::Protocol, Some(key), true);
    if key.command != "message.send"
        || !connections::valid_scope(&key.scope)
        || value["connection_id"].as_str()
            != key.scope.connection_id.as_ref().map(ConnectionId::as_str)
        || value["core_queue_admission"] != true
    {
        return Err(bad());
    }
    Ok(DataAcceptance {
        message_id: serde_json::from_value(value["message_id"].clone()).map_err(|_| bad())?,
        connection_id: key.scope.connection_id.clone().ok_or_else(bad)?,
        boundary: DataAcceptanceBoundary::CoreQueueAdmission,
    })
}
impl Stream {
    /// Send on this owned data Stream. Persist both IDs and the original payload
    /// before calling. Completion means Core queue admission, never delivery.
    pub async fn send_data(
        &self,
        message_id: MessageId,
        to: NonEmpty<ParticipantId>,
        payload: DataPayload,
        options: MutationOptions,
    ) -> Result<Operation<DataAcceptance>> {
        let kind = self.client.operation_kinds().data_send;
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope {
                conversation_id: Some(self.key.conversation_id.clone()),
                session_id: Some(self.key.session_id.clone()),
                connection_id: Some(self.key.connection_id.clone()),
            },
        };
        let bad = |kind| Error::request(kind, Some(&key), false);
        let recipients = to.into_vec();
        if recipients.len() > 256
            || recipients
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != recipients.len()
        {
            return Err(bad(ErrorKind::Validation));
        }
        let (body, content_type, encoding) = payload.wire().map_err(|e| bad(e.kind))?;
        let payload = conversations::extend(json!({"msg_id":message_id,"to":recipients,"label":self.id(),"content_type":content_type,"body":body,"body_encoding":encoding,"reliability":{"mode":"reliable_ordered"}}), options.extensions).map_err(|e| bad(e.kind))?;
        let batch = self
            .client
            .request(
                kind.command,
                payload,
                key.scope.command(Some(&key.operation_id)),
                Some(&key),
                options.request,
            )
            .await?;
        let invalid = || Error::request(ErrorKind::Protocol, Some(&key), true);
        let Some(crate::canonical::Payload::OperationUpdated(update)) =
            batch.frames.first().map(|f| f.payload())
        else {
            return Err(invalid());
        };
        if batch.frames.len() != 2
            || update.operation_id.as_str() != key.operation_id.as_str()
            || update.revision > 9_007_199_254_740_991
        {
            return Err(invalid());
        }
        if serde_json::to_value(update.state).map_err(|_| invalid())? == json!("completed") {
            let accepted = decode(
                update.result.clone().ok_or_else(invalid)?,
                &self.client,
                &key,
            )?;
            if accepted.message_id != message_id {
                return Err(invalid());
            }
        }
        Ok(Operation::new(
            self.client.clone(),
            OperationReference {
                key,
                kind,
                continuation: None,
            },
        ))
    }
    /// Clones consume one ordered queue. A slow reader's overflow fails this
    /// Stream with ResyncRequired until a new physical binding is negotiated.
    /// Cancellation/deadline never consumes a queued message or sends a command.
    pub async fn receive_data(&self, options: RequestOptions) -> Result<ReceivedData> {
        self.client.shared.receive_data(&self.key, options).await
    }
}
impl Client {
    pub fn data_stats(&self) -> Result<DataStats> {
        self.shared.data_stats()
    }
}
