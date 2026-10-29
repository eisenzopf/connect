"""Typed transition context/results; metadata never acquires native custody."""
from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeAlias, TypeVar, cast

from .client import Client, Operation, OperationKind, _bad, _id, _invalid, _mutation, _natural, _object, _revision
from .sdk_connections import Connection
from .sdk_sessions import Session, _nonempty, session_handle
from .sdk_types import (
    CandidateTarget, ConnectionId, ConsultationResult, ConsultInput, ConversationRevision,
    ConversationRole, EndpointTarget, HandoffInput, HandoffResult, HandoffRoleChange,
    Json, MoveBinding, MoveInput, MoveResult, MutationOptions, OperationKey, Scope,
    SessionRole, TopologyChange, TopologyRevision, identifier,
)

ConsultationOperation: TypeAlias = Operation[ConsultationResult]
T = TypeVar("T")
TRANSITION_COMMANDS = frozenset(("session.consult", "session.consult.return", "session.handoff", "connection.replace"))


def _role(value: Any) -> bool:
    if type(value) is not dict:
        return False
    if value.get("scope") == "session":
        return set(value) == {"scope"}
    return (set(value) == {"scope", "expected_conversation_revision"} and value["scope"] == "conversation"
            and _natural(value["expected_conversation_revision"]) and value["expected_conversation_revision"] < 2**64-1)


def valid_context(value: Any, command: str, via: Any) -> bool:
    if (not identifier(via) or type(value) is not dict or not identifier(value.get("retained_connection_id"))
            or value["retained_connection_id"] == via):
        return False
    if command == "session.consult":
        return (set(value) >= {"kind","retained_connection_id"} and set(value) <= {"kind","retained_connection_id","record_id"}
                and value["kind"] == "consultation" and (value.get("record_id") is None or identifier(value["record_id"])))
    if command == "session.consult.return":
        return (_object(value,("kind","retained_connection_id","consultation_record_id"))
                and value["kind"] == "consultation-return" and identifier(value["consultation_record_id"]))
    if command == "connection.replace":
        target: Any = value.get("target")
        return (_object(value,("kind","retained_connection_id","target")) and value["kind"] == "replacement"
                and _object(target,("kind","id")) and identifier(target["id"])
                and (target["kind"] == "endpoint" or target["kind"] == "candidate" and target["id"] not in (via,value["retained_connection_id"])))
    if command == "session.handoff":
        fields={"kind","retained_connection_id","source_participant","target_participant","role","outgoing"}
        return (fields <= set(value) <= fields | {"candidate"} and value["kind"] == "handoff"
                and identifier(value["source_participant"]) and identifier(value["target_participant"])
                and value["source_participant"] != value["target_participant"] and _role(value["role"])
                and type(value["outgoing"]) is str and value["outgoing"] in ("leave","observe")
                and (value.get("candidate") is None or identifier(value["candidate"]) and value["candidate"] not in (via,value["retained_connection_id"])))
    return False


def _context(saved: Json) -> Any:
    if not isinstance(saved,Mapping) or not isinstance(saved.get("continuation"),Mapping):
        return _bad()
    return saved["continuation"]


def _topology(value: Any, client: Client, key: OperationKey, *, consultation: bool = False) -> TopologyChange:
    if (type(value["retained"]) is not list or not value["retained"] or type(value["admitted"]) is not list or not value["admitted"]
            or not consultation and len(value["admitted"]) != 1 or type(value["retired"]) is not list
            or sum(len(value[k]) for k in ("retained","admitted","retired")) > 100 or not _natural(value["topologyRevision"],1)
            or value["conversationRevision"] is not None and not _natural(value["conversationRevision"],1)):
        return _bad()
    seen=set()
    def id(value):
        if not identifier(value) or value in seen:
            return _bad()
        seen.add(value)
        return value
    def connection(value):
        if (not _object(value,("id","conversationId","sessionId","receiptStreamRevision"))
                or value["conversationId"] != key.scope.conversation_id or value["sessionId"] != key.scope.session_id
                or value["receiptStreamRevision"] is not None):
            return _bad()
        return client.conversations.by_id(value["conversationId"]).sessions.by_id(value["sessionId"]).connections.by_id(id(value["id"]))
    return TopologyChange(tuple(connection(v) for v in value["retained"]),tuple(connection(v) for v in value["admitted"]),tuple(id(v) for v in value["retired"]),TopologyRevision(value["topologyRevision"]),None if value["conversationRevision"] is None else ConversationRevision(value["conversationRevision"]))


