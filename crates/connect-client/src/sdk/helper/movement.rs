//! Typed transitions delegate custody, current authority and deadlines to the SDK.
use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    via: ConnectionId,
    retain: ConnectionId,
    from_participant: ParticipantId,
    to: ParticipantId,
    outgoing: Outgoing,
    role: HandoffRole,
    candidate: Option<ConnectionId>,
    consultation: Option<Value>,
    expected_revision: TopologyRevision,
    mutation: Mutation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveRequest {
    retain: ConnectionId,
    target: MoveTarget,
    expected_revision: TopologyRevision,
    mutation: Mutation,
}
pub(super) async fn handoff(
    client: &Client,
    cid: ConversationId,
    sid: SessionId,
    input: HandoffRequest,
    options: RequestOptions,
) -> Result<Value> {
    let consultation = input
        .consultation
        .map(|saved| {
            client
                .restore(saved, client.operation_kinds().session_consult)
                .map(|op| op.reference())
        })
        .transpose()?;
    Ok(client
        .conversation_by_id(cid)
        .session_by_id(sid)
        .handoff(
            HandoffInput {
                via: input.via,
                retain: input.retain,
                from_participant: input.from_participant,
                to: input.to,
                outgoing: input.outgoing,
                role: input.role,
                candidate: input.candidate,
                consultation,
                expected_topology_revision: input.expected_revision,
            },
            input.mutation.options(options),
        )
        .await?
        .reference_json())
}
pub(super) async fn replace(
    client: &Client,
    cid: ConversationId,
    sid: SessionId,
    connid: ConnectionId,
    input: MoveRequest,
    options: RequestOptions,
) -> Result<Value> {
    Ok(client
        .conversation_by_id(cid)
        .session_by_id(sid)
        .connection_by_id(connid)
        .replace_device(
            MoveInput {
                retain: input.retain,
                target: input.target,
                expected_topology_revision: input.expected_revision,
            },
            input.mutation.options(options),
        )
        .await?
        .reference_json())
}
fn topology(v: TopologyChange) -> Value {
    json!({"retained":v.retained.into_iter().map(super::connection::handle_json).collect::<Vec<_>>(),
        "admitted":v.admitted.into_iter().map(super::connection::handle_json).collect::<Vec<_>>(),
        "retired":v.retired,"topologyRevision":v.topology_revision,"conversationRevision":v.conversation_revision})
}
pub(super) fn handoff_json(v: HandoffResult) -> Value {
    let mut result = topology(v.topology);
    result["roleChange"] = v.role_change.map(|v|json!({"participantRevision":v.participant_revision,"roleScope":v.role_scope,
        "outgoing":v.outgoing,"sourceParticipant":v.source_participant,"targetParticipant":v.target_participant,"cleanup":v.cleanup})).unwrap_or(Value::Null);
    result
}
pub(super) fn move_json(v: MoveResult) -> Value {
    let mut result = topology(v.topology);
    result["connection"] = super::connection::handle_json(v.connection);
    result["binding"] = json!({"generation":v.binding.generation,"participantId":v.binding.participant_id,"membershipEpoch":v.binding.membership_epoch,
        "endpointId":v.binding.endpoint_id,"deviceId":v.binding.device_id});
    result
}
