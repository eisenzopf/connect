// Each @ts-expect-error must remain an actual type error, never a cast or ignore.
import type {
    ParticipantClient,
    SessionId,
    ParticipantId,
    ConnectionId,
    EndpointId,
    Revision,
    MoveTarget,
    NonEmpty,
    RequestOptions,
    OperationReference,
    MoveResult,
    MessageAcceptance,
    Operation,
} from "./contract.js";
declare const participant: ParticipantId,
    connection: ConnectionId,
    endpoint: EndpointId;
declare const sessionRevision: Revision<"session">,
    client: ParticipantClient,
    wait: RequestOptions;
declare const message: OperationReference<MessageAcceptance>;
// @ts-expect-error Participant and Session IDs cannot substitute for one another.
const session: SessionId = participant;
// @ts-expect-error A Session observation revision cannot authorize a topology mutation.
const topology: Revision<"topology"> = sessionRevision;
// @ts-expect-error Device movement selects exactly one endpoint or candidate.
const move: MoveTarget = { endpoint, candidate: connection };
// @ts-expect-error A private audience must have a first Participant.
const audience: NonEmpty<ParticipantId> = { rest: [] };
// @ts-expect-error Participant credentials have no administrative provisioning surface.
client.provision({}, wait);
// @ts-expect-error Restoring a message reference cannot create a typed move Operation.
const wrongRecovery: Operation<MoveResult> = client.operations.recover(message);
void [session, topology, move, audience, wrongRecovery];
