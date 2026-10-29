"""Typed asynchronous public SDK, delegating current authority to one Rust owner.

Construction/recovery is local metadata only. Mutation await returns an Operation;
completion and readiness waits are explicit. No implicit reconnect or replay.
"""
from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
import os
import re
from typing import TYPE_CHECKING, Any, Callable, Generic, Literal, TypeVar, cast

if TYPE_CHECKING:
    from .sdk_sessions import Session, SessionDecision, Sessions
    from .sdk_connections import Connection
    from .sdk_events import EventStream
    from .sdk_incoming import InvitationStream
    from .sdk_raw import RawCanonical

from ._sdk_helper import _SdkHelper, _encode
from .sdk_types import (
    CapabilitySnapshot, Completion, ConversationId, ConversationRevision,
    ConversationSnapshot, DecisionReceipt, DeliveryObservation, DeliveryProvenance, DeliveryState, IdentityId,
    IdentityKind, Json, MessageAcceptance, MessageId, MutationOptions, NonEmpty,
    OperationId, OperationKey, OperationSnapshot, OperationState, ParticipantId,
    ParticipantSnapshot, RequestOptions, Scope, SdkError, freeze, identifier,
    ConnectionUpdateReceipt, StreamSelectionReceipt, ConsultationResult, HandoffResult, MoveResult, TopologyRevision, Cursor,
)

T = TypeVar("T")
R = TypeVar("R")
_PROOF = object()
_STATES = frozenset(("accepted", "preparing", "ready", "committing", "completed", "cancelled", "failed", "interrupted", "unknown"))


def _invalid() -> Any:
    raise SdkError("validation", "not-submitted")


def _bad() -> Any:
    raise ValueError()


def _object(value: Any, fields: Sequence[str]) -> bool:
    return type(value) is dict and set(value) == set(fields)


def _natural(value: Any, minimum: int = 0) -> bool:
    return type(value) is int and minimum <= value < 2**64


def _id(value: T) -> T:
    if not identifier(value):
        _invalid()
    return value


def _revision(value: int) -> int:
    if not _natural(value):
        _invalid()
    return value


def _key(value: Any) -> OperationKey:
    if not _object(value, ("operationId", "command", "scope")) or not _object(value["scope"], ("conversationId", "sessionId", "connectionId")):
        return _bad()
    scope = value["scope"]
    try:
        return OperationKey(value["operationId"], value["command"], Scope(scope["conversationId"], scope["sessionId"], scope["connectionId"]))
    except SdkError:
        return _bad()


def _snapshot(value: Any, key: OperationKey) -> OperationSnapshot:
    if (not _object(value, ("key", "state", "revision", "recordId", "cancelRequested", "expiresAt", "problem"))
            or _key(value["key"]) != key or type(value["state"]) is not str or value["state"] not in _STATES
            or not _natural(value["revision"])
            or value["recordId"] is not None and not identifier(value["recordId"])
            or value["cancelRequested"] is not None and type(value["cancelRequested"]) is not bool
            or value["expiresAt"] is not None and (type(value["expiresAt"]) is not int or not -(2**63) <= value["expiresAt"] < 2**63)):
        return _bad()
    return OperationSnapshot(key, cast(OperationState, value["state"]), value["revision"], value["recordId"], value["cancelRequested"], value["expiresAt"], freeze(value["problem"]))


def _mutation(options: MutationOptions) -> dict:
    if type(options) is not MutationOptions:
        return _invalid()
    return {"operation_id": options.operation_id, "extensions": options.extensions}


@dataclass(frozen=True, eq=False, repr=False)
class OperationKind(Generic[T]):
    command: str
    _name: str
    _decode: Callable[[Any, Client, OperationKey, Json], T]
    _proof: object = field(repr=False)

    def __post_init__(self):
        if self._proof is not _PROOF:
            _invalid()

    def _result_type(self, value: T) -> T:
        return value


