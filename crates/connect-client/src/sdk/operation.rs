//! Original-ID status, bounded observation and command-checked saved references.
use super::*;
use crate::canonical::{CommandScope, Payload};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<ConversationId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<ConnectionId>,
}
impl OperationScope {
    pub(super) fn command(&self, operation_id: Option<&OperationId>) -> CommandScope {
        CommandScope {
            conversation_id: self.conversation_id.as_ref().map(|i| i.as_str().into()),
            session_id: self.session_id.as_ref().map(|i| i.as_str().into()),
            connection_id: self.connection_id.as_ref().map(|i| i.as_str().into()),
            operation_id: operation_id.map(|i| i.as_str().into()),
            expected_revision: None,
        }
    }
    fn valid(&self) -> bool {
        (self.session_id.is_none() || self.conversation_id.is_some())
            && (self.connection_id.is_none() || self.session_id.is_some())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationKey {
    pub operation_id: OperationId,
    pub command: String,
    pub scope: OperationScope,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedReference {
    schema: String,
    operation_id: OperationId,
    command: String,
    scope: OperationScope,
}
pub struct OperationKind<T> {
    pub(super) command: &'static str,
    pub(super) decode: fn(Value, &Client, &OperationKey) -> Result<T>,
    pub(super) valid_scope: fn(&OperationScope) -> bool,
    output: PhantomData<fn(T) -> T>,
}
impl<T> Copy for OperationKind<T> {}
impl<T> Clone for OperationKind<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> OperationKind<T> {
    pub fn command(&self) -> &str {
        self.command
    }
    pub(super) fn new(
        command: &'static str,
        decode: fn(Value, &Client, &OperationKey) -> Result<T>,
        valid_scope: fn(&OperationScope) -> bool,
    ) -> Self {
        Self {
            command,
            decode,
            valid_scope,
            output: PhantomData,
        }
    }
}
pub struct OperationKinds {
    pub conversation_create: OperationKind<Conversation>,
    pub message_send: OperationKind<MessageAcceptance>,
}
impl OperationKinds {
    pub(super) fn new() -> Self {
        Self {
            conversation_create: OperationKind::new(
                "conversation.create",
                conversations::decode_conversation,
                |s| s == &OperationScope::default(),
            ),
            message_send: OperationKind::new("message.send", conversations::decode_message, |s| {
                s.conversation_id.is_some() && s.session_id.is_none() && s.connection_id.is_none()
            }),
        }
    }
}
pub struct OperationReference<T> {
    pub(super) key: OperationKey,
    pub(super) kind: OperationKind<T>,
}
impl<T> Clone for OperationReference<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            kind: self.kind,
        }
    }
}
impl<T> OperationReference<T> {
    pub fn key(&self) -> &OperationKey {
        &self.key
    }
    pub fn to_json(&self) -> Value {
        serde_json::json!({"schema":"connect-operation-reference/1","operation_id":self.key.operation_id,"command":self.key.command,"scope":self.key.scope})
    }
}
#[derive(Clone)]
pub struct OperationSnapshot {
    pub key: OperationKey,
    pub state: OperationState,
    pub revision: u64,
    pub cancel_requested: Option<bool>,
    pub expires_at: Option<i64>,
    pub problem: Option<Value>,
}
impl std::fmt::Debug for OperationSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationSnapshot")
            .field("key", &self.key)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
