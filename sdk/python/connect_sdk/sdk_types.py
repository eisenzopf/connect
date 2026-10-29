"""Current SDK intent/options; observations and diagnostics confer no authority."""
from __future__ import annotations

from dataclasses import dataclass
import asyncio
import re
import threading
from types import MappingProxyType
from typing import Generic, Literal, Mapping, NewType, Protocol, Sequence, TypeAlias, TypeVar, Union

ConversationId = NewType("ConversationId", str)
ParticipantId = NewType("ParticipantId", str)
SessionId = NewType("SessionId", str)
ConnectionId = NewType("ConnectionId", str)
StreamId = NewType("StreamId", str)
OperationId = NewType("OperationId", str)
MessageId = NewType("MessageId", str)
EndpointId = NewType("EndpointId", str)
IdentityId = NewType("IdentityId", str)
Cursor = NewType("Cursor", str)
ConversationRevision = NewType("ConversationRevision", int)
SessionRevision = NewType("SessionRevision", int)
TopologyRevision = NewType("TopologyRevision", int)
StreamRevision = NewType("StreamRevision", int)
T = TypeVar("T")
IdentityKind: TypeAlias = Literal["human", "ai", "system", "external"]
OperationState: TypeAlias = Literal["accepted", "preparing", "ready", "committing", "completed", "cancelled", "failed", "interrupted", "unknown"]
DeliveryState: TypeAlias = Literal["queued", "accepted", "delivered", "read", "failed", "unknown"]
DeliveryProvenance: TypeAlias = Literal["journal", "client_reported", "provider_observation", "local_attempt"]
Json: TypeAlias = Union[None, bool, int, float, str, Sequence["Json"], Mapping[str, "Json"]]
ErrorKind: TypeAlias = Literal["validation", "configuration", "trust", "authentication", "protocol", "deadline", "cancelled", "transport-uncertain", "unsupported", "conflict", "application", "receipt-unavailable", "resync-required", "cleanup-incomplete"]
Certainty: TypeAlias = Literal["not-submitted", "refused", "uncertain", "recorded", "read-only"]
ERROR_KINDS = frozenset(("validation", "configuration", "trust", "authentication", "protocol", "deadline", "cancelled", "transport-uncertain", "unsupported", "conflict", "application", "receipt-unavailable", "resync-required", "cleanup-incomplete"))
CERTAINTIES = frozenset(("not-submitted", "refused", "uncertain", "recorded", "read-only"))


def identifier(value: object) -> bool:
    return type(value) is str and re.fullmatch(r"[A-Za-z0-9._~-]{1,128}", value) is not None


@dataclass(frozen=True)
class Scope:
    conversation_id: ConversationId | None = None
    session_id: SessionId | None = None
    connection_id: ConnectionId | None = None

    def __post_init__(self):
        if any(value is not None and not identifier(value) for value in (self.conversation_id, self.session_id, self.connection_id)):
            raise SdkError("validation", "read-only")
        if self.session_id is not None and self.conversation_id is None or self.connection_id is not None and self.session_id is None:
            raise SdkError("validation", "read-only")


@dataclass(frozen=True)
class OperationKey:
    operation_id: OperationId
    command: str
    scope: Scope = Scope()

    def __post_init__(self):
        if not identifier(self.operation_id) or type(self.command) is not str or not re.fullmatch(r"[a-z][a-z0-9.-]{0,127}", self.command) or type(self.scope) is not Scope:
            raise SdkError("validation", "not-submitted")

    def _wire(self):
        return {"operationId": self.operation_id, "command": self.command, "scope": {
            "conversationId": self.scope.conversation_id, "sessionId": self.scope.session_id,
            "connectionId": self.scope.connection_id,
        }}


class CancellationSignal(Protocol):
    def cancel(self) -> None: ...
    @property
    def is_cancelled(self) -> bool: ...


class Cancellation:
    """Thread-safe explicit cancellation; it never cancels another owner."""
    def __init__(self):
        self._event = threading.Event()

    def cancel(self) -> None:
        self._event.set()

    @property
    def is_cancelled(self) -> bool:
        return self._event.is_set()


def cancelled(signal: CancellationSignal | None) -> bool:
    return signal is not None and signal.is_cancelled


@dataclass(frozen=True)
class RequestOptions:
    timeout_ms: int
    signal: CancellationSignal | None = None

    def __post_init__(self):
        # Match the original Rust SDK's finite request range.
        if type(self.timeout_ms) is not int or not 1 <= self.timeout_ms <= 60_000:
            raise SdkError("validation", "read-only")
        try:
            if self.signal is not None and (type(self.signal.is_cancelled) is not bool or not callable(self.signal.cancel)):
                raise ValueError()
        except Exception:
            raise SdkError("validation", "read-only") from None


