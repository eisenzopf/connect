use super::*;
use connect_client::transitions::{OutgoingDisposition, RoleScope, TransitionIntent};
fn topology(value: &sdk::TopologyChange) -> Value {
    json!({"topology_revision":value.topology_revision.get(),"retained":value.retained.iter().map(|c|c.id().as_str()).collect::<Vec<_>>(),"admitted":value.admitted.iter().map(|c|c.id().as_str()).collect::<Vec<_>>(),"retired":value.retired.iter().map(|id|id.as_str()).collect::<Vec<_>>()})
}
fn handoff(value: &sdk::HandoffResult) -> Value {
    let role=value.role_change.as_ref().map(|r|json!({"participant_revision":r.participant_revision,"role_scope":r.role_scope,"outgoing":r.outgoing,"source_participant":r.source_participant,"target_participant":r.target_participant,"cleanup":r.cleanup}));
    json!({"topology":topology(&value.topology),"role_change":role})
}
fn movement(value: &sdk::MoveResult) -> Result<Value> {
    Ok(
        json!({"topology":topology(&value.topology),"connection_id":value.connection.id().as_str(),"generation":value.binding.generation,"participant_id":value.binding.participant_id,"membership_epoch":value.binding.membership_epoch,"endpoint_id":value.binding.endpoint_id,"device_id":value.binding.device_id,"native_owned":value.connection.native_negotiation()?.is_some()}),
    )
}
async fn report<T>(
    operation: sdk::Operation<T>,
    summarize: impl Fn(&T) -> Result<Value>,
) -> Result<(Value, Value)> {
    let reference = operation.reference_json();
    emit(json!({"phase":"accepted","reference":reference})).await?;
    observe(operation, summarize).await
}
async fn observe<T>(
    operation: sdk::Operation<T>,
    summarize: impl Fn(&T) -> Result<Value>,
) -> Result<(Value, Value)> {
    let reference = operation.reference_json();
    match operation.wait(options()).await {
        Ok(completion) => Ok((
            reference,
            json!({"phase":"completed","summary":summarize(&completion.value)?}),
        )),
        Err(error) => Ok((
            reference,
            json!({"phase":if error.certainty==sdk::Certainty::Recorded{"stopped"}else{"error"},"state":error.observation.as_ref().map(|o|o.state),"kind":format!("{:?}",error.kind),"certainty":format!("{:?}",error.certainty)}),
        )),
    }
}
pub(super) async fn run(
    config: ClientConfig,
    command: TransitionCommand,
    consultation: Option<Value>,
) -> Result<()> {
    let client = Client::connect_configured(config.clone()).await?;
    let result = perform(&client, &command, consultation).await;
    client.close(options()).await?;
    let (reference, mut terminal) = result?;
    if terminal["phase"] == "completed" {
        // A fresh physical reader reconstructs the original; it never replays it.
        let fresh = Client::connect_configured(config).await?;
        let recovered = restored(&fresh, &command, reference.clone()).await;
        fresh.close(options()).await?;
        let (_, recovered) = recovered?;
        if recovered["summary"] != terminal["summary"] {
            return Err("fresh recovery changed original captured result".into());
        }
        terminal["recovered"] = json!(true);
    }
    terminal["reference"] = reference;
    emit(terminal).await
}
async fn perform(
    client: &Client,
    command: &TransitionCommand,
    consultation: Option<Value>,
) -> Result<(Value, Value)> {
    let room = client.conversation_by_id(command.conversation_id.parse()?);
    room.refresh(options()).await?;
    client
        .capabilities()
        .require(sdk::features::ORIGINAL_ID_OPERATIONS, options())
        .await?;
    emit(json!({"phase":"registered"})).await?;
    proceed().await?;
    let session = room.session_by_id(command.session_id.parse()?);
    let mut options = mutation(&command.operation_id)?;
    let mut extensions = command.payload_extensions.clone();
    extensions.remove("consultation_record_id");
    options.extensions = (!extensions.is_empty()).then_some(extensions);
    match &command.intent {
        TransitionIntent::Handoff {
            source_participant,
            target_participant,
            retained_connid,
            candidate_connid,
            outgoing,
            role_scope,
            expected_conversation_revision,
        } => {
            let parent = consultation
                .map(|saved| client.restore(saved, client.operation_kinds().session_consult))
                .transpose()?;
            let operation = session
                .handoff(
                    sdk::HandoffInput {
                        via: command
                            .connection_id
                            .as_ref()
                            .ok_or("missing source Connection")?
                            .parse()?,
                        from_participant: source_participant.parse()?,
                        to: target_participant.parse()?,
                        retain: retained_connid.parse()?,
                        outgoing: if *outgoing == OutgoingDisposition::Leave {
                            sdk::Outgoing::Leave
                        } else {
                            sdk::Outgoing::Observe
                        },
                        role: if *role_scope == RoleScope::Session {
                            sdk::HandoffRole::Session
                        } else {
                            sdk::HandoffRole::Conversation {
                                expected_conversation_revision:
                                    sdk::ConversationRevision::try_from(
                                        expected_conversation_revision
                                            .ok_or("missing Conversation revision")?,
                                    )?,
                            }
                        },
                        expected_topology_revision: sdk::TopologyRevision::try_from(
                            command.expected_revision,
                        )?,
                        candidate: candidate_connid.as_ref().map(|s| s.parse()).transpose()?,
                        consultation: parent.map(|p| p.reference()),
                    },
                    options,
                )
                .await?;
            report(operation, |v| Ok(handoff(v))).await
        }
        TransitionIntent::Replace {
            endpoint_id,
            candidate_connid,
            retained_connid,
        } => {
            let target = match (endpoint_id, candidate_connid) {
                (Some(e), None) => sdk::MoveTarget::Endpoint(e.parse()?),
                (None, Some(c)) => sdk::MoveTarget::Candidate(c.parse()?),
                _ => return Err("exactly one replacement target required".into()),
            };
            let operation = session
                .connection_by_id(
                    command
                        .connection_id
                        .as_ref()
                        .ok_or("missing source Connection")?
                        .parse()?,
                )
                .replace_device(
                    sdk::MoveInput {
                        retain: retained_connid.parse()?,
                        target,
                        expected_topology_revision: sdk::TopologyRevision::try_from(
                            command.expected_revision,
                        )?,
                    },
                    options,
                )
                .await?;
            report(operation, movement).await
        }
        _ => Err("unsupported transition case".into()),
    }
}
async fn restored(
    client: &Client,
    command: &TransitionCommand,
    reference: Value,
) -> Result<(Value, Value)> {
    match command.intent {
        TransitionIntent::Handoff { .. } => {
            observe(
                client.restore(reference, client.operation_kinds().session_handoff)?,
                |v| Ok(handoff(v)),
            )
            .await
        }
        TransitionIntent::Replace { .. } => {
            observe(
                client.restore(reference, client.operation_kinds().connection_replace)?,
                movement,
            )
            .await
        }
        _ => Err("unsupported recovery kind".into()),
    }
}
pub(super) async fn recover(
    config: ClientConfig,
    command: TransitionCommand,
    reference: Value,
) -> Result<()> {
    let client = Client::connect_configured(config).await?;
    let result = restored(&client, &command, reference).await;
    client.close(options()).await?;
    let (reference, mut terminal) = result?;
    terminal["reference"] = reference;
    emit(terminal).await
}
