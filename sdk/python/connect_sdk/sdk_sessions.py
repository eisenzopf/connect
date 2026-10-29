"""Typed Session control; topology and authorization remain with the Rust SDK."""
from __future__ import annotations

from collections.abc import Callable, Sequence
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any, TypeVar, cast

if TYPE_CHECKING:
    from .sdk_connections import Connections
    from .sdk_movements import ConsultationOperation

from .client import Client, Operation, OperationKind, _bad, _id, _invalid, _mutation, _natural, _object, _revision
from .sdk_types import (
    CodecRenegotiate, ConversationId, InviteMedium, Json, Medium, MediumUpgrade,
    MutationOptions, NonEmpty, OperationKey, ParticipantId, ParticipantLeave,
    RequestOptions, RoleChange, Scope, SessionId, SessionRevision, SessionSnapshot,
    SessionUpdate, TopologyRevision, freeze, identifier,
    ConsultInput, HandoffInput, HandoffResult,
)

T = TypeVar("T")
_MEDIA = frozenset(("audio", "video", "screen", "data"))
_INVITE_MEDIA = frozenset(("voice", "video", "voice+video", "screen-share", "text-chat", "mixed"))


def _nonempty(value: NonEmpty[T], check: Callable[[Any], bool]) -> list[T]:
    if type(value) is not NonEmpty or len(value.rest) >= 65536:
        return _invalid()
    result = [value.first, *value.rest]
    if any(not check(v) for v in result):
        return _invalid()
    return result


def _text(value: Any) -> bool:
    return type(value) is str and len(value) > 0


def _medium(value: Any) -> bool:
    return type(value) is str and value in _MEDIA


def _invite(to: Sequence[ParticipantId], medium: InviteMedium, intent: str, capabilities: Json) -> dict:
    if (type(to) not in (list, tuple) or len(to) > 65536 or any(not identifier(id) for id in to)
            or type(medium) is not str or medium not in _INVITE_MEDIA or type(intent) is not str):
        return _invalid()
    return {"type":"invite", "to":list(to), "medium":medium, "intent":intent, "capabilities":capabilities}


@dataclass(frozen=True, repr=False)
class SessionDecision:
    session: Session
    state: str
    topology_revision: TopologyRevision


def session_handle(value: Any, client: Client, key: OperationKey) -> Session:
    if (not _object(value, ("id", "conversationId")) or value["id"] != key.scope.session_id
            or value["conversationId"] != key.scope.conversation_id or not identifier(value["id"])):
        return _bad()
    return client.conversations.by_id(value["conversationId"]).sessions.by_id(value["id"])


def session_decision(value: Any, client: Client, key: OperationKey) -> SessionDecision:
    if (not _object(value, ("session", "state", "topologyRevision"))
            or value["state"] != ("rejected" if key.command == "session.reject" else "cancelled")
            or not _natural(value["topologyRevision"])):
        return _bad()
    return SessionDecision(session_handle(value["session"], client, key), value["state"], TopologyRevision(value["topologyRevision"]))


def _snapshot(value: Any, cid: ConversationId, sid: SessionId) -> SessionSnapshot:
    if (not _object(value, ("id", "conversationId", "state", "media", "intent", "capabilitiesOffer", "codecPreferences", "isMain", "revision", "topologyRevision", "createdAt", "updatedAt", "endedReason"))
            or value["id"] != sid or value["conversationId"] != cid or type(value["state"]) is not str
            or type(value["media"]) is not list or any(not _medium(v) for v in value["media"])
            or value["intent"] is not None and type(value["intent"]) is not str
            or type(value["codecPreferences"]) is not list or any(type(v) is not str for v in value["codecPreferences"])
            or type(value["isMain"]) is not bool or any(not _natural(value[k]) for k in ("revision", "topologyRevision", "createdAt", "updatedAt"))
            or value["endedReason"] is not None and type(value["endedReason"]) is not str):
        return _bad()
    return SessionSnapshot(sid, cid, value["state"], tuple(cast(Medium,v) for v in value["media"]), value["intent"], freeze(value["capabilitiesOffer"]), tuple(value["codecPreferences"]), value["isMain"], SessionRevision(value["revision"]), TopologyRevision(value["topologyRevision"]), value["createdAt"], value["updatedAt"], value["endedReason"])


