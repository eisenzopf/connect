"""G6-059 declaration-only target; no SDK transport or implementation is supplied."""
from __future__ import annotations
from dataclasses import dataclass
from typing import AsyncIterator, Generic, Literal, Mapping, NewType, Protocol, Sequence, TypeVar

ConversationId = NewType('ConversationId', str)
ParticipantId = NewType('ParticipantId', str)
SessionId = NewType('SessionId', str)
ConnectionId = NewType('ConnectionId', str)
StreamId = NewType('StreamId', str)
OperationId = NewType('OperationId', str)
MessageId = NewType('MessageId', str)
EndpointId = NewType('EndpointId', str)
IdentityId = NewType('IdentityId', str)
Cursor = NewType('Cursor', str)
SessionRevision = NewType('SessionRevision', int)
TopologyRevision = NewType('TopologyRevision', int)
ConversationRevision = NewType('ConversationRevision', int)
StreamRevision = NewType('StreamRevision', int)
type Json = None | bool | int | float | str | Sequence[Json] | Mapping[str, Json]
type Medium = Literal['audio', 'video', 'screen', 'data']
type OperationState = Literal['accepted', 'preparing', 'ready', 'committing', 'completed', 'cancelled', 'failed', 'interrupted', 'unknown']
T = TypeVar('T')

@dataclass(frozen=True)
class NonEmpty(Generic[T]):
    first: T
    rest: Sequence[T] = ()

@dataclass(frozen=True)
class Scope:
    conversation_id: ConversationId | None = None
    session_id: SessionId | None = None
    connection_id: ConnectionId | None = None

@dataclass(frozen=True)
class OperationKey:
    operation_id: OperationId
    command: str
    scope: Scope

@dataclass(frozen=True)
class OperationReference(OperationKey, Generic[T]):
    """Produced/restored by the SDK's command-kind validator, not an unchecked cast."""

class Cancellation(Protocol):
    def cancel(self) -> None: ...
    @property
    def is_cancelled(self) -> bool: ...

@dataclass(frozen=True)
class RequestOptions:
    timeout_ms: int
    signal: Cancellation | None = None

@dataclass(frozen=True)
class MutationOptions:
    operation_id: OperationId
    request: RequestOptions
    extensions: Mapping[str, Json] | None = None

@dataclass(frozen=True)
class CanonicalProblem:
    code: int
    category: str
    reason: str

class ConnectError(Exception):
    kind: Literal['validation', 'configuration', 'trust', 'authentication', 'protocol', 'deadline', 'cancelled', 'transport-uncertain', 'unsupported', 'conflict', 'application', 'receipt-unavailable', 'resync-required', 'cleanup-incomplete']
    certainty: Literal['not-submitted', 'refused', 'uncertain', 'recorded', 'read-only']
    operation: OperationKey | None
    problem: CanonicalProblem | None

@dataclass(frozen=True)
class OperationSnapshot:
    key: OperationKey
    state: OperationState
    revision: int
    cancel_requested: bool | None
    expires_at: int | None
    problem: Json

@dataclass(frozen=True)
class Completion(Generic[T]):
    snapshot: OperationSnapshot
    value: T

@dataclass(frozen=True)
class DecisionReceipt:
    decision: OperationKey
    target: OperationId
    observed_state: OperationState
    disposition: str
    evidence_id: str | None

class Operation(Protocol[T]):
    def reference_json(self) -> Json: ...
    @property
    def reference(self) -> OperationReference[T]: ...
    async def status(self, options: RequestOptions) -> OperationSnapshot: ...
    async def wait(self, options: RequestOptions) -> Completion[T]: ...
    async def wait_until_ready(self, options: RequestOptions) -> OperationSnapshot: ...
    async def cancel(self, options: MutationOptions) -> Operation[DecisionReceipt]: ...
    async def reconcile(self, outcome: Literal['completed', 'cancelled', 'failed'], evidence_id: str, options: MutationOptions) -> Operation[DecisionReceipt]: ...

@dataclass(frozen=True)
class SessionSnapshot:
    id: SessionId
    conversation_id: ConversationId
    state: str
    media: Sequence[Medium]
    intent: str | None
    capabilities_offer: Json
    codec_preferences: Sequence[str]
    is_main: bool
    revision: SessionRevision
    topology_revision: TopologyRevision
    created_at: int
    updated_at: int
    ended_reason: str | None