@dataclass(frozen=True, eq=False, repr=False)
class OperationReference(Generic[T]):
    """A typed SDK-created reference, never a replay grant or native resource."""
    _key: OperationKey
    _kind: OperationKind[T]
    _saved: Json
    _proof: object = field(repr=False)
    _decision_target: OperationId | None = None

    def __post_init__(self):
        if self._proof is not _PROOF:
            _invalid()

    @property
    def operation_id(self) -> OperationId:
        return self._key.operation_id

    @property
    def command(self) -> str:
        return self._key.command

    @property
    def scope(self) -> Scope:
        return self._key.scope


@dataclass(frozen=True, repr=False)
class Operation(Generic[T]):
    _client: Client
    reference: OperationReference[T]
    _record: str | None = field(default=None, init=False)

    def __post_init__(self):
        if self.reference.command == "session.consult":
            context = cast(Any,self.reference._saved)["continuation"]
            object.__setattr__(self, "_record", context.get("record_id"))

    def reference_json(self) -> Json:
        value, _ = _encode(self.reference._saved, 4096)
        return value

    def _observe(self, value: Any) -> OperationSnapshot:
        snapshot = _snapshot(value, self.reference._key)
        if snapshot.record_id is not None:
            if self._record is not None and snapshot.record_id != self._record:
                return _bad()
            object.__setattr__(self, "_record", snapshot.record_id)
            if self.reference.command == "session.consult":
                saved = cast(Any,self.reference._saved)
                context = {**saved["continuation"], "record_id":snapshot.record_id}
                object.__setattr__(self.reference, "_saved", freeze({**saved, "continuation":context}))
        return snapshot

    async def _run(self, action: dict, options: RequestOptions, decode: Callable[[Any], R],
                   key: OperationKey | None = None, mutation: bool = False) -> R:
        return await self._client._helper.request(
            {"type":"operation", "reference":self.reference._saved, "kind":self.reference._kind._name, "action":action},
            options, decode, key or self.reference._key, mutation)

    async def status(self, options: RequestOptions) -> OperationSnapshot:
        return await self._run({"type":"status"}, options, self._observe)

    async def wait_until_ready(self, options: RequestOptions) -> OperationSnapshot:
        def decode(value):
            observed = self._observe(value)
            if observed.state not in ("ready", "committing", "completed"):
                return _bad()
            return observed
        return await self._run({"type":"wait_until_ready"}, options, decode)

    async def wait(self, options: RequestOptions) -> Completion[T]:
        def decode(value):
            if not _object(value, ("snapshot", "value")):
                return _bad()
            snapshot = self._observe(value["snapshot"])
            if snapshot.state != "completed":
                return _bad()
            result = self.reference._kind._decode(value["value"], self._client, self.reference._key, self.reference._saved)
            if isinstance(result, DecisionReceipt) and self.reference._decision_target is not None and result.target != self.reference._decision_target:
                return _bad()
            return Completion(snapshot, result)
        return await self._run({"type":"wait"}, options, decode)

    async def return_to_retained(self: Operation[ConsultationResult], expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]:
        from .sdk_movements import return_to_retained
        return await return_to_retained(self, expected_topology_revision, options)

    async def _decide(self, action: dict, options: MutationOptions, kind: OperationKind[DecisionReceipt]) -> Operation[DecisionReceipt]:
        action = {**action, "mutation":_mutation(options)}
        scope = self.reference.scope
        key = OperationKey(options.operation_id, kind.command, Scope(scope.conversation_id, scope.session_id))
        if key.operation_id == self.reference.operation_id:
            return _invalid()
        def decode(value):
            operation = self._client._from_reply(value, kind, key)
            object.__setattr__(operation.reference, "_decision_target", self.reference.operation_id)
            return operation
        return await self._run(action, options.request, decode, key, True)

    async def cancel(self, options: MutationOptions) -> Operation[DecisionReceipt]:
        return await self._decide({"type":"cancel"}, options, self._client.operation_kinds.operation_cancel)

    async def reconcile(self, outcome: Literal["completed", "cancelled", "failed"], evidence_id: str, options: MutationOptions) -> Operation[DecisionReceipt]:
        if type(outcome) is not str or outcome not in ("completed", "cancelled", "failed") or not identifier(evidence_id):
            return _invalid()
        return await self._decide({"type":"reconcile", "outcome":outcome, "evidence_id":evidence_id}, options, self._client.operation_kinds.operation_reconcile)