def consultation_result(value: Any, client: Client, key: OperationKey, saved: Json) -> ConsultationResult:
    if not _object(value,("session","retained","admitted","retired","topologyRevision","conversationRevision")):
        return _bad()
    t=_topology(value,client,key,consultation=True)
    context=_context(saved)
    if t.retired or t.conversation_revision is not None or not any(v.id == context["retained_connection_id"] for v in t.retained):
        return _bad()
    return ConsultationResult(t.retained,t.admitted,t.retired,t.topology_revision,t.conversation_revision,session_handle(value["session"],client,key))


def handoff_result(value: Any, client: Client, key: OperationKey, saved: Json) -> HandoffResult:
    if not _object(value,("retained","admitted","retired","topologyRevision","conversationRevision","roleChange")):
        return _bad()
    t=_topology(value,client,key);context=_context(saved);role=context["role"]
    if (not any(v.id == context["retained_connection_id"] for v in t.retained)
            or context.get("candidate") is not None and t.admitted[0].id != context["candidate"]
            or (context["outgoing"] == "leave") != (key.scope.connection_id in t.retired)
            or (t.conversation_revision is not None if role["scope"] == "session" else t.conversation_revision != role["expected_conversation_revision"]+1)):
        return _bad()
    r=value["roleChange"];change=None
    if r is not None:
        if (not _object(r,("participantRevision","roleScope","outgoing","sourceParticipant","targetParticipant","cleanup"))
                or not _natural(r["participantRevision"],1) or r["roleScope"] != role["scope"] or r["outgoing"] != context["outgoing"]
                or r["sourceParticipant"] != context["source_participant"] or r["targetParticipant"] != context["target_participant"]
                or r["cleanup"] != ("pending" if context["outgoing"] == "leave" else "not_requested")):
            return _bad()
        change=HandoffRoleChange(r["participantRevision"],r["roleScope"],r["outgoing"],r["sourceParticipant"],r["targetParticipant"],r["cleanup"])
    return HandoffResult(t.retained,t.admitted,t.retired,t.topology_revision,t.conversation_revision,change)


def move_result(value: Any, client: Client, key: OperationKey, saved: Json) -> MoveResult:
    if not _object(value,("retained","admitted","retired","topologyRevision","conversationRevision","connection","binding")):
        return _bad()
    t=_topology(value,client,key);context=_context(saved);target=context["target"];v=value["connection"];b=value["binding"]
    if (t.conversation_revision is not None or not any(v.id == context["retained_connection_id"] for v in t.retained)
            or key.scope.connection_id not in t.retired or not _object(v,("id","conversationId","sessionId","receiptStreamRevision"))
            or (v["id"],v["conversationId"],v["sessionId"],v["receiptStreamRevision"]) != (t.admitted[0].id,key.scope.conversation_id,key.scope.session_id,None)
            or not _object(b,("generation","participantId","membershipEpoch","endpointId","deviceId"))
            or not _natural(b["generation"],1) or not _natural(b["membershipEpoch"],1)
            or any(not identifier(b[k]) for k in ("participantId","endpointId","deviceId"))
            or (b["endpointId"] != target["id"] if target["kind"] == "endpoint" else v["id"] != target["id"])):
        return _bad()
    binding=MoveBinding(b["generation"],b["participantId"],b["membershipEpoch"],b["endpointId"],b["deviceId"])
    return MoveResult(t.retained,t.admitted,t.retired,t.topology_revision,t.conversation_revision,t.admitted[0],binding)


async def consult(session: Session, input: ConsultInput, options: MutationOptions) -> ConsultationOperation:
    if (type(input) is not ConsultInput or not identifier(input.via) or not identifier(input.retain) or input.via == input.retain
            or type(input.hold) is not str or input.hold not in ("silence","configured-media")):
        return _invalid()
    action={"type":"consult","via":input.via,"retain":input.retain,"with_participants":_nonempty(input.with_participants,identifier),
            "configured_media":input.hold == "configured-media","expected_revision":_revision(input.expected_topology_revision),"mutation":_mutation(options)}
    key=OperationKey(options.operation_id,"session.consult",Scope(session.conversation_id,session.id,input.via))
    def decode(value):
        operation=session._client._from_reply(value,session._client.operation_kinds.session_consult,key)
        if _context(operation.reference._saved)["retained_connection_id"] != input.retain:
            return _bad()
        return operation
    return await session._client._helper.request({"type":"session","conversation_id":session.conversation_id,"session_id":session.id,"action":action},options.request,decode,key,True)