@dataclass(frozen=True)
class ParticipantSnapshot:
    id: ParticipantId
    conversation_id: ConversationId
    identity_id: IdentityId
    kind: Literal["human", "ai", "system", "external"]
    removal_reason: str | None
    membership_epoch: int
    active: bool
    role: str

@dataclass(frozen=True)
class ConversationSnapshot:
    id: ConversationId
    revision: ConversationRevision
    participants: Sequence[ParticipantSnapshot]
    closed: bool

@dataclass(frozen=True)
class ConnectionSnapshot:
    id: ConnectionId
    conversation_id: ConversationId
    session_id: SessionId
    generation: int
    stream_revision: StreamRevision | None
    state: str

type Direction = Literal["sendonly", "recvonly", "sendrecv", "inactive"]

# Historical captured parameters, never a local ownership grant.
@dataclass(frozen=True)
class NegotiatedStream:
    id: StreamId
    medium: Medium
    direction: Direction
    codec: Json
    stream_local_id: int | None

@dataclass(frozen=True)
class CodecObservation:
    parameters: NegotiatedStream
    stream_generation: int
    connection_generation: int

@dataclass(frozen=True)
class StreamGateObservation(CodecObservation):
    state: Literal['active', 'paused']
    source_muted: bool
    held_direction: Direction | None

@dataclass(frozen=True)
class CodecOutcome:
    resources_changed: bool
    streams: Sequence[CodecObservation]

@dataclass(frozen=True)
class GateOutcome:
    action: Literal['hold', 'resume', 'mute', 'unmute']
    streams: Sequence[StreamGateObservation]

@dataclass(frozen=True)
class QualityOutcome:
    interval_ms: int

@dataclass(frozen=True)
class IceRestartOutcome:
    ice_generation: int

type ConnectionUpdateOutcome = CodecOutcome | GateOutcome | QualityOutcome | IceRestartOutcome

@dataclass(frozen=True)
class ConnectionUpdateReceipt:
    connection: Connection
    connection_generation: int
    stream_revision: StreamRevision
    outcome: ConnectionUpdateOutcome

@dataclass(frozen=True)
class StreamTarget:
    stream: StreamId

@dataclass(frozen=True)
class ParticipantStreams:
    participant: ParticipantId
    kinds: NonEmpty[Medium] | None = None

type StreamSelector = StreamTarget | ParticipantStreams

@dataclass(frozen=True)
class StreamSelectionReceipt:
    connection: Connection
    connection_generation: int
    stream_revision: StreamRevision
    resources_changed: bool
    # None is unavailable capture; [] is a known empty binding set.
    bindings: Sequence[CodecObservation] | None

@dataclass(frozen=True)
class MessageAcceptance:
    message_id: MessageId
    operation: OperationKey
    boundary: Literal['local-mutation']

@dataclass(frozen=True)
class DeliveryObservation:
    participant_id: ParticipantId
    membership_epoch: int
    channel: str
    delivery_id: str
    state: Literal['queued', 'accepted', 'delivered', 'read', 'failed', 'unknown']
    provenance: str
    observed_at: int

@dataclass(frozen=True)
class TopologyChange:
    retained: Sequence[Connection]
    admitted: Sequence[Connection]
    retired: Sequence[ConnectionId]
    topology_revision: TopologyRevision
    conversation_revision: ConversationRevision | None

@dataclass(frozen=True)
class ConsultationResult(TopologyChange):
    session: Session

@dataclass(frozen=True)
class HandoffResult(TopologyChange):
    pass

@dataclass(frozen=True)
class MoveResult(TopologyChange):
    connection: Connection

@dataclass(frozen=True)
class SessionDecision:
    session: Session
    state: str
    topology_revision: TopologyRevision

@dataclass(frozen=True)
class EventSource:
    kind: str
    correlation_id: str

@dataclass(frozen=True)
class AudienceMember:
    participant_id: ParticipantId
    membership_epoch: int

@dataclass(frozen=True)
class JournalEvent:
    event_id: str
    cursor: Cursor
    conversation_id: ConversationId
    session_id: SessionId | None
    connection_id: ConnectionId | None
    operation_id: OperationId | None
    participant_id: ParticipantId | None
    audience: Sequence[AudienceMember]
    kind: str
    revision: int
    source: EventSource
    observed_at: int
    data: Json