def _decision(value: Any, client: Client, key: OperationKey, saved: Json) -> DecisionReceipt:
    if (not _object(value, ("decision", "target", "observedState", "disposition", "evidenceId"))
            or _key(value["decision"]) != key or not identifier(value["target"]) or value["target"] == key.operation_id
            or type(value["observedState"]) is not str or value["observedState"] not in _STATES
            or type(value["disposition"]) is not str or not re.fullmatch(r"[^\x00-\x1f\x7f]{1,256}", value["disposition"])
            or (value["evidenceId"] is not None if key.command == "operation.cancel" else not identifier(value["evidenceId"]))):
        return _bad()
    return DecisionReceipt(key, value["target"], cast(OperationState, value["observedState"]), value["disposition"], value["evidenceId"])


def _message(value: Any, client: Client, key: OperationKey, saved: Json) -> MessageAcceptance:
    if not _object(value, ("messageId", "operation", "boundary")) or not identifier(value["messageId"]) or _key(value["operation"]) != key or value["boundary"] != "local-mutation":
        return _bad()
    return MessageAcceptance(value["messageId"], key)


def _conversation(value: Any, client: Client, key: OperationKey, saved: Json) -> Conversation:
    if not _object(value, ("id",)) or not identifier(value["id"]):
        return _bad()
    return client.conversations.by_id(value["id"])


def _participant(value: Any, client: Client, key: OperationKey, saved: Json) -> Participant:
    if not _object(value, ("id", "conversationId")) or not identifier(value["id"]) or value["conversationId"] != key.scope.conversation_id:
        return _bad()
    return client.conversations.by_id(value["conversationId"]).participants.by_id(value["id"])


def _session(value: Any, client: Client, key: OperationKey, saved: Json) -> Session:
    from .sdk_sessions import session_handle
    return session_handle(value, client, key)


def _session_decision(value: Any, client: Client, key: OperationKey, saved: Json) -> SessionDecision:
    from .sdk_sessions import session_decision
    return session_decision(value, client, key)


def _connection(value: Any, client: Client, key: OperationKey, saved: Json) -> Connection:
    from .sdk_connections import connection_handle
    return connection_handle(value, client, key)


def _connection_update(value: Any, client: Client, key: OperationKey, saved: Json) -> ConnectionUpdateReceipt:
    from .sdk_connections import connection_update
    return connection_update(value, client, key)


def _stream_selection(value: Any, client: Client, key: OperationKey, saved: Json) -> StreamSelectionReceipt:
    from .sdk_connections import stream_selection
    return stream_selection(value, client, key)


def _consultation(value: Any, client: Client, key: OperationKey, saved: Json) -> ConsultationResult:
    from .sdk_movements import consultation_result
    return consultation_result(value, client, key, saved)


def _handoff(value: Any, client: Client, key: OperationKey, saved: Json) -> HandoffResult:
    from .sdk_movements import handoff_result
    return handoff_result(value, client, key, saved)


def _move(value: Any, client: Client, key: OperationKey, saved: Json) -> MoveResult:
    from .sdk_movements import move_result
    return move_result(value, client, key, saved)


