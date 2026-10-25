//! Must fail with six independent type errors; never compiled as a positive SDK.
mod contract;
use contract::*;
pub fn reject<A: Api>(
    client: &impl ParticipantClient<A>,
    participant: ParticipantId,
    session_revision: SessionRevision,
    endpoint: EndpointId,
    connection: ConnectionId,
    wait: RequestOptions,
    message: OperationReference<MessageAcceptance>,
) {
    let _session: SessionId = participant;
    let _topology: TopologyRevision = session_revision;
    let _move = MoveTarget::Endpoint(endpoint, connection);
    let _audience: NonEmpty<ParticipantId> = NonEmpty { rest: vec![] };
    let _provisioning = client.provision(Json::Null, wait);
    // Result typing must stay bound to the original command reference.
    let wrong = client.recover(message);
    fn only_move<C>(_: &impl Operation<MoveResult<C>>) {}
    only_move::<A::Connection>(&wrong);
}