@dataclass(frozen=True)
class NonEmpty(Generic[T]):
    first: T
    rest: Sequence[T] = ()

    def __post_init__(self):
        if not isinstance(self.rest, (list, tuple)):
            raise SdkError("validation", "not-submitted")
        object.__setattr__(self, "rest", tuple(self.rest))


@dataclass(frozen=True, repr=False)
class MutationOptions:
    operation_id: OperationId
    request: RequestOptions
    extensions: Mapping[str, Json] | None = None

    def __post_init__(self):
        if not identifier(self.operation_id) or type(self.request) is not RequestOptions:
            raise SdkError("validation", "not-submitted")
        if self.extensions is not None:
            from ._sdk_helper import _encode
            try:
                if not isinstance(self.extensions, Mapping):
                    raise ValueError()
                value, _ = _encode(self.extensions, 63 * 1024)
            except (ValueError, TypeError, UnicodeError, RecursionError):
                raise SdkError("validation", "not-submitted") from None
            object.__setattr__(self, "extensions", freeze(value))


@dataclass(frozen=True, repr=False)
class OperationSnapshot:
    key: OperationKey
    state: OperationState
    revision: int
    record_id: str | None
    cancel_requested: bool | None
    expires_at: int | None
    problem: Json


@dataclass(frozen=True, repr=False)
class Completion(Generic[T]):
    snapshot: OperationSnapshot
    value: T


@dataclass(frozen=True, repr=False)
class DecisionReceipt:
    decision: OperationKey
    target: OperationId
    observed_state: OperationState
    disposition: str
    evidence_id: str | None


@dataclass(frozen=True, repr=False)
class MessageAcceptance:
    message_id: MessageId
    operation: OperationKey
    boundary: Literal["local-mutation"] = "local-mutation"


@dataclass(frozen=True, repr=False)
class ParticipantSnapshot:
    id: ParticipantId
    conversation_id: ConversationId
    identity_id: IdentityId
    kind: IdentityKind
    role: str
    membership_epoch: int
    active: bool
    removal_reason: str | None


@dataclass(frozen=True, repr=False)
class ConversationSnapshot:
    id: ConversationId
    revision: ConversationRevision
    participants: Sequence[ParticipantSnapshot]
    closed: bool


@dataclass(frozen=True)
class CapabilitySnapshot:
    profile: Literal["connect/2"]
    runtime_role: Literal["python"]
    selected_transport: Literal["quic"]
    fallback_reason: str | None
    features: frozenset[str]


@dataclass(frozen=True, repr=False)
class DeliveryObservation:
    participant_id: ParticipantId
    membership_epoch: int
    channel: str
    delivery_id: str
    state: DeliveryState
    provenance: DeliveryProvenance
    observed_at: int


@dataclass(frozen=True, repr=False)
class CanonicalProblem:
    code: int
    category: str
    reason: str


def freeze(value):
    if type(value) is dict:
        return MappingProxyType({key: freeze(child) for key, child in value.items()})
    if type(value) is list:
        return tuple(freeze(child) for child in value)
    return value


class SdkError(Exception):
    """Stable kind/certainty and original intent; repr/str omit private fields."""
    def __init__(self, kind: ErrorKind, certainty: Certainty, operation: OperationKey | None = None,
                 problem: CanonicalProblem | None = None, observation: Json = None):
        super().__init__(f"Connect SDK {kind} ({certainty})")
        self._kind, self._certainty, self._operation = kind, certainty, operation
        self._problem, self._observation = problem, freeze(observation)

    @property
    def kind(self) -> ErrorKind:
        return self._kind

    @property
    def certainty(self) -> Certainty:
        return self._certainty

    @property
    def operation(self) -> OperationKey | None:
        return self._operation

    @property
    def problem(self) -> CanonicalProblem | None:
        return self._problem

    @property
    def observation(self) -> Json:
        return self._observation


class SdkCancelledError(asyncio.CancelledError):
    """Normal asyncio cancellation carrying original logical intent/certainty."""
    def __init__(self, detail: SdkError):
        super().__init__(str(detail))
        self._detail = detail

    @property
    def kind(self) -> ErrorKind:
        return self._detail.kind

    @property
    def certainty(self) -> Certainty:
        return self._detail.certainty

    @property
    def operation(self) -> OperationKey | None:
        return self._detail.operation