@dataclass(frozen=True)
class OperationKinds:
    conversation_create: OperationKind[Conversation] = field(default_factory=lambda:OperationKind("conversation.create", "conversation_create", _conversation, _PROOF))
    message_send: OperationKind[MessageAcceptance] = field(default_factory=lambda:OperationKind("message.send", "message_send", _message, _PROOF))
    conversation_participant_add: OperationKind[Participant] = field(default_factory=lambda:OperationKind("conversation.participant.add", "conversation_participant_add", _participant, _PROOF))
    conversation_participant_update: OperationKind[Participant] = field(default_factory=lambda:OperationKind("conversation.participant.update", "conversation_participant_update", _participant, _PROOF))
    conversation_participant_remove: OperationKind[Participant] = field(default_factory=lambda:OperationKind("conversation.participant.remove", "conversation_participant_remove", _participant, _PROOF))
    operation_cancel: OperationKind[DecisionReceipt] = field(default_factory=lambda:OperationKind("operation.cancel", "operation_cancel", _decision, _PROOF))
    operation_reconcile: OperationKind[DecisionReceipt] = field(default_factory=lambda:OperationKind("operation.reconcile", "operation_reconcile", _decision, _PROOF))
    session_invite: OperationKind[Session] = field(default_factory=lambda:OperationKind("session.invite", "session_invite", _session, _PROOF))
    session_accept: OperationKind[Session] = field(default_factory=lambda:OperationKind("session.accept", "session_accept", _session, _PROOF))
    session_update: OperationKind[Session] = field(default_factory=lambda:OperationKind("session.update", "session_update", _session, _PROOF))
    session_end: OperationKind[Session] = field(default_factory=lambda:OperationKind("session.end", "session_end", _session, _PROOF))
    session_reject: OperationKind[SessionDecision] = field(default_factory=lambda:OperationKind("session.reject", "session_reject", _session_decision, _PROOF))
    session_cancel: OperationKind[SessionDecision] = field(default_factory=lambda:OperationKind("session.cancel", "session_cancel", _session_decision, _PROOF))
    connection_offer: OperationKind[Connection] = field(default_factory=lambda:OperationKind("connection.offer", "connection_offer", _connection, _PROOF))
    connection_update: OperationKind[ConnectionUpdateReceipt] = field(default_factory=lambda:OperationKind("connection.update", "connection_update", _connection_update, _PROOF))
    connection_end: OperationKind[Connection] = field(default_factory=lambda:OperationKind("connection.end", "connection_end", _connection, _PROOF))
    stream_subscribe: OperationKind[StreamSelectionReceipt] = field(default_factory=lambda:OperationKind("stream.subscribe", "stream_subscribe", _stream_selection, _PROOF))
    stream_unsubscribe: OperationKind[StreamSelectionReceipt] = field(default_factory=lambda:OperationKind("stream.unsubscribe", "stream_unsubscribe", _stream_selection, _PROOF))

    session_consult: OperationKind[ConsultationResult] = field(default_factory=lambda:OperationKind("session.consult", "session_consult", _consultation, _PROOF))
    session_consult_return: OperationKind[Session] = field(default_factory=lambda:OperationKind("session.consult.return", "session_consult_return", _session, _PROOF))
    session_handoff: OperationKind[HandoffResult] = field(default_factory=lambda:OperationKind("session.handoff", "session_handoff", _handoff, _PROOF))
    connection_replace: OperationKind[MoveResult] = field(default_factory=lambda:OperationKind("connection.replace", "connection_replace", _move, _PROOF))


_KINDS = OperationKinds()


