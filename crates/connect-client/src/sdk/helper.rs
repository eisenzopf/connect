//! Typed managed-helper control. All behavior delegates to the public SDK.
//!
//! This control slice covers Conversation/Participant and message operations and their
//! durable decisions. The wire adapter owns framing; this module owns no Store,
//! alternate journal, credential reload, or automatic mutation replay.
use super::*;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeMap;
pub mod administrative;
mod connection;
mod movement;
pub use movement::{HandoffRequest, MoveRequest};
mod session;
pub use connection::{
    ConnectionAction, Offer as ConnectionOffer, Selector as ReceiveSelector,
    Update as ConnectionChange,
};
pub use session::{Change as SessionChange, InviteMedium, SessionAction};

pub const PROFILE: &str = "connect-sdk-helper/2";
pub const VALIDATION_PROFILE: &str = "connect-sdk-validation/1";
/// Local frozen validation/intent description; no payload echo, client or grant.
pub fn validation_descriptor_bytes(bytes: &[u8]) -> Value {
    match crate::canonical::Envelope::parse(bytes) {
        Err(_) => json!({"profile":VALIDATION_PROFILE,"valid":false,"request":null}),
        Ok(envelope) => validation_descriptor(envelope.original().clone()),
    }
}
pub fn validation_descriptor(value: Value) -> Value {
    match super::raw::validate(value) {
        Err(_) => json!({"profile":VALIDATION_PROFILE,"valid":false,"request":null}),
        Ok(envelope) => {
            let request = request_descriptor(&envelope);
            json!({"profile":VALIDATION_PROFILE,"valid":true,"request":request})
        }
    }
}