class EventStream(Protocol):
    def __aiter__(self) -> AsyncIterator[JournalEvent]: ...
    async def close(self) -> None: ...

@dataclass(frozen=True)
class IncomingInvitation:
    session: Session
    from_participant: ParticipantId
    operation_id: OperationId
    source: EventSource

@dataclass(frozen=True)
class CapabilitySnapshot:
    profile: Literal['connect/2']
    runtime_role: Literal['rust', 'browser', 'node', 'python', 'http-control']
    selected_transport: Literal['quic', 'webtransport', 'wss-webrtc', 'http']
    fallback_reason: str | None
    features: frozenset[str]

class Capabilities(Protocol):
    async def refresh(self, options: RequestOptions) -> CapabilitySnapshot: ...
    async def require(self, feature: str, options: RequestOptions) -> None: ...

class ValidatedEnvelope(Protocol):
    @property
    def value(self) -> Json: ...

class RawCanonical(Protocol):
    def validate(self, value: Json) -> ValidatedEnvelope: ...
    async def request(self, envelope: ValidatedEnvelope, options: RequestOptions) -> Sequence[ValidatedEnvelope]: ...

class Participant(Protocol):
    @property
    def id(self) -> ParticipantId: ...
    @property
    def conversation_id(self) -> ConversationId: ...
    async def refresh(self, options: RequestOptions) -> ParticipantSnapshot: ...
    async def update(self, role: str, expected_conversation_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]: ...
    async def remove(self, expected_conversation_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]: ...

@dataclass(frozen=True)
class ConsultInput:
    via: ConnectionId
    retain: ConnectionId
    with_participants: NonEmpty[ParticipantId]
    hold: Literal['silence', 'configured-media']
    expected_topology_revision: TopologyRevision

@dataclass(frozen=True)
class SessionRole:
    scope: Literal['session'] = 'session'

@dataclass(frozen=True)
class ConversationRole:
    expected_conversation_revision: ConversationRevision
    scope: Literal['conversation'] = 'conversation'

@dataclass(frozen=True)
class HandoffInput:
    via: ConnectionId
    from_participant: ParticipantId
    to: ParticipantId
    retain: ConnectionId
    outgoing: Literal['leave', 'observe']
    role: SessionRole | ConversationRole
    expected_topology_revision: TopologyRevision
    candidate: ConnectionId | None = None
    consultation: ConsultationOperation | None = None

@dataclass(frozen=True)
class EndpointTarget:
    endpoint: EndpointId

@dataclass(frozen=True)
class CandidateTarget:
    candidate: ConnectionId

@dataclass(frozen=True)
class MoveInput:
    retain: ConnectionId
    target: EndpointTarget | CandidateTarget
    expected_topology_revision: TopologyRevision

class ConsultationOperation(Operation[ConsultationResult], Protocol):
    async def return_to_retained(self, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]: ...

@dataclass(frozen=True)
class ParticipantLeave:
    participant_id: ParticipantId

@dataclass(frozen=True)
class RoleChange:
    participant_id: ParticipantId
    role: str

@dataclass(frozen=True)
class MediumUpgrade:
    media: NonEmpty[Medium]

@dataclass(frozen=True)
class CodecRenegotiate:
    codec_preferences: NonEmpty[str]

type SessionUpdate = ParticipantLeave | RoleChange | MediumUpgrade | CodecRenegotiate

@dataclass(frozen=True)
class Mute:
    action: Literal['mute', 'unmute']
    streams: NonEmpty[StreamId]

@dataclass(frozen=True)
class HoldConnection:
    direction: Literal['sendonly', 'recvonly', 'inactive']
    streams: NonEmpty[StreamId] | None = None

@dataclass(frozen=True)
class ResumeConnection:
    streams: NonEmpty[StreamId] | None = None

@dataclass(frozen=True)
class Quality:
    interval_ms: int

@dataclass(frozen=True)
class ConnectionCodecRenegotiate:
    codec_preferences: NonEmpty[str]
    streams: NonEmpty[StreamId] | None = None