@dataclass(frozen=True, repr=False)
class Operations:
    _client: Client

    def restore(self, saved: Json, expected: OperationKind[T]) -> Operation[T]:
        if not any(expected is known for known in vars(_KINDS).values()):
            return _invalid()
        try:
            value, _ = _encode(saved, 4096)
            transition = expected.command in ("session.consult","session.consult.return","session.handoff","connection.replace")
            fields = ("schema", "operation_id", "command", "scope") + (("continuation",) if transition else ())
            if (not _object(value, fields)
                    or value["schema"] != "connect-operation-reference/1" or not identifier(value["operation_id"])
                    or value["command"] != expected.command or type(value["scope"]) is not dict
                    or not set(value["scope"]) <= {"conversation_id", "session_id", "connection_id"}
                    or any(not identifier(v) for v in value["scope"].values())):
                return _invalid()
            scope = Scope(**value["scope"])
            if expected.command == "conversation.create" and value["scope"]:
                return _invalid()
            if expected.command == "message.send" or expected.command.startswith("conversation.participant."):
                if scope.conversation_id is None or scope.session_id is not None:
                    return _invalid()
            if expected.command.startswith("operation.") and scope.connection_id is not None:
                return _invalid()
            if expected.command.startswith("session.") and not transition and (scope.conversation_id is None or scope.session_id is None or scope.connection_id is not None):
                return _invalid()
            if expected.command.startswith(("connection.","stream.")) and scope.connection_id is None:
                return _invalid()
            if transition:
                from .sdk_movements import valid_context
                if scope.connection_id is None or not valid_context(value["continuation"], expected.command, scope.connection_id):
                    return _invalid()
            key = OperationKey(value["operation_id"], value["command"], scope)
            return Operation(self._client, OperationReference(key, expected, freeze(value), _PROOF))
        except (ValueError, TypeError, UnicodeError, RecursionError):
            return _invalid()

    def recover(self, reference: OperationReference[T]) -> Operation[T]:
        if type(reference) is not OperationReference or reference._proof is not _PROOF:
            return _invalid()
        operation = self.restore(reference._saved, reference._kind)
        object.__setattr__(operation.reference, "_decision_target", reference._decision_target)
        return operation


def _participant_snapshot(value: Any, cid: ConversationId, pid: ParticipantId) -> ParticipantSnapshot:
    if (not _object(value, ("id", "conversationId", "identityId", "kind", "role", "membershipEpoch", "active", "removalReason"))
            or value["id"] != pid or value["conversationId"] != cid or not identifier(value["identityId"])
            or type(value["kind"]) is not str or value["kind"] not in ("human", "ai", "system", "external")
            or type(value["role"]) is not str or not _natural(value["membershipEpoch"])
            or type(value["active"]) is not bool or value["removalReason"] is not None and type(value["removalReason"]) is not str):
        return _bad()
    return ParticipantSnapshot(pid, cid, value["identityId"], cast(IdentityKind, value["kind"]), value["role"], value["membershipEpoch"], value["active"], value["removalReason"])


@dataclass(frozen=True, repr=False)
class Participant:
    _client: Client
    conversation_id: ConversationId
    id: ParticipantId

    def __post_init__(self):
        _id(self.conversation_id)
        _id(self.id)

    async def refresh(self, options: RequestOptions) -> ParticipantSnapshot:
        return await self._client._helper.request({"type":"participant_refresh", "conversation_id":self.conversation_id, "participant_id":self.id}, options, lambda value:_participant_snapshot(value, self.conversation_id, self.id))

    async def update(self, role: str, expected_conversation_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]:
        if type(role) is not str:
            return _invalid()
        return await self._client._submit({"type":"participant_update", "conversation_id":self.conversation_id, "participant_id":self.id, "role":role, "expected_revision":_revision(expected_conversation_revision)}, self._client.operation_kinds.conversation_participant_update, Scope(self.conversation_id), options)

    async def remove(self, expected_conversation_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]:
        return await self._client._submit({"type":"participant_remove", "conversation_id":self.conversation_id, "participant_id":self.id, "expected_revision":_revision(expected_conversation_revision)}, self._client.operation_kinds.conversation_participant_remove, Scope(self.conversation_id), options)


@dataclass(frozen=True, repr=False)
class Participants:
    _client: Client
    _cid: ConversationId

    def by_id(self, id: ParticipantId) -> Participant:
        return Participant(self._client, self._cid, _id(id))

    async def add(self, identity_id: IdentityId, kind: IdentityKind, role: str, expected_revision: ConversationRevision, options: MutationOptions) -> Operation[Participant]:
        if type(kind) is not str or kind not in ("human", "ai", "system", "external") or type(role) is not str:
            return _invalid()
        return await self._client._submit({"type":"participant_add", "conversation_id":self._cid, "identity_id":_id(identity_id), "kind":kind, "role":role, "expected_revision":_revision(expected_revision)}, self._client.operation_kinds.conversation_participant_add, Scope(self._cid), options)


