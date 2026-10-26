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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    continuation: Option<super::consultations::Continuation>,
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
    /// Native reliable data: full Connection scope, distinct from Conversation messages.
    pub data_send: OperationKind<DataAcceptance>,
    /// Read-only recovery of a server-created receive reconciliation.
    pub stream_reconcile: OperationKind<StreamSelectionReceipt>,
    pub stream_subscribe: OperationKind<StreamSelectionReceipt>,
    pub stream_unsubscribe: OperationKind<StreamSelectionReceipt>,
    pub session_handoff: OperationKind<HandoffResult>,
    pub connection_replace: OperationKind<MoveResult>,
    pub session_consult: OperationKind<ConsultationResult>,
    pub session_consult_return: OperationKind<Session>,
    pub connection_update: OperationKind<ConnectionUpdateReceipt>,
    pub connection_end: OperationKind<Connection>,
    pub connection_offer: OperationKind<Connection>,
    pub conversation_create: OperationKind<Conversation>,
    pub message_send: OperationKind<MessageAcceptance>,
    pub operation_cancel: OperationKind<DecisionReceipt>,
    pub operation_reconcile: OperationKind<DecisionReceipt>,
    pub conversation_participant_add: OperationKind<Participant>,
    pub conversation_participant_update: OperationKind<Participant>,
    pub conversation_participant_remove: OperationKind<Participant>,
    pub session_invite: OperationKind<Session>,
    pub session_accept: OperationKind<Session>,
    pub session_update: OperationKind<Session>,
    pub session_end: OperationKind<Session>,
    pub session_reject: OperationKind<SessionDecision>,
    pub session_cancel: OperationKind<SessionDecision>,
}
impl OperationKinds {
    pub(super) fn new() -> Self {
        Self {
            data_send: OperationKind::new("message.send", data::decode, connections::valid_scope),
            stream_reconcile: OperationKind::new(
                "stream.reconcile",
                selections::decode,
                connections::valid_scope,
            ),
            stream_subscribe: OperationKind::new(
                "stream.subscribe",
                selections::decode,
                connections::valid_scope,
            ),
            stream_unsubscribe: OperationKind::new(
                "stream.unsubscribe",
                selections::decode,
                connections::valid_scope,
            ),
            session_handoff: OperationKind::new(
                "session.handoff",
                movements::decode_handoff,
                connections::valid_scope,
            ),
            connection_replace: OperationKind::new(
                "connection.replace",
                movements::decode_move,
                connections::valid_scope,
            ),
            session_consult: OperationKind::new(
                "session.consult",
                consultations::decode_consult,
                connections::valid_scope,
            ),
            session_consult_return: OperationKind::new(
                "session.consult.return",
                consultations::decode_return,
                connections::valid_scope,
            ),
            connection_update: OperationKind::new(
                "connection.update",
                connection_updates::decode,
                connections::valid_scope,
            ),
            connection_end: OperationKind::new(
                "connection.end",
                connection_controls::decode_end,
                connections::valid_scope,
            ),
            connection_offer: OperationKind::new(
                "connection.offer",
                connections::decode,
                connections::valid_scope,
            ),
            conversation_participant_add: OperationKind::new(
                "conversation.participant.add",
                participants::decode,
                conversation_scope,
            ),
            conversation_participant_update: OperationKind::new(
                "conversation.participant.update",
                participants::decode,
                conversation_scope,
            ),
            conversation_participant_remove: OperationKind::new(
                "conversation.participant.remove",
                participants::decode,
                conversation_scope,
            ),
            session_invite: OperationKind::new("session.invite", sessions::decode, session_scope),
            session_accept: OperationKind::new("session.accept", sessions::decode, session_scope),
            session_update: OperationKind::new("session.update", sessions::decode, session_scope),
            session_end: OperationKind::new("session.end", sessions::decode, session_scope),
            session_reject: OperationKind::new(
                "session.reject",
                sessions::decode_decision,
                session_scope,
            ),
            session_cancel: OperationKind::new(
                "session.cancel",
                sessions::decode_decision,
                session_scope,
            ),
            operation_cancel: OperationKind::new(
                "operation.cancel",
                decisions::decode,
                decisions::valid_scope,
            ),
            operation_reconcile: OperationKind::new(
                "operation.reconcile",
                decisions::decode,
                decisions::valid_scope,
            ),
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
    pub(super) continuation: Option<super::consultations::Continuation>,
}
impl<T> Clone for OperationReference<T> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            kind: self.kind,
            continuation: self.continuation.clone(),
        }
    }
}
impl<T> OperationReference<T> {
    pub fn key(&self) -> &OperationKey {
        &self.key
    }
    pub fn to_json(&self) -> Value {
        let mut value = serde_json::json!({"schema":"connect-operation-reference/1","operation_id":self.key.operation_id,"command":self.key.command,"scope":self.key.scope});
        if let Some(context) = &self.continuation {
            value["continuation"] =
                serde_json::to_value(context).expect("validated typed continuation");
        }
        value
    }
}
#[derive(Clone)]
pub struct OperationSnapshot {
    pub key: OperationKey,
    pub state: OperationState,
    pub revision: u64,
    /// Stable host-issued record identity, distinct from the logical operation ID.
    pub record_id: Option<OperationRecordId>,
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
    observed_record: Arc<std::sync::Mutex<Option<OperationRecordId>>>,
}
impl<T> Clone for Operation<T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            reference: self.reference.clone(),
            observed_record: self.observed_record.clone(),
        }
    }
}
struct Observation {
    snapshot: OperationSnapshot,
    result: Option<Value>,
}
impl<T> Operation<T> {
    pub(super) fn client(&self) -> &Client {
        &self.client
    }
    pub(super) fn new(client: Client, reference: OperationReference<T>) -> Self {
        let record = match reference.continuation.as_ref() {
            Some(consultations::Continuation::Consultation { record_id, .. }) => record_id.clone(),
            _ => None,
        };
        Self {
            client,
            reference,
            observed_record: Arc::new(std::sync::Mutex::new(record)),
        }
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
            || !consultations::valid_continuation(
                kind.command,
                &saved.scope,
                saved.continuation.as_ref(),
            )
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
                continuation: saved.continuation,
            },
        ))
    }
    pub fn reference(&self) -> OperationReference<T> {
        let mut reference = self.reference.clone();
        if let Some(consultations::Continuation::Consultation { record_id, .. }) =
            &mut reference.continuation
        {
            if let Ok(observed) = self.observed_record.lock() {
                *record_id = observed.clone().or_else(|| record_id.clone());
            }
        }
        reference
    }
    pub fn reference_json(&self) -> Value {
        self.reference().to_json()
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
            let mut raw_payload = None;
            for frame in &batch.frames {
                match frame.payload() {
                    Payload::OperationUpdated(value)
                        if update.is_none()
                            && value.operation_id.as_str() == key.operation_id.as_str() =>
                    {
                        update = Some(value);
                        raw_payload = Some(&frame.original()["payload"]);
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
            if consultations::is_command(&key.command) {
                consultations::validate_observation(
                    raw_payload
                        .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(key), false))?,
                    key,
                    None,
                )?;
                if state == OperationState::Completed {
                    consultations::validate_completion(
                        update
                            .result
                            .as_ref()
                            .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(key), false))?,
                        key,
                        self.reference.continuation.as_ref(),
                    )?;
                }
            }
            let record_id = update
                .record_id
                .as_ref()
                .map(|id| id.as_str().parse::<OperationRecordId>())
                .transpose()
                .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?;
            if let Some(record) = &record_id {
                let mut known = self
                    .observed_record
                    .lock()
                    .map_err(|_| Error::request(ErrorKind::Protocol, None, false))?;
                if known.as_ref().is_some_and(|old| old != record) {
                    return Err(Error::request(ErrorKind::Protocol, None, false));
                }
                *known = Some(record.clone());
            }
            if selections::is_command(&key.command) {
                self.client.shared.remember_selector(
                    key,
                    raw_payload
                        .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(key), false))?,
                )?;
                match state {
                    OperationState::Completed => self.client.shared.record_selection(
                        key,
                        &selections::parse(
                            update.result.as_ref().ok_or_else(|| {
                                Error::request(ErrorKind::Protocol, Some(key), false)
                            })?,
                            key,
                        )?,
                    )?,
                    OperationState::Failed | OperationState::Cancelled => {
                        self.client.shared.fail_connection_update(key)?
                    }
                    _ => (),
                }
            }
            if key.command == "stream.reconcile" {
                self.client.shared.observe_reconciliation(
                    key,
                    raw_payload
                        .ok_or_else(|| Error::request(ErrorKind::Protocol, Some(key), false))?,
                )?;
            }
            if key.command == "connection.update" {
                match state {
                    OperationState::Completed => {
                        let observed = connection_updates::parse(
                            update.result.as_ref().ok_or_else(|| {
                                Error::request(ErrorKind::Protocol, Some(key), false)
                            })?,
                            key,
                        )?;
                        self.client
                            .shared
                            .record_connection_update(key, &observed)?;
                    }
                    OperationState::Failed | OperationState::Cancelled => {
                        self.client.shared.fail_connection_update(key)?
                    }
                    _ => (),
                }
            }
            Ok(Observation {
                snapshot: OperationSnapshot {
                    key: key.clone(),
                    state,
                    revision: update.revision,
                    record_id,
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

fn conversation_scope(scope: &OperationScope) -> bool {
    scope.conversation_id.is_some() && scope.session_id.is_none() && scope.connection_id.is_none()
}
fn session_scope(scope: &OperationScope) -> bool {
    scope.conversation_id.is_some() && scope.session_id.is_some() && scope.connection_id.is_none()
}
