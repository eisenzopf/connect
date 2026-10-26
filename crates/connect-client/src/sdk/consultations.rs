//! Consultation intent and original-record continuation; receipts grant no media ownership.
use super::*;
use connect_protocol::transitions::{TransitionCommand, TransitionIntent};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeSet;
#[derive(Clone, Copy, Debug)]
pub enum Hold {
    Silence,
    ConfiguredMedia,
}
#[derive(Clone, Debug)]
pub struct ConsultInput {
    pub via: ConnectionId,
    pub retain: ConnectionId,
    pub with_participants: NonEmpty<ParticipantId>,
    pub hold: Hold,
    pub expected_topology_revision: TopologyRevision,
}
#[derive(Clone, Debug)]
pub struct TopologyChange {
    pub retained: Vec<Connection>,
    pub admitted: Vec<Connection>,
    pub retired: Vec<ConnectionId>,
    pub topology_revision: TopologyRevision,
    pub conversation_revision: Option<ConversationRevision>,
}
#[derive(Clone)]
pub struct ConsultationResult {
    pub session: Session,
    pub topology: TopologyChange,
}
#[derive(Clone, Debug)]
pub struct ConsultationReturnReceipt {
    pub consultation_record_id: OperationRecordId,
    pub topology: TopologyChange,
    pub remote_continuing: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum Continuation {
    Handoff {
        retained_connection_id: ConnectionId,
        source_participant: ParticipantId,
        target_participant: ParticipantId,
        candidate: Option<ConnectionId>,
        role: HandoffRole,
        outgoing: Outgoing,
    },
    Replacement {
        retained_connection_id: ConnectionId,
        target: MoveTarget,
    },

    Consultation {
        retained_connection_id: ConnectionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        record_id: Option<OperationRecordId>,
    },
    ConsultationReturn {
        consultation_record_id: OperationRecordId,
        retained_connection_id: ConnectionId,
    },
}
pub(super) fn is_command(command: &str) -> bool {
    matches!(
        command,
        "session.consult" | "session.consult.return" | "session.handoff" | "connection.replace"
    )
}
pub(super) fn valid_continuation(
    command: &str,
    scope: &OperationScope,
    context: Option<&Continuation>,
) -> bool {
    if matches!(command, "session.handoff" | "connection.replace") {
        return movements::valid_continuation(command, scope, context);
    }
    match (command, context) {
        (
            "session.consult",
            Some(Continuation::Consultation {
                retained_connection_id,
                ..
            }),
        )
        | (
            "session.consult.return",
            Some(Continuation::ConsultationReturn {
                retained_connection_id,
                ..
            }),
        ) => {
            connections::valid_scope(scope)
                && scope.connection_id.as_ref() != Some(retained_connection_id)
        }
        ("session.consult" | "session.consult.return", _) => false,
        (_, None) => true,
        _ => false,
    }
}
fn fail(key: &OperationKey, submitted: bool) -> Error {
    Error::request(ErrorKind::Protocol, Some(key), submitted)
}
pub(super) fn validate_observation(
    value: &Value,
    key: &OperationKey,
    expected: Option<u64>,
) -> Result<()> {
    let bad = || fail(key, true);
    if value["expected_revision"]
        .as_u64()
        .is_none_or(|v| v >= 9_007_199_254_740_991)
        || value["command_type"] != key.command
        || value["record_id"]
            .as_str()
            .and_then(|s| s.parse::<OperationRecordId>().ok())
            .is_none()
        || expected.is_some_and(|r| value["expected_revision"].as_u64() != Some(r))
    {
        return Err(bad());
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    retained_connection_ids: Vec<ConnectionId>,
    admitted_connection_ids: Vec<ConnectionId>,
    retired_connection_ids: Vec<ConnectionId>,
    session_revision: TopologyRevision,
    conversation_revision: Option<ConversationRevision>,
    consultation_record_id: Option<OperationRecordId>,
    remote_continuing: Option<bool>,
}
fn wire(value: &Value, key: &OperationKey) -> Result<Wire> {
    let bad = || fail(key, true);
    if serde_json::to_vec(value).map_err(|_| bad())?.len() > 32 * 1024 {
        return Err(bad());
    }
    let w: Wire = serde_json::from_value(value.clone()).map_err(|_| bad())?;
    let mut ids = BTreeSet::new();
    let count = w.retained_connection_ids.len()
        + w.admitted_connection_ids.len()
        + w.retired_connection_ids.len();
    if !connections::valid_scope(&key.scope)
        || w.session_revision.get() == 0
        || count > 100
        || w.retained_connection_ids.is_empty()
        || w.retained_connection_ids
            .iter()
            .chain(&w.admitted_connection_ids)
            .chain(&w.retired_connection_ids)
            .any(|id| !ids.insert(id))
        || w.conversation_revision.is_some()
    {
        return Err(bad());
    }
    match key.command.as_str() {
        "session.consult"
            if w.consultation_record_id.is_none()
                && w.remote_continuing.is_none()
                && !w.admitted_connection_ids.is_empty()
                && w.retired_connection_ids.is_empty() =>
        {
            ()
        }
        "session.consult.return"
            if w.consultation_record_id.is_some()
                && w.remote_continuing.is_some()
                && w.admitted_connection_ids.is_empty() =>
        {
            ()
        }
        _ => return Err(bad()),
    }
    Ok(w)
}
pub(super) fn validate_completion(
    value: &Value,
    key: &OperationKey,
    context: Option<&Continuation>,
) -> Result<()> {
    if matches!(
        key.command.as_str(),
        "session.handoff" | "connection.replace"
    ) {
        return movements::validate_completion(value, key, context);
    }
    let w = wire(value, key)?;
    let retained = match context {
        Some(Continuation::Consultation {
            retained_connection_id,
            ..
        }) if key.command == "session.consult" => retained_connection_id,
        Some(Continuation::ConsultationReturn {
            consultation_record_id,
            retained_connection_id,
        }) if key.command == "session.consult.return"
            && w.consultation_record_id.as_ref() == Some(consultation_record_id) =>
        {
            retained_connection_id
        }
        _ => return Err(fail(key, true)),
    };
    // Remote EOF may retire the originally retained leg. Never invent a continuing call.
    if !w.retained_connection_ids.contains(retained)
        && !(key.command == "session.consult.return"
            && w.remote_continuing == Some(false)
            && w.retired_connection_ids.contains(retained))
    {
        return Err(fail(key, true));
    }
    Ok(())
}
fn session(client: &Client, key: &OperationKey) -> Result<Session> {
    Ok(client
        .conversation_by_id(
            key.scope
                .conversation_id
                .clone()
                .ok_or_else(|| fail(key, true))?,
        )
        .session_by_id(
            key.scope
                .session_id
                .clone()
                .ok_or_else(|| fail(key, true))?,
        ))
}
fn topology(w: &Wire, session: &Session) -> TopologyChange {
    TopologyChange {
        retained: w
            .retained_connection_ids
            .iter()
            .cloned()
            .map(|id| session.connection_by_id(id))
            .collect(),
        admitted: w
            .admitted_connection_ids
            .iter()
            .cloned()
            .map(|id| session.connection_by_id(id))
            .collect(),
        retired: w.retired_connection_ids.clone(),
        topology_revision: w.session_revision,
        conversation_revision: w.conversation_revision,
    }
}
pub(super) fn decode_consult(
    value: Value,
    client: &Client,
    key: &OperationKey,
) -> Result<ConsultationResult> {
    let w = wire(&value, key)?;
    let session = session(client, key)?;
    Ok(ConsultationResult {
        topology: topology(&w, &session),
        session,
    })
}
pub(super) fn decode_return(value: Value, client: &Client, key: &OperationKey) -> Result<Session> {
    let w = wire(&value, key)?;
    let mut session = session(client, key)?;
    session.consultation_return = Some(Arc::new(ConsultationReturnReceipt {
        topology: topology(&w, &session),
        consultation_record_id: w.consultation_record_id.ok_or_else(|| fail(key, true))?,
        remote_continuing: w.remote_continuing.ok_or_else(|| fail(key, true))?,
    }));
    Ok(session)
}
impl Session {
    pub async fn consult(
        &self,
        input: ConsultInput,
        options: MutationOptions,
    ) -> Result<Operation<ConsultationResult>> {
        let context = Continuation::Consultation {
            retained_connection_id: input.retain.clone(),
            record_id: None,
        };
        let intent = TransitionIntent::Consult {
            retained_connid: input.retain.into(),
            private_participants: input
                .with_participants
                .into_vec()
                .into_iter()
                .map(String::from)
                .collect(),
            hold_treatment: match input.hold {
                Hold::Silence => connect_protocol::transitions::HoldTreatment::Silence,
                Hold::ConfiguredMedia => {
                    connect_protocol::transitions::HoldTreatment::ConfiguredMedia
                }
            },
        };
        self.submit_transition(
            self.client().operation_kinds().session_consult,
            input.via,
            input.expected_topology_revision,
            intent,
            context,
            options,
        )
        .await
    }
    pub(super) async fn submit_transition<T>(
        &self,
        kind: OperationKind<T>,
        via: ConnectionId,
        expected: TopologyRevision,
        intent: TransitionIntent,
        mut context: Continuation,
        options: MutationOptions,
    ) -> Result<Operation<T>> {
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: kind.command.into(),
            scope: OperationScope {
                conversation_id: Some(self.conversation_id().clone()),
                session_id: Some(self.id().clone()),
                connection_id: Some(via),
            },
        };
        let invalid = || Error::request(ErrorKind::Validation, Some(&key), false);
        options.request.validate().map_err(|_| invalid())?;
        let extensions = options.extensions.unwrap_or_default();
        let mut command = TransitionCommand {
            operation_id: key.operation_id.as_str().into(),
            conversation_id: self.conversation_id().as_str().into(),
            session_id: self.id().as_str().into(),
            connection_id: key.scope.connection_id.as_ref().map(|s| s.as_str().into()),
            expected_revision: expected.get(),
            intent,
            payload_extensions: extensions,
        };
        command.normalize().map_err(|_| invalid())?;
        let mut payload = serde_json::to_value(&command.intent).map_err(|_| invalid())?;
        payload
            .as_object_mut()
            .ok_or_else(invalid)?
            .retain(|key, value| key != "type" && !value.is_null());
        for (name, value) in command.payload_extensions {
            payload[name] = value;
        }
        let mut scope = key.scope.command(Some(&key.operation_id));
        scope.expected_revision = Some(expected.get());
        let batch = self
            .client()
            .request(kind.command, payload, scope, Some(&key), options.request)
            .await?;
        if batch.frames.len() != 2 {
            return Err(fail(&key, true));
        }
        let frame = &batch.frames[0];
        let crate::canonical::Payload::OperationUpdated(update) = frame.payload() else {
            return Err(fail(&key, true));
        };
        if update.operation_id.as_str() != key.operation_id.as_str()
            || update.revision > 9_007_199_254_740_991
        {
            return Err(fail(&key, true));
        }
        validate_observation(&frame.original()["payload"], &key, Some(expected.get()))?;
        if serde_json::to_value(update.state).map_err(|_| fail(&key, true))? == json!("completed") {
            validate_completion(
                update.result.as_ref().ok_or_else(|| fail(&key, true))?,
                &key,
                Some(&context),
            )?;
        }
        if let Continuation::Consultation { record_id, .. } = &mut context {
            *record_id = Some(
                frame.original()["payload"]["record_id"]
                    .as_str()
                    .ok_or_else(|| fail(&key, true))?
                    .parse()
                    .map_err(|_| fail(&key, true))?,
            );
        }
        Ok(Operation::new(
            self.client().clone(),
            OperationReference {
                key,
                kind,
                continuation: Some(context),
            },
        ))
    }
}
impl Operation<ConsultationResult> {
    /// Restore the original retained engagement through a new, explicitly named operation.
    /// Use the original host-issued record observation; only an as-yet unknown
    /// record requires a current-authorized read. The new mutation always checks
    /// current return authority on the host. Record identity is never guessed.
    pub async fn return_to_retained(
        &self,
        expected: TopologyRevision,
        mut options: MutationOptions,
    ) -> Result<Operation<Session>> {
        let reference = self.reference();
        let key = OperationKey {
            operation_id: options.operation_id.clone(),
            command: "session.consult.return".into(),
            scope: reference.key.scope.clone(),
        };
        let invalid = || Error::request(ErrorKind::Validation, Some(&key), false);
        if options.operation_id == reference.key.operation_id {
            return Err(invalid());
        }
        options.request.validate().map_err(|_| invalid())?;
        let Some(Continuation::Consultation {
            retained_connection_id,
            record_id,
        }) = reference.continuation
        else {
            return Err(invalid());
        };
        let until = tokio::time::Instant::now() + options.request.timeout;
        let record = if let Some(record) = record_id {
            record
        } else {
            let observed = self
                .status(options.request.clone())
                .await
                .map_err(|mut e| {
                    e.operation = Some(key.clone());
                    e.certainty = Certainty::NotSubmitted;
                    e
                })?;
            observed
                .record_id
                .ok_or_else(|| Error::request(ErrorKind::ReceiptUnavailable, Some(&key), false))?
        };
        let remaining = until.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::request(ErrorKind::Deadline, Some(&key), false));
        }
        options.request.timeout = remaining;
        let session = session(self.client(), &key)?;
        let intent = TransitionIntent::ConsultReturn {
            consultation_record_id: record.as_str().into(),
            retained_connid: retained_connection_id.as_str().into(),
        };
        session
            .submit_transition(
                self.client().operation_kinds().session_consult_return,
                key.scope.connection_id.clone().ok_or_else(invalid)?,
                expected,
                intent,
                Continuation::ConsultationReturn {
                    consultation_record_id: record,
                    retained_connection_id,
                },
                options,
            )
            .await
    }
}
