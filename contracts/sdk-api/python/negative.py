"""Each expected diagnostic is checked against an unannotated negative run."""
from contract import (
    ParticipantClient, ParticipantId, SessionId, SessionRevision, TopologyRevision,
    EndpointId, ConnectionId, EndpointTarget, NonEmpty, RequestOptions,
    OperationReference, MessageAcceptance, MoveResult, Operation,
)

def reject(client: ParticipantClient, participant: ParticipantId, session_revision: SessionRevision,
           endpoint: EndpointId, connection: ConnectionId, wait: RequestOptions,
           message: OperationReference[MessageAcceptance]) -> None:
    session: SessionId = participant  # type: ignore[assignment]
    topology: TopologyRevision = session_revision  # type: ignore[assignment]
    move = EndpointTarget(endpoint, connection)  # type: ignore[call-arg]
    audience: NonEmpty[ParticipantId] = NonEmpty()  # type: ignore[call-arg]
    client.provision({}, wait)  # type: ignore[attr-defined]
    recovery: Operation[MoveResult] = client.operations.recover(message)  # type: ignore[arg-type]
    _ = session, topology, move, audience, recovery