fn request_descriptor(value: &ValidatedEnvelope) -> Value {
    match super::raw::request_key(value) {
        Ok(key) => json!({"key":key.as_ref().map(key_json)}),
        Err(error) => json!({"error":error_json(&error)}),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub timeout_ms: u64,
    pub action: Action,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub operation_id: OperationId,
    #[serde(default)]
    pub extensions: Option<BTreeMap<String, Value>>,
}
impl Mutation {
    fn options(self, request: RequestOptions) -> MutationOptions {
        MutationOptions {
            operation_id: self.operation_id,
            request,
            extensions: self.extensions,
        }
    }
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    RawRequest {
        envelope: Value,
    },
    SessionHandoff {
        conversation_id: ConversationId,
        session_id: SessionId,
        input: HandoffRequest,
    },
    ConnectionMove {
        conversation_id: ConversationId,
        session_id: SessionId,
        connection_id: ConnectionId,
        input: MoveRequest,
    },
    CapabilitiesRefresh,
    ConsultationReturn {
        reference: Value,
        expected_revision: TopologyRevision,
        mutation: Mutation,
    },
    ConversationCreate {
        participants: Vec<IdentityId>,
        policy: ConversationPolicy,
        mutation: Mutation,
    },
    ConversationSend {
        conversation_id: ConversationId,
        message_id: MessageId,
        to: Vec<ParticipantId>,
        text: String,
        mutation: Mutation,
    },
    MessageReceipts {
        conversation_id: ConversationId,
        message_id: MessageId,
    },
    ConversationRefresh {
        conversation_id: ConversationId,
    },
    ParticipantAdd {
        conversation_id: ConversationId,
        identity_id: IdentityId,
        kind: IdentityKind,
        role: String,
        expected_revision: ConversationRevision,
        mutation: Mutation,
    },
    ParticipantUpdate {
        conversation_id: ConversationId,
        participant_id: ParticipantId,
        role: String,
        expected_revision: ConversationRevision,
        mutation: Mutation,
    },
    ParticipantRemove {
        conversation_id: ConversationId,
        participant_id: ParticipantId,
        expected_revision: ConversationRevision,
        mutation: Mutation,
    },
    ParticipantRefresh {
        conversation_id: ConversationId,
        participant_id: ParticipantId,
    },
    Session {
        conversation_id: ConversationId,
        session_id: SessionId,
        action: SessionAction,
    },
    Connection {
        conversation_id: ConversationId,
        session_id: SessionId,
        connection_id: ConnectionId,
        action: ConnectionAction,
    },
    Operation {
        reference: Value,
        kind: Kind,
        action: OperationAction,
    },
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ConversationCreate,
    MessageSend,
    OperationCancel,
    OperationReconcile,
    ConversationParticipantAdd,
    ConversationParticipantUpdate,
    ConversationParticipantRemove,
    SessionInvite,
    SessionAccept,
    SessionUpdate,
    SessionEnd,
    SessionReject,
    SessionCancel,
    SessionConsult,
    SessionConsultReturn,
    SessionHandoff,
    ConnectionReplace,
    ConnectionOffer,
    ConnectionUpdate,
    ConnectionEnd,
    StreamSubscribe,
    StreamUnsubscribe,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationAction {
    Status,
    Wait,
    WaitUntilReady,
    Cancel {
        mutation: Mutation,
    },
    Reconcile {
        outcome: ReconcileOutcome,
        evidence_id: String,
        mutation: Mutation,
    },
}

/// A request never restores a grant. Every observation and decision goes through
/// the original SDK's command/scope/result checks and current host authority.
pub async fn execute(client: &Client, request: Request) -> Result<Value> {
    let options = RequestOptions::new(Duration::from_millis(request.timeout_ms))?;
    match request.action {
        Action::RawRequest { envelope } => {
            let raw = client.raw();
            let checked = raw.validate(envelope)?;
            let key = super::raw::request_key(&checked)?;
            let frames = raw.request(checked, options).await?;
            let request_id = frames
                .first()
                .and_then(|v| v.to_json()["in_reply_to"].as_str().map(str::to_owned))
                .ok_or_else(|| Error::request(ErrorKind::Protocol, key.as_ref(), true))?;
            Ok(
                json!({"requestId":request_id,"envelopes":frames.into_iter().map(|v|json!({"value":v.to_json(),"request":request_descriptor(&v)})).collect::<Vec<_>>() }),
            )
        }

        Action::SessionHandoff {
            conversation_id,
            session_id,
            input,
        } => movement::handoff(client, conversation_id, session_id, input, options).await,
        Action::ConnectionMove {
            conversation_id,
            session_id,
            connection_id,
            input,
        } => {
            movement::replace(
                client,
                conversation_id,
                session_id,
                connection_id,
                input,
                options,
            )
            .await
        }
        Action::ConsultationReturn {
            reference,
            expected_revision,
            mutation,
        } => {
            let operation = client.restore(reference, client.operation_kinds().session_consult)?;
            Ok(operation
                .return_to_retained(expected_revision, mutation.options(options))
                .await?
                .reference_json())
        }
        Action::CapabilitiesRefresh => {
            serde_json::to_value(client.capabilities().refresh(options).await?)
                .map_err(|_| Error::local(ErrorKind::Protocol))
        }
        Action::ConversationCreate {
            participants,
            policy,
            mutation,
        } => Ok(client
            .create_conversation(participants, policy, mutation.options(options))
            .await?
            .reference_json()),
        Action::ConversationSend {
            conversation_id,
            message_id,
            to,
            text,
            mutation,
        } => {
            let mut to = to.into_iter();
            let first = to
                .next()
                .ok_or_else(|| Error::local(ErrorKind::Validation))?;
            Ok(client
                .conversation_by_id(conversation_id)
                .send(
                    message_id,
                    NonEmpty {
                        first,
                        rest: to.collect(),
                    },
                    text,
                    mutation.options(options),
                )
                .await?
                .reference_json())
        }
        Action::MessageReceipts {
            conversation_id,
            message_id,
        } => {
            let observations = client
                .conversation_by_id(conversation_id.clone())
                .receipts(message_id.clone(), options)
                .await?;
            Ok(
                json!({"conversationId":conversation_id,"messageId":message_id,
                "observations": observations.into_iter().map(|v| json!({
                    "participantId":v.participant_id,"membershipEpoch":v.membership_epoch,
                    "channel":v.channel,"deliveryId":v.delivery_id,"state":v.state,
                    "provenance":v.provenance,"observedAt":v.observed_at
                })).collect::<Vec<_>>() }),
            )
        }
        Action::ConversationRefresh { conversation_id } => {
            let room = client
                .conversation_by_id(conversation_id)
                .refresh(options)
                .await?;
            Ok(
                json!({"id":room.id,"revision":room.revision,"closed":room.closed,
                "participants":room.participants.into_iter().map(participant_snapshot_json).collect::<Vec<_>>()}),
            )
        }
        Action::ParticipantAdd {
            conversation_id,
            identity_id,
            kind,
            role,
            expected_revision,
            mutation,
        } => Ok(client
            .conversation_by_id(conversation_id)
            .add_participant(
                identity_id,
                kind,
                role,
                expected_revision,
                mutation.options(options),
            )
            .await?
            .reference_json()),
        Action::ParticipantUpdate {
            conversation_id,
            participant_id,
            role,
            expected_revision,
            mutation,
        } => Ok(client
            .conversation_by_id(conversation_id)
            .participant_by_id(participant_id)
            .update(role, expected_revision, mutation.options(options))
            .await?
            .reference_json()),
        Action::ParticipantRemove {
            conversation_id,
            participant_id,
            expected_revision,
            mutation,
        } => Ok(client
            .conversation_by_id(conversation_id)
            .participant_by_id(participant_id)
            .remove(expected_revision, mutation.options(options))
            .await?
            .reference_json()),
        Action::ParticipantRefresh {
            conversation_id,
            participant_id,
        } => Ok(participant_snapshot_json(
            client
                .conversation_by_id(conversation_id)
                .participant_by_id(participant_id)
                .refresh(options)
                .await?,
        )),
        Action::Session {
            conversation_id,
            session_id,
            action,
        } => session::execute(client, conversation_id, session_id, action, options).await,
        Action::Connection {
            conversation_id,
            session_id,
            connection_id,
            action,
        } => {
            connection::execute(
                client,
                conversation_id,
                session_id,
                connection_id,
                action,
                options,
            )
            .await
        }
        Action::Operation {
            reference,
            kind,
            action,
        } => {
            let kinds = client.operation_kinds();
            match kind {
                Kind::ConversationCreate => run(client.restore(reference, kinds.conversation_create)?, action, options,
                    |room| json!({"id":room.id()})).await,
                Kind::MessageSend => run(client.restore(reference, kinds.message_send)?, action, options,
                    |message| json!({"messageId":message.message_id,"operation":key_json(&message.operation),"boundary":message.boundary})).await,
                Kind::OperationCancel => run(client.restore(reference, kinds.operation_cancel)?, action, options, decision_json).await,
                Kind::OperationReconcile => run(client.restore(reference, kinds.operation_reconcile)?, action, options, decision_json).await,
                Kind::ConversationParticipantAdd => run(client.restore(reference, kinds.conversation_participant_add)?, action, options, participant_json).await,
                Kind::ConversationParticipantUpdate => run(client.restore(reference, kinds.conversation_participant_update)?, action, options, participant_json).await,
                Kind::ConversationParticipantRemove => run(client.restore(reference, kinds.conversation_participant_remove)?, action, options, participant_json).await,
                Kind::SessionInvite => run(client.restore(reference,kinds.session_invite)?,action,options,session::handle_json).await,
                Kind::SessionAccept => run(client.restore(reference,kinds.session_accept)?,action,options,session::handle_json).await,
                Kind::SessionUpdate => run(client.restore(reference,kinds.session_update)?,action,options,session::handle_json).await,
                Kind::SessionEnd => run(client.restore(reference,kinds.session_end)?,action,options,session::handle_json).await,
                Kind::SessionReject => run(client.restore(reference,kinds.session_reject)?,action,options,session::decision_json).await,
                Kind::SessionHandoff => run(client.restore(reference,kinds.session_handoff)?,action,options,movement::handoff_json).await,
                Kind::ConnectionReplace => run(client.restore(reference,kinds.connection_replace)?,action,options,movement::move_json).await,
                Kind::SessionConsult => run(client.restore(reference,kinds.session_consult)?,action,options,session::consultation_json).await,
                Kind::SessionConsultReturn => run(client.restore(reference,kinds.session_consult_return)?,action,options,session::handle_json).await,
                Kind::SessionCancel => run(client.restore(reference,kinds.session_cancel)?,action,options,session::decision_json).await,
                Kind::ConnectionOffer => run(client.restore(reference,kinds.connection_offer)?,action,options,connection::handle_json).await,
                Kind::ConnectionUpdate => run(client.restore(reference,kinds.connection_update)?,action,options,connection::update_json).await,
                Kind::ConnectionEnd => run(client.restore(reference,kinds.connection_end)?,action,options,connection::handle_json).await,
                Kind::StreamSubscribe => run(client.restore(reference,kinds.stream_subscribe)?,action,options,connection::selection_json).await,
                Kind::StreamUnsubscribe => run(client.restore(reference,kinds.stream_unsubscribe)?,action,options,connection::selection_json).await,
            }
        }
    }
}
async fn run<T>(
    operation: Operation<T>,
    action: OperationAction,
    options: RequestOptions,
    encode: fn(T) -> Value,
) -> Result<Value> {
    match action {
        OperationAction::Status => Ok(snapshot_json(&operation.status(options).await?)),
        OperationAction::WaitUntilReady => {
            Ok(snapshot_json(&operation.wait_until_ready(options).await?))
        }
        OperationAction::Wait => {
            let completed = operation.wait(options).await?;
            Ok(
                json!({"snapshot":snapshot_json(&completed.snapshot),"value":encode(completed.value)}),
            )
        }
        OperationAction::Cancel { mutation } => Ok(operation
            .cancel(mutation.options(options))
            .await?
            .reference_json()),
        OperationAction::Reconcile {
            outcome,
            evidence_id,
            mutation,
        } => Ok(operation
            .reconcile(outcome, evidence_id, mutation.options(options))
            .await?
            .reference_json()),
    }
}
fn decision_json(value: DecisionReceipt) -> Value {
    json!({"decision":key_json(&value.decision),"target":value.target,
        "observedState":value.observed_state,"disposition":value.disposition,"evidenceId":value.evidence_id})
}
pub fn key_json(key: &OperationKey) -> Value {
    json!({"operationId":key.operation_id,"command":key.command,"scope":{
        "conversationId":key.scope.conversation_id,"sessionId":key.scope.session_id,
        "connectionId":key.scope.connection_id}})
}
pub fn snapshot_json(snapshot: &OperationSnapshot) -> Value {
    json!({"key":key_json(&snapshot.key),"state":snapshot.state,"revision":snapshot.revision,
        "recordId":snapshot.record_id,"cancelRequested":snapshot.cancel_requested,
        "expiresAt":snapshot.expires_at,"problem":snapshot.problem})
}
/// Preserve safe SDK distinctions and recorded observations, never Debug/raw
/// transport bodies, command text, stderr, or credentials.
pub fn error_json(error: &Error) -> Value {
    let kind = match error.kind {
        ErrorKind::Validation => "validation",
        ErrorKind::Configuration => "configuration",
        ErrorKind::Trust => "trust",
        ErrorKind::Authentication => "authentication",
        ErrorKind::Protocol => "protocol",
        ErrorKind::Deadline => "deadline",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::TransportUncertain => "transport-uncertain",
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::Conflict => "conflict",
        ErrorKind::Application => "application",
        ErrorKind::ReceiptUnavailable => "receipt-unavailable",
        ErrorKind::ResyncRequired => "resync-required",
        ErrorKind::CleanupIncomplete => "cleanup-incomplete",
    };
    let certainty = match error.certainty {
        Certainty::NotSubmitted => "not-submitted",
        Certainty::Refused => "refused",
        Certainty::Uncertain => "uncertain",
        Certainty::Recorded => "recorded",
        Certainty::ReadOnly => "read-only",
    };
    json!({"kind":kind,"certainty":certainty,"operation":error.operation.as_ref().map(key_json),
        "problem":error.problem.as_ref().map(|p| json!({"code":p.code,"category":p.category,"reason":p.reason})),
        "observation":error.observation.as_ref().map(snapshot_json)})
}

fn participant_json(value: Participant) -> Value {
    json!({"id":value.id(),"conversationId":value.conversation_id()})
}
fn participant_snapshot_json(p: ParticipantSnapshot) -> Value {
    json!({"id":p.id,"conversationId":p.conversation_id,"identityId":p.identity_id,
        "kind":p.kind,"role":p.role,"membershipEpoch":p.membership_epoch,
        "active":p.active,"removalReason":p.removal_reason})
}
