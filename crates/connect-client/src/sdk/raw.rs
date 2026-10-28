//! Advanced finite canonical exchanges over the same owned signed peer.
//!
//! ```no_run
//! # async fn advanced(client: connect_client::sdk::Client) -> connect_client::sdk::Result<()> {
//! use connect_client::sdk::RequestOptions;
//! use std::time::Duration;
//! let raw = client.raw();
//! let request = raw.validate(serde_json::json!({
//!     "v": 2, "id": "local-validation", "ts": "2026-10-26T00:00:00Z",
//!     "type": "operation.get", "cid": "conv_saved",
//!     "payload": {"operation_id": "saved-mutation"}
//! }))?;
//! let replies = raw.request(request, RequestOptions::new(Duration::from_secs(5))?).await?;
//! for reply in replies { let _original_signed_json = reply.to_json(); }
//! # Ok(()) }
//! ```
use super::*;
use crate::canonical::{CommandScope, Envelope, Scope};

/// Schema-checked current envelope. Local validation does not authenticate a
/// signature, grant authorization, or establish local media custody. Replies
/// returned by `request` have passed the owned peer's signature/correlation checks.
#[derive(Clone)]
pub struct ValidatedEnvelope(Envelope);
impl ValidatedEnvelope {
    pub fn kind(&self) -> &str {
        self.0.kind()
    }
    /// Includes original optional fields and signature. Persist privately.
    pub fn to_json(&self) -> Value {
        self.0.original().clone()
    }
}
impl std::fmt::Debug for ValidatedEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedEnvelope")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}
/// Uses the Client's bounded request queue, current authority and joined cleanup.
#[derive(Clone)]
pub struct RawCanonical {
    client: Client,
}
impl Client {
    pub fn raw(&self) -> RawCanonical {
        RawCanonical {
            client: self.clone(),
        }
    }
}
impl RawCanonical {
    pub fn validate(&self, value: Value) -> Result<ValidatedEnvelope> {
        validate(value)
    }
    /// Submit original logical intent with a fresh transport ID, timestamp and
    /// signature. Save full intent and logical ID before I/O. No automatic retry.
    /// Canonical refusals become structured errors; successful batches include
    /// every correlated frame and the final ACK, retaining optional fields.
    ///
    /// Auth/step-up, native offer/answer, binary record transfer and live event
    /// leases require their dedicated lifecycle helpers. Finite journal replay
    /// requires explicit `live:false`; use Conversation::events for live work.
    pub async fn request(
        &self,
        envelope: ValidatedEnvelope,
        options: RequestOptions,
    ) -> Result<Vec<ValidatedEnvelope>> {
        let (scope, key) = intent(&envelope.0)?;
        let key = key.as_ref();
        options
            .validate()
            .map_err(|e| Error::request(e.kind, key, false))?;
        let batch = self
            .client
            .request(
                envelope.kind(),
                envelope.0.original()["payload"].clone(),
                scope,
                key,
                options,
            )
            .await?;
        Ok(batch.frames.into_iter().map(ValidatedEnvelope).collect())
    }
}
fn intent(envelope: &Envelope) -> Result<(CommandScope, Option<OperationKey>)> {
    let kind = envelope.kind();
    let value = envelope.original();
    let scope = OperationScope {
        conversation_id: envelope
            .scope(Scope::Conversation)
            .map(str::parse)
            .transpose()?,
        session_id: envelope.scope(Scope::Session).map(str::parse).transpose()?,
        connection_id: envelope
            .scope(Scope::Connection)
            .map(str::parse)
            .transpose()?,
    };
    let read = matches!(
        kind,
        "conversation.get"
            | "conversation.list"
            | "conversation.subscribe"
            | "message.history"
            | "operation.get"
    );
    let key = if read {
        None
    } else {
        envelope
            .scope(Scope::Operation)
            .map(|id| {
                Ok(OperationKey {
                    operation_id: id.parse()?,
                    command: kind.into(),
                    scope: scope.clone(),
                })
            })
            .transpose()?
    };
    let fail = |kind| Error::request(kind, key.as_ref(), false);
    // Never silently discard top-level caller intent. Only physical-attempt
    // metadata is replaced; all payload extensions remain intact.
    if value.as_object().is_none_or(|o| {
        o.keys().any(|name| {
            !matches!(
                name.as_str(),
                "v" | "id"
                    | "ts"
                    | "type"
                    | "payload"
                    | "signature"
                    | "cid"
                    | "sid"
                    | "connid"
                    | "operation_id"
                    | "expected_revision"
            )
        })
    }) {
        return Err(fail(ErrorKind::Unsupported));
    }
    if !matches!(
        kind,
        "conversation.create"
            | "conversation.close"
            | "conversation.get"
            | "conversation.list"
            | "conversation.participant.add"
            | "conversation.participant.update"
            | "conversation.participant.remove"
            | "conversation.subscribe"
            | "session.invite"
            | "session.accept"
            | "session.reject"
            | "session.cancel"
            | "session.end"
            | "session.update"
            | "session.consult"
            | "session.consult.return"
            | "session.handoff"
            | "connection.replace"
            | "connection.update"
            | "connection.end"
            | "stream.subscribe"
            | "stream.unsubscribe"
            | "message.send"
            | "message.read"
            | "message.history"
            | "capability.advertise"
            | "dtmf.send"
            | "operation.get"
            | "operation.cancel"
            | "operation.reconcile"
    ) || kind == "conversation.subscribe" && value["payload"]["live"] != false
    {
        return Err(fail(ErrorKind::Unsupported));
    }
    if !read && key.is_none() {
        return Err(Error::local(ErrorKind::Validation));
    }
    if scope.session_id.is_some() && scope.conversation_id.is_none()
        || scope.connection_id.is_some() && scope.session_id.is_none()
    {
        return Err(fail(ErrorKind::Validation));
    }
    let scope = CommandScope {
        conversation_id: envelope.scope(Scope::Conversation).map(str::to_owned),
        session_id: envelope.scope(Scope::Session).map(str::to_owned),
        connection_id: envelope.scope(Scope::Connection).map(str::to_owned),
        operation_id: envelope.scope(Scope::Operation).map(str::to_owned),
        expected_revision: envelope.expected_revision(),
    };
    Ok((scope, key))
}

// Local helper validation uses the identical frozen parser and finite-request
// classification. It opens no client, credentials, Store or network endpoint.
pub(super) fn validate(value: Value) -> Result<ValidatedEnvelope> {
    Envelope::from_value(value)
        .map(ValidatedEnvelope)
        .map_err(|_| Error::local(ErrorKind::Validation))
}
pub(super) fn request_key(value: &ValidatedEnvelope) -> Result<Option<OperationKey>> {
    intent(&value.0).map(|(_, key)| key)
}