@dataclass(frozen=True, repr=False)
class Sessions:
    _client: Client
    _cid: ConversationId

    def by_id(self, id: SessionId) -> Session:
        return Session(self._client, self._cid, _id(id))

    async def invite(self, session_id: SessionId, to: Sequence[ParticipantId], medium: InviteMedium, intent: str, capabilities: Json, options: MutationOptions) -> Operation[Session]:
        return await self.by_id(session_id)._mutate(_invite(to, medium, intent, capabilities), self._client.operation_kinds.session_invite, options)


@dataclass(frozen=True, repr=False)
class Session:
    _client: Client
    conversation_id: ConversationId
    id: SessionId

    def __post_init__(self):
        _id(self.conversation_id)
        _id(self.id)

    @property
    def connections(self) -> Connections:
        from .sdk_connections import Connections
        return Connections(self._client, self.conversation_id, self.id)

    async def consult(self, input: ConsultInput, options: MutationOptions) -> ConsultationOperation:
        from .sdk_movements import consult
        return await consult(self, input, options)

    async def handoff(self, input: HandoffInput, options: MutationOptions) -> Operation[HandoffResult]:
        from .sdk_movements import handoff
        return await handoff(self, input, options)

    async def _mutate(self, action: dict, kind: OperationKind[T], options: MutationOptions) -> Operation[T]:
        action = {**action, "mutation":_mutation(options)}
        key = OperationKey(options.operation_id, kind.command, Scope(self.conversation_id, self.id))
        return await self._client._helper.request({"type":"session", "conversation_id":self.conversation_id, "session_id":self.id, "action":action}, options.request, lambda value:self._client._from_reply(value, kind, key), key, True)

    async def refresh(self, options: RequestOptions) -> SessionSnapshot:
        return await self._client._helper.request({"type":"session", "conversation_id":self.conversation_id, "session_id":self.id, "action":{"type":"refresh"}}, options, lambda value:_snapshot(value, self.conversation_id, self.id))

    async def invite_participants(self, to: NonEmpty[ParticipantId], medium: InviteMedium, intent: str, capabilities: Json, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]:
        action = _invite(_nonempty(to, identifier), medium, intent, capabilities)
        action["expected_revision"] = _revision(expected_topology_revision)
        return await self._mutate(action, self._client.operation_kinds.session_invite, options)

    async def accept(self, capabilities: Json, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]:
        return await self._mutate({"type":"accept", "capabilities":capabilities, "expected_revision":_revision(expected_topology_revision)}, self._client.operation_kinds.session_accept, options)

    async def _decide(self, name: str, reason_code: int, reason: str, expected: TopologyRevision, options: MutationOptions, kind: OperationKind[T]) -> Operation[T]:
        if type(reason_code) is not int or not 0 <= reason_code <= 65535 or type(reason) is not str:
            return _invalid()
        return await self._mutate({"type":name, "reason_code":reason_code, "reason":reason, "expected_revision":_revision(expected)}, kind, options)

    async def reject(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[SessionDecision]:
        return await self._decide("reject", reason_code, reason, expected_topology_revision, options, self._client.operation_kinds.session_reject)

    async def cancel_invitation(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[SessionDecision]:
        return await self._decide("cancel", reason_code, reason, expected_topology_revision, options, self._client.operation_kinds.session_cancel)

    async def end(self, reason_code: int, reason: str, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]:
        return await self._decide("end", reason_code, reason, expected_topology_revision, options, self._client.operation_kinds.session_end)

    async def update(self, input: SessionUpdate, expected_topology_revision: TopologyRevision, options: MutationOptions) -> Operation[Session]:
        update: dict
        if type(input) is ParticipantLeave:
            update = {"action":"participant-leave", "participant_id":_id(input.participant_id)}
        elif type(input) is RoleChange:
            if type(input.role) is not str:
                return _invalid()
            update = {"action":"role-change", "participant_id":_id(input.participant_id), "role":input.role}
        elif type(input) is MediumUpgrade:
            update = {"action":"medium-upgrade", "media":_nonempty(input.media, _medium)}
        elif type(input) is CodecRenegotiate:
            update = {"action":"codec-renegotiate", "codec_preferences":_nonempty(input.codec_preferences, _text)}
        else:
            return _invalid()
        return await self._mutate({"type":"update", "update":update, "expected_revision":_revision(expected_topology_revision)}, self._client.operation_kinds.session_update, options)