async def return_to_retained(operation: ConsultationOperation, expected: TopologyRevision, options: MutationOptions) -> Operation[Session]:
    mutation=_mutation(options)
    if operation.reference.command != "session.consult" or options.operation_id == operation.reference.operation_id:
        return _invalid()
    original=_context(operation.reference._saved);client=operation._client
    key=OperationKey(options.operation_id,"session.consult.return",operation.reference.scope)
    def decode(value):
        result=client._from_reply(value,client.operation_kinds.session_consult_return,key);after=_context(result.reference._saved)
        if after["retained_connection_id"] != original["retained_connection_id"] or original.get("record_id") is not None and after["consultation_record_id"] != original["record_id"]:
            return _bad()
        return result
    return await client._helper.request({"type":"consultation_return","reference":operation.reference._saved,"expected_revision":_revision(expected),"mutation":mutation},options.request,decode,key,True)


async def _movement(client: Client, action_type: str, key: OperationKey, input: dict, context: dict, kind: OperationKind[T], options: MutationOptions) -> Operation[T]:
    input={**input,"mutation":_mutation(options)}
    def decode(value):
        operation=client._from_reply(value,kind,key)
        if _context(operation.reference._saved) != context:
            return _bad()
        return operation
    action={"type":action_type,"conversation_id":key.scope.conversation_id,"session_id":key.scope.session_id,"input":input}
    if action_type == "connection_move":action["connection_id"]=key.scope.connection_id
    return await client._helper.request(action,options.request,decode,key,True)


async def handoff(session: Session, input: HandoffInput, options: MutationOptions) -> Operation[HandoffResult]:
    _mutation(options)
    if (type(input) is not HandoffInput or not identifier(input.via) or not identifier(input.retain) or input.via == input.retain
            or not identifier(input.from_participant) or not identifier(input.to) or input.from_participant == input.to
            or type(input.outgoing) is not str or input.outgoing not in ("leave","observe")
            or input.candidate is not None and (not identifier(input.candidate) or input.candidate in (input.via,input.retain))):
        return _invalid()
    if type(input.role) is SessionRole and input.role.scope == "session":role={"scope":"session"}
    elif type(input.role) is ConversationRole and input.role.scope == "conversation" and _natural(input.role.expected_conversation_revision) and input.role.expected_conversation_revision < 2**64-1:
        role={"scope":"conversation","expected_conversation_revision":input.role.expected_conversation_revision}
    else:return _invalid()
    captured={"via":input.via,"retain":input.retain,"from_participant":input.from_participant,"to":input.to,"outgoing":input.outgoing,
              "role":role,"candidate":input.candidate,"expected_revision":_revision(input.expected_topology_revision),"consultation":None}
    if input.consultation is not None:
        if not isinstance(input.consultation,Operation):return _invalid()
        ref=input.consultation.reference
        if (ref._kind is not session._client.operation_kinds.session_consult or ref.scope.conversation_id != session.conversation_id
                or ref.scope.session_id != session.id or ref.operation_id == options.operation_id
                or _context(ref._saved)["retained_connection_id"] != input.retain
                or options.extensions is not None and "consultation_record_id" in options.extensions):
            return _invalid()
        captured["consultation"]=ref._saved
    context={"kind":"handoff","retained_connection_id":input.retain,"source_participant":input.from_participant,"target_participant":input.to,
             "candidate":input.candidate,"role":role,"outgoing":input.outgoing}
    key=OperationKey(options.operation_id,"session.handoff",Scope(session.conversation_id,session.id,input.via))
    return await _movement(session._client,"session_handoff",key,captured,context,session._client.operation_kinds.session_handoff,options)


async def replace_device(connection: Connection, input: MoveInput, options: MutationOptions) -> Operation[MoveResult]:
    _mutation(options)
    if type(input) is not MoveInput or not identifier(input.retain) or input.retain == connection.id:return _invalid()
    if type(input.target) is EndpointTarget:target={"kind":"endpoint","id":_id(input.target.endpoint)}
    elif type(input.target) is CandidateTarget and input.target.candidate not in (connection.id,input.retain):target={"kind":"candidate","id":_id(input.target.candidate)}
    else:return _invalid()
    captured={"retain":input.retain,"target":target,"expected_revision":_revision(input.expected_topology_revision)}
    context={"kind":"replacement","retained_connection_id":input.retain,"target":target}
    key=OperationKey(options.operation_id,"connection.replace",Scope(connection.conversation_id,connection.session_id,connection.id))
    return await _movement(connection._client,"connection_move",key,captured,context,connection._client.operation_kinds.connection_replace,options)