def _receipts(value: Any, cid: ConversationId, mid: MessageId) -> tuple[DeliveryObservation, ...]:
    if not _object(value, ("conversationId", "messageId", "observations")) or value["conversationId"] != cid or value["messageId"] != mid or type(value["observations"]) is not list or len(value["observations"]) > 4096:
        return _bad()
    result = []
    seen = set()
    for item in value["observations"]:
        if (not _object(item, ("participantId", "membershipEpoch", "channel", "deliveryId", "state", "provenance", "observedAt"))
                or not identifier(item["participantId"]) or not _natural(item["membershipEpoch"], 1)
                or type(item["channel"]) is not str or not re.fullmatch(r"[\x21-\x7e]{1,64}", item["channel"])
                or type(item["deliveryId"]) is not str or not re.fullmatch(r"[\x21-\x7e]{1,512}", item["deliveryId"])
                or type(item["state"]) is not str or item["state"] not in ("queued", "accepted", "delivered", "read", "failed", "unknown")
                or type(item["provenance"]) is not str or item["provenance"] not in ("journal", "client_reported", "provider_observation", "local_attempt")
                or not _natural(item["observedAt"])):
            return _bad()
        key = (item["participantId"], item["membershipEpoch"], item["channel"], item["deliveryId"])
        if key in seen:
            return _bad()
        seen.add(key)
        result.append(DeliveryObservation(item["participantId"], item["membershipEpoch"], item["channel"], item["deliveryId"], cast(DeliveryState, item["state"]), cast(DeliveryProvenance, item["provenance"]), item["observedAt"]))
    return tuple(result)


@dataclass(frozen=True, repr=False)
class Conversation:
    _client: Client
    id: ConversationId

    def __post_init__(self):
        _id(self.id)

    @property
    def participants(self) -> Participants:
        return Participants(self._client, self.id)

    @property
    def sessions(self) -> Sessions:
        from .sdk_sessions import Sessions
        return Sessions(self._client, self.id)

    def events(self, after: Cursor | None, options: RequestOptions) -> EventStream:
        from .sdk_events import EventStream
        return EventStream(self._client._helper, self.id, after, options)

    async def refresh(self, options: RequestOptions) -> ConversationSnapshot:
        def decode(value):
            if (not _object(value, ("id", "revision", "closed", "participants")) or value["id"] != self.id
                    or not _natural(value["revision"]) or type(value["closed"]) is not bool
                    or type(value["participants"]) is not list or len(value["participants"]) > 4096):
                return _bad()
            participants = []
            seen = set()
            for p in value["participants"]:
                if type(p) is not dict or not identifier(p.get("id")) or p["id"] in seen:
                    return _bad()
                seen.add(p["id"])
                participants.append(_participant_snapshot(p, self.id, p["id"]))
            return ConversationSnapshot(self.id, ConversationRevision(value["revision"]), tuple(participants), value["closed"])
        return await self._client._helper.request({"type":"conversation_refresh", "conversation_id":self.id}, options, decode)

    async def send(self, message_id: MessageId, to: NonEmpty[ParticipantId], text: str, options: MutationOptions) -> Operation[MessageAcceptance]:
        if type(to) is not NonEmpty or len(to.rest) >= 65536 or type(text) is not str:
            return _invalid()
        recipients = [_id(to.first), *(_id(id) for id in to.rest)]
        if len(recipients) != len(set(recipients)):
            return _invalid()
        return await self._client._submit({"type":"conversation_send", "conversation_id":self.id, "message_id":_id(message_id), "to":recipients, "text":text}, self._client.operation_kinds.message_send, Scope(self.id), options)

    async def receipts(self, message_id: MessageId, options: RequestOptions) -> Sequence[DeliveryObservation]:
        _id(message_id)
        return await self._client._helper.request({"type":"message_receipts", "conversation_id":self.id, "message_id":message_id}, options, lambda value:_receipts(value, self.id, message_id))