type ConnectionUpdate = Mute | HoldConnection | ResumeConnection | Quality | ConnectionCodecRenegotiate

class Connections(Protocol):
    def by_id(self, id: ConnectionId) -> Connection: ...
    async def offer(self, connection_id: ConnectionId, streams: NonEmpty[OfferedStream], options: MutationOptions) -> Operation[Connection]: ...

@dataclass(frozen=True)
class OfferedStream:
    id: StreamId
    kind: Medium
    direction: Literal['sendonly', 'recvonly', 'sendrecv', 'inactive']
    codecs: NonEmpty[str]

class Session(Protocol):
    async def invite_participants(self, to: NonEmpty[ParticipantId], medium: Literal['voice', 'video', 'voice+video', 'screen-share', 'text-chat', 'mixed'], intent: str, capabilities: Json, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]: ...
    @property
    def id(self) -> SessionId: ...
    @property
    def conversation_id(self) -> ConversationId: ...
    @property
    def connections(self) -> Connections: ...
    async def refresh(self, options: RequestOptions) -> SessionSnapshot: ...
    async def accept(self, capabilities: Json, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]: ...
    async def reject(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[SessionDecision]: ...
    async def cancel_invitation(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[SessionDecision]: ...
    async def consult(self, input: ConsultInput, options: MutationOptions) -> ConsultationOperation: ...
    async def handoff(self, input: HandoffInput, options: MutationOptions) -> Operation[HandoffResult]: ...
    async def update(self, input: SessionUpdate, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]: ...
    async def end(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]: ...

class Streams(Protocol):
    def by_id(self, id: StreamId) -> Stream: ...
    async def subscribe(self, selectors: NonEmpty[StreamSelector], expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[StreamSelectionReceipt]: ...
    async def unsubscribe(self, streams: NonEmpty[StreamId], expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[StreamSelectionReceipt]: ...

class Connection(Protocol):
    @property
    def id(self) -> ConnectionId: ...
    @property
    def session_id(self) -> SessionId: ...
    @property
    def conversation_id(self) -> ConversationId: ...
    @property
    def streams(self) -> Streams: ...
    async def refresh(self, options: RequestOptions) -> ConnectionSnapshot: ...
    async def update(self, input: ConnectionUpdate, expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[ConnectionUpdateReceipt]: ...
    async def replace_device(self, input: MoveInput, options: MutationOptions) -> Operation[MoveResult]: ...
    async def end(self, reason_code: int, reason: str, expected_stream_revision: StreamRevision | None, options: MutationOptions) -> Operation[Connection]: ...

@dataclass(frozen=True)
class MediaFormat:
    codec: str
    clock_rate: int
    channels: int | None
    width: int | None
    height: int | None

@dataclass(frozen=True)
class SourceClock:
    source_id: str
    generation: int
    units_per_second: int
    synchronized: bool

@dataclass(frozen=True)
class StreamSnapshot:
    id: StreamId
    conversation_id: ConversationId
    session_id: SessionId
    connection_id: ConnectionId
    medium: Medium
    state: str
    direction: Literal["sendonly", "recvonly", "sendrecv", "inactive"] | None
    format: MediaFormat | None
    clock: SourceClock | None
    ownership: Literal["observation", "local-source", "local-sink", "local-duplex"]

class Stream(Protocol):
    async def refresh(self, options: RequestOptions) -> StreamSnapshot: ...
    @property
    def id(self) -> StreamId: ...
    @property
    def connection_id(self) -> ConnectionId: ...
    @property
    def session_id(self) -> SessionId: ...
    @property
    def conversation_id(self) -> ConversationId: ...

class Participants(Protocol):
    def by_id(self, id: ParticipantId) -> Participant: ...
    async def add(self, identity_id: IdentityId, kind: Literal["human", "ai", "system", "external"], role: str, expected_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]: ...

class Sessions(Protocol):
    def by_id(self, id: SessionId) -> Session: ...
    async def invite(self, session_id: SessionId, to: Sequence[ParticipantId], medium: Literal['voice', 'video', 'voice+video', 'screen-share', 'text-chat', 'mixed'], intent: str, capabilities: Json, options: MutationOptions) -> Operation[Session]: ...

class Conversation(Protocol):
    @property
    def id(self) -> ConversationId: ...
    @property
    def participants(self) -> Participants: ...
    @property
    def sessions(self) -> Sessions: ...
    async def refresh(self, options: RequestOptions) -> ConversationSnapshot: ...
    async def send(self, message_id: MessageId, to: NonEmpty[ParticipantId], text: str, options: MutationOptions) -> Operation[MessageAcceptance]: ...
    async def receipts(self, message_id: MessageId, options: RequestOptions) -> Sequence[DeliveryObservation]: ...
    def events(self, after: Cursor | None, options: RequestOptions) -> EventStream: ...

class Conversations(Protocol):
    def by_id(self, id: ConversationId) -> Conversation: ...
    async def create(self, participants: Sequence[IdentityId], policy: Literal['persistent', 'ephemeral'], options: MutationOptions) -> Operation[Conversation]: ...

class OperationKind(Protocol[T]):
    """SDK-created command-kind witness; implementations reject foreign witnesses."""
    @property
    def command(self) -> str: ...
    def _result_type(self, value: T) -> T: ...

class OperationKinds(Protocol):
    @property
    def conversation_create(self) -> OperationKind[Conversation]: ...
    @property
    def message_send(self) -> OperationKind[MessageAcceptance]: ...
    @property
    def conversation_participant_add(self) -> OperationKind[Participant]: ...
    @property
    def conversation_participant_update(self) -> OperationKind[Participant]: ...
    @property
    def conversation_participant_remove(self) -> OperationKind[Participant]: ...
    @property
    def session_invite(self) -> OperationKind[Session]: ...
    @property
    def session_accept(self) -> OperationKind[Session]: ...
    @property
    def session_reject(self) -> OperationKind[SessionDecision]: ...
    @property
    def session_cancel(self) -> OperationKind[SessionDecision]: ...
    @property
    def session_consult(self) -> OperationKind[ConsultationResult]: ...
    @property
    def session_consult_return(self) -> OperationKind[Session]: ...
    @property
    def session_handoff(self) -> OperationKind[HandoffResult]: ...
    @property
    def session_update(self) -> OperationKind[Session]: ...
    @property
    def session_end(self) -> OperationKind[Session]: ...
    @property
    def connection_offer(self) -> OperationKind[Connection]: ...
    @property
    def connection_update(self) -> OperationKind[ConnectionUpdateReceipt]: ...
    @property
    def connection_replace(self) -> OperationKind[MoveResult]: ...
    @property
    def connection_end(self) -> OperationKind[Connection]: ...
    @property
    def stream_subscribe(self) -> OperationKind[StreamSelectionReceipt]: ...
    @property
    def stream_unsubscribe(self) -> OperationKind[StreamSelectionReceipt]: ...
    @property
    def operation_cancel(self) -> OperationKind[DecisionReceipt]: ...
    @property
    def operation_reconcile(self) -> OperationKind[DecisionReceipt]: ...

class Operations(Protocol):
    def restore[S](self, saved: Json, kind: OperationKind[S]) -> Operation[S]: ...
    def recover[S](self, reference: OperationReference[S]) -> Operation[S]: ...

class InvitationStream(Protocol):
    def __aiter__(self) -> AsyncIterator[IncomingInvitation]: ...
    async def close(self) -> None: ...

class ParticipantClient(Protocol):
    @property
    def operation_kinds(self) -> OperationKinds: ...
    @property
    def conversations(self) -> Conversations: ...
    @property
    def operations(self) -> Operations: ...
    @property
    def capabilities(self) -> Capabilities: ...
    @property
    def raw(self) -> RawCanonical: ...
    def incoming_invitations(self, options: RequestOptions) -> InvitationStream: ...
    async def close(self, options: RequestOptions) -> None: ...
    async def __aenter__(self) -> ParticipantClient: ...
    async def __aexit__(self, exc_type: object, exc: object, traceback: object) -> None: ...

class TrustedProvisioningCommand(Protocol):
    @property
    def value(self) -> Json: ...

class AdministrativeClient(Protocol):
    def validate_provisioning(self, value: Json) -> TrustedProvisioningCommand: ...
    async def provision(self, command: TrustedProvisioningCommand, options: MutationOptions) -> Operation[Json]: ...
    async def close(self, options: RequestOptions) -> None: ...
