//! Signed transition commands enter the durable ledger, never stock core state.
use crate::{Service, StoreInvocationAuthority};
use connect_protocol::transitions::OperationState;
use connect_protocol::{transitions::*, Error, Result};
use rvoip_uctp::v2::{payloads::*, Envelope, Payload, Scope};

pub(crate) fn command(e: &Envelope) -> Result<Option<TransitionCommand>> {
    if !matches!(
        e.payload(),
        Payload::SessionConsult(_)
            | Payload::SessionConsultReturn(_)
            | Payload::SessionHandoff(_)
            | Payload::ConnectionReplace(_)
    ) {
        return Ok(None);
    }
    if e.scope(Scope::Reply).is_some() {
        return Err(Error::forbidden());
    }
    let id = |scope| {
        e.scope(scope)
            .map(str::to_owned)
            .ok_or_else(|| Error::new("invalid-request", "Missing transition resource assertion"))
    };
    let intent = match e.payload() {
        Payload::SessionConsultReturn(v) => TransitionIntent::ConsultReturn {
            consultation_record_id: v.consultation_record_id.as_str().into(),
            retained_connid: v.retained_connid.as_str().into(),
        },
        Payload::SessionConsult(v) => TransitionIntent::Consult {
            retained_connid: v.retained_connid.as_str().into(),
            private_participants: v
                .private_participants
                .iter()
                .map(|p| p.as_str().into())
                .collect(),
            hold_treatment: match v
                .hold_treatment
                .unwrap_or(SessionConsultHoldTreatment::Silence)
            {
                SessionConsultHoldTreatment::Silence => HoldTreatment::Silence,
                SessionConsultHoldTreatment::ConfiguredMedia => HoldTreatment::ConfiguredMedia,
            },
        },
        Payload::SessionHandoff(v) => TransitionIntent::Handoff {
            source_participant: v.source_participant.as_str().into(),
            target_participant: v.target_participant.as_str().into(),
            retained_connid: v.retained_connid.as_str().into(),
            candidate_connid: v.candidate_connid.as_ref().map(|p| p.as_str().into()),
            outgoing: match v.outgoing {
                SessionHandoffOutgoing::Leave => OutgoingDisposition::Leave,
                SessionHandoffOutgoing::Observe => OutgoingDisposition::Observe,
            },
            role_scope: match v
                .role_scope
                .unwrap_or(SessionHandoffRoleScope::Conversation)
            {
                SessionHandoffRoleScope::Conversation => RoleScope::Conversation,
                SessionHandoffRoleScope::Session => RoleScope::Session,
            },
            expected_conversation_revision: v.expected_conversation_revision,
        },
        Payload::ConnectionReplace(v) => TransitionIntent::Replace {
            retained_connid: v.retained_connid.as_str().into(),
            endpoint_id: v.endpoint_id.as_ref().map(|p| p.as_str().into()),
            candidate_connid: v.candidate_connid.as_ref().map(|p| p.as_str().into()),
        },
        _ => return Ok(None),
    };
    let mut command = TransitionCommand {
        payload_extensions: e.original()["payload"]
            .as_object()
            .ok_or_else(|| Error::new("invalid-request", "Invalid transition payload"))?
            .iter()
            .filter(|(key, _)| !intent.payload_fields().contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        operation_id: id(Scope::Operation)?,
        conversation_id: id(Scope::Conversation)?,
        session_id: id(Scope::Session)?,
        connection_id: e.scope(Scope::Connection).map(str::to_owned),
        expected_revision: e
            .expected_revision()
            .ok_or_else(|| Error::new("invalid-request", "Missing transition revision"))?,
        intent,
    };
    command.normalize()?;
    Ok(Some(command))
}
impl Service {
    pub(crate) async fn canonical_transition_held(
        &self,
        credential: String,
        command: TransitionCommand,
        authority: Option<StoreInvocationAuthority>,
    ) -> Result<OperationRecord> {
        let store = self.store.clone();
        let initialize_accepted = self.canonical_runtime.get().is_some_and(|r| {
            r.consultation_initializer.get().is_some()
                || r.initial_telephone_host
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .is_some()
        });
        let queued_command = matches!(
            command.intent,
            TransitionIntent::Consult { .. }
                | TransitionIntent::ConsultReturn { .. }
                | TransitionIntent::Handoff { .. }
                | TransitionIntent::Replace { .. }
        )
        .then(|| command.clone());
        let (result, queue_credential) = tokio::task::spawn_blocking(move || {
            let result = match authority {
                Some(authority) => store.admit_transition_with_peer(
                    &credential,
                    command,
                    authority.peer_guard().ok_or_else(Error::forbidden)?,
                ),
                None => store.admit_transition_command(&credential, command),
            };
            let queue_credential = result
                .as_ref()
                .ok()
                .filter(|r| {
                    matches!(
                        r.command_type.as_str(),
                        "session.consult"
                            | "session.consult.return"
                            | "session.handoff"
                            | "connection.replace"
                    )
                })
                .filter(|r| {
                    initialize_accepted
                        || (r.command_type == "session.consult.return"
                            || r.command_type == "session.handoff"
                            || r.command_type == "connection.replace"
                            || matches!(
                                r.state,
                                OperationState::Preparing | OperationState::Unknown
                            ))
                        || store
                            .native_consultation_has_ready_leg(&credential, &r.record_id)
                            .unwrap_or(false)
                })
                .and_then(|_| store.revalidate(&credential).ok())
                .map(|actor| actor.credential_id);
            Ok::<_, Error>((result, queue_credential))
        })
        .await
        .map_err(|_| Error::storage())??;
        if let (Some(runtime), Some(command), Some(key), Ok(record)) = (
            self.canonical_runtime.get(),
            queued_command,
            queue_credential,
            &result,
        ) {
            if matches!(command.intent, TransitionIntent::Replace { .. }) {
                if matches!(
                    record.state,
                    OperationState::Accepted | OperationState::Preparing | OperationState::Unknown
                ) {
                    runtime.queue_replacement(key, command, record.record_id.clone());
                }
            } else if matches!(command.intent, TransitionIntent::Handoff { .. }) {
                if matches!(
                    record.state,
                    OperationState::Accepted | OperationState::Preparing | OperationState::Unknown
                ) {
                    runtime.queue_handoff(key, command, record.record_id.clone());
                }
            } else if matches!(command.intent, TransitionIntent::ConsultReturn { .. }) {
                if matches!(
                    record.state,
                    OperationState::Accepted | OperationState::Preparing | OperationState::Unknown
                ) {
                    runtime.queue_consultation_return(key, command, record.record_id.clone());
                }
            } else if matches!(
                record.state,
                OperationState::Accepted | OperationState::Preparing | OperationState::Unknown
            ) {
                runtime.queue_consultation(key, command, record.record_id.clone());
            }
        }
        if result.is_ok() {
            self.notify_changed();
        }
        result
    }
}