@dataclass(frozen=True, repr=False)
class Conversations:
    _client: Client

    def by_id(self, id: ConversationId) -> Conversation:
        return Conversation(self._client, _id(id))

    async def create(self, participants: Sequence[IdentityId], policy: Literal["persistent", "ephemeral"], options: MutationOptions) -> Operation[Conversation]:
        if type(participants) not in (list, tuple) or len(participants) > 65536 or type(policy) is not str or policy not in ("persistent", "ephemeral"):
            return _invalid()
        return await self._client._submit({"type":"conversation_create", "participants":[_id(id) for id in participants], "policy":policy}, self._client.operation_kinds.conversation_create, Scope(), options)


@dataclass(frozen=True, repr=False)
class Capabilities:
    _client: Client

    async def refresh(self, options: RequestOptions) -> CapabilitySnapshot:
        def decode(value):
            if (not _object(value, ("profile", "runtime_role", "selected_transport", "fallback_reason", "features"))
                    or value["profile"] != "connect/2" or value["runtime_role"] != "rust" or value["selected_transport"] != "quic"
                    or value["fallback_reason"] is not None or type(value["features"]) is not list or len(value["features"]) > 128
                    or any(type(f) is not str or not re.fullmatch(r"[a-z0-9-]{1,128}", f) for f in value["features"])
                    or len(set(value["features"])) != len(value["features"])):
                return _bad()
            # Expose only interfaces actually implemented by this Python facade.
            return CapabilitySnapshot("connect/2", "python", "quic", None, frozenset(f for f in value["features"] if f in ("original-id-operations", "message-receipts", "durable-events", "incoming-invitations", "raw-canonical")))
        return await self._client._helper.request({"type":"capabilities_refresh"}, options, decode)

    async def require(self, feature: str, options: RequestOptions) -> None:
        if type(feature) is not str or not re.fullmatch(r"[a-z0-9-]{1,128}", feature):
            raise SdkError("validation", "read-only")
        if feature not in (await self.refresh(options)).features:
            raise SdkError("unsupported", "read-only")


@dataclass(frozen=True, init=False, repr=False)
class Client:
    """One asynchronous participant SDK owner; use async with for bounded close."""
    _helper: _SdkHelper
    operation_kinds: OperationKinds
    operations: Operations
    conversations: Conversations
    capabilities: Capabilities
    raw: RawCanonical

    def __init__(self, config: str | os.PathLike, executable: str | os.PathLike = "connect"):
        from .sdk_raw import RawCanonical
        object.__setattr__(self, "_helper", _SdkHelper(config, executable))
        object.__setattr__(self, "operation_kinds", _KINDS)
        object.__setattr__(self, "operations", Operations(self))
        object.__setattr__(self, "conversations", Conversations(self))
        object.__setattr__(self, "capabilities", Capabilities(self))
        object.__setattr__(self, "raw", RawCanonical(self._helper))

    def _from_reply(self, value: Any, kind: OperationKind[T], key: OperationKey) -> Operation[T]:
        try:
            result = self.operations.restore(value, kind)
            if result.reference._key != key:
                return _bad()
            return result
        except SdkError:
            return _bad()

    def incoming_invitations(self, options: RequestOptions) -> InvitationStream:
        from .sdk_incoming import InvitationStream
        return InvitationStream(self, options)

    async def _submit(self, action: dict, kind: OperationKind[T], scope: Scope, options: MutationOptions) -> Operation[T]:
        action = {**action, "mutation":_mutation(options)}
        key = OperationKey(options.operation_id, kind.command, scope)
        return await self._helper.request(action, options.request, lambda value:self._from_reply(value, kind, key), key, True)

    async def close(self, options: RequestOptions) -> None:
        await self._helper.close(options)

    async def __aenter__(self) -> Client:
        return self

    async def __aexit__(self, exc_type, exc, traceback) -> None:
        await self.close(RequestOptions(5000))