pub struct Completion<T> {
    pub snapshot: OperationSnapshot,
    pub value: T,
}
pub struct Operation<T> {
    client: Client,
    reference: OperationReference<T>,
}
impl<T> Clone for Operation<T> {
    fn clone(&self) -> Self {
        Self::new(self.client.clone(), self.reference.clone())
    }
}
struct Observation {
    snapshot: OperationSnapshot,
    result: Option<Value>,
}
impl<T> Operation<T> {
    pub(super) fn new(client: Client, reference: OperationReference<T>) -> Self {
        Self { client, reference }
    }
    pub(super) fn restore(client: Client, value: Value, kind: OperationKind<T>) -> Result<Self> {
        if serde_json::to_vec(&value)
            .map_err(|_| Error::local(ErrorKind::Validation))?
            .len()
            > 4096
        {
            return Err(Error::local(ErrorKind::Validation));
        }
        let saved: SavedReference =
            serde_json::from_value(value).map_err(|_| Error::local(ErrorKind::Validation))?;
        if saved.schema != "connect-operation-reference/1"
            || saved.command != kind.command
            || !saved.scope.valid()
            || !(kind.valid_scope)(&saved.scope)
        {
            return Err(Error::local(ErrorKind::Validation));
        }
        Ok(Self::new(
            client,
            OperationReference {
                key: OperationKey {
                    operation_id: saved.operation_id,
                    command: saved.command,
                    scope: saved.scope,
                },
                kind,
            },
        ))
    }
    pub fn reference(&self) -> OperationReference<T> {
        self.reference.clone()
    }
    pub fn reference_json(&self) -> Value {
        self.reference.to_json()
    }
    async fn observe(&self, options: RequestOptions) -> Result<Observation> {
        let key = &self.reference.key;
        let result = async {
            let batch = self
                .client
                .request(
                    "operation.get",
                    serde_json::json!({"operation_id":key.operation_id}),
                    key.scope.command(None),
                    None,
                    options,
                )
                .await?;
            let mut update = None;
            for frame in &batch.frames {
                match frame.payload() {
                    Payload::OperationUpdated(value)
                        if update.is_none()
                            && value.operation_id.as_str() == key.operation_id.as_str() =>
                    {
                        update = Some(value)
                    }
                    Payload::Ack(_) => {}
                    _ => return Err(Error::request(ErrorKind::Protocol, None, false)),
                }
            }
            let update = update.ok_or_else(|| Error::request(ErrorKind::Protocol, None, false))?;
            if update.revision > 9_007_199_254_740_991 {
                return Err(Error::request(ErrorKind::Protocol, None, false));
            }
            let state = serde_json::from_value(
                serde_json::to_value(update.state)
                    .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?,
            )
            .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?;
            Ok(Observation {
                snapshot: OperationSnapshot {
                    key: key.clone(),
                    state,
                    revision: update.revision,
                    cancel_requested: None,
                    expires_at: None,
                    problem: update.problem.clone(),
                },
                result: update.result.clone(),
            })
        }
        .await;
        result.map_err(|mut error: Error| {
            error.operation = Some(key.clone());
            error
        })
    }
    pub async fn status(&self, options: RequestOptions) -> Result<OperationSnapshot> {
        Ok(self.observe(options).await?.snapshot)
    }
    pub async fn wait(&self, options: RequestOptions) -> Result<Completion<T>> {
        let observed = self.wait_observation(options, false).await?;
        let value = observed
            .result
            .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(&self.reference.key), false))?;
        let value = (self.reference.kind.decode)(value, &self.client, &self.reference.key)?;
        Ok(Completion {
            snapshot: observed.snapshot,
            value,
        })
    }
    pub async fn wait_until_ready(&self, options: RequestOptions) -> Result<OperationSnapshot> {
        Ok(self.wait_observation(options, true).await?.snapshot)
    }
    async fn wait_observation(&self, options: RequestOptions, ready: bool) -> Result<Observation> {
        options.validate()?;
        let deadline = tokio::time::Instant::now() + options.timeout;
        loop {
            if options.signal.is_cancelled() {
                return Err(self.wait_error(ErrorKind::Cancelled));
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(self.wait_error(ErrorKind::Deadline));
            }
            let observed = self
                .observe(RequestOptions {
                    timeout: remaining,
                    signal: options.signal.clone(),
                })
                .await?;
            match observed.snapshot.state {
                OperationState::Completed => return Ok(observed),
                OperationState::Ready | OperationState::Committing if ready => return Ok(observed),
                OperationState::Cancelled | OperationState::Failed => {
                    let mut error =
                        self.wait_error(if observed.snapshot.state == OperationState::Cancelled {
                            ErrorKind::Cancelled
                        } else {
                            ErrorKind::Application
                        });
                    error.certainty = Certainty::Recorded;
                    error.observation = Some(observed.snapshot);
                    return Err(error);
                }
                _ => {} // Unknown/interrupted remain observations, never a new command.
            }
            tokio::select! {biased;_=options.signal.cancelled()=>return Err(self.wait_error(ErrorKind::Cancelled)),_=tokio::time::sleep_until((tokio::time::Instant::now()+Duration::from_millis(125)).min(deadline))=>{}}
        }
    }
    fn wait_error(&self, kind: ErrorKind) -> Error {
        let mut error = Error::request(kind, None, false);
        error.operation = Some(self.reference.key.clone());
        error
    }
}

impl<T> std::fmt::Debug for Operation<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Operation")
            .field("key", &self.reference.key)
            .finish_non_exhaustive()
    }
}
