"""Native control handles and captured observations; no Python media custody."""
from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Any, Callable, TypeVar, cast, TYPE_CHECKING

if TYPE_CHECKING:
    from .sdk_media import MediaConnection, MediaStream

from .client import Client, Operation, OperationKind, _bad, _id, _invalid, _mutation, _natural, _object, _revision
from .sdk_sessions import _medium, _nonempty, _text
from .sdk_types import (
    CodecObservation, CodecOutcome, ConnectionCodecRenegotiate, ConnectionId,
    ConnectionSnapshot, ConnectionUpdate, ConnectionUpdateOutcome, ConnectionUpdateReceipt,
    ConversationId, Direction, GateOutcome, HoldConnection, MediaFormat, Medium,
    Mute, MutationOptions, NegotiatedStream, NonEmpty, OfferedStream, OperationKey,
    ParticipantStreams, Quality, QualityOutcome, IceRestartOutcome, RequestOptions, ResumeConnection,
    Scope, SessionId, SourceClock, StreamGateObservation, StreamId, StreamResourceRevision,
    StreamRevision, StreamSelectionReceipt, StreamSelector, StreamSnapshot, StreamTarget,
    freeze, identifier,
    MoveInput, MoveResult,
)

T = TypeVar("T")


def _direction(value: Any) -> bool:
    return type(value) is str and value in ("sendonly", "recvonly", "sendrecv", "inactive")


def connection_handle(value: Any, client: Client, key: OperationKey) -> Connection:
    if (not _object(value, ("id", "conversationId", "sessionId", "receiptStreamRevision"))
            or value["id"] != key.scope.connection_id or value["conversationId"] != key.scope.conversation_id
            or value["sessionId"] != key.scope.session_id
            or value["receiptStreamRevision"] is not None and not _natural(value["receiptStreamRevision"],1)):
        return _bad()
    return Connection(client, value["conversationId"], value["sessionId"], value["id"], value["receiptStreamRevision"])


def _codec(value: Any, generation: int, gates: bool = False) -> CodecObservation:
    fields = ("parameters", "streamGeneration", "connectionGeneration") + (("state", "sourceMuted", "heldDirection") if gates else ())
    if (not _object(value, fields) or not _natural(value["streamGeneration"],1)
            or not _natural(value["connectionGeneration"],1) or value["connectionGeneration"] != generation
            or not _object(value["parameters"], ("id", "medium", "direction", "codec", "streamLocalId"))):
        return _bad()
    p = value["parameters"]
    if (not identifier(p["id"]) or not _medium(p["medium"]) or not _direction(p["direction"])
            or p["streamLocalId"] is not None and (not _natural(p["streamLocalId"],1) or p["streamLocalId"] > 65535)):
        return _bad()
    parameters = NegotiatedStream(p["id"], cast(Medium,p["medium"]), cast(Direction,p["direction"]), freeze(p["codec"]), p["streamLocalId"])
    if not gates:
        return CodecObservation(parameters, value["streamGeneration"], generation)
    if (type(value["state"]) is not str or value["state"] not in ("active", "paused") or type(value["sourceMuted"]) is not bool
            or value["heldDirection"] is not None and not _direction(value["heldDirection"])):
        return _bad()
    return StreamGateObservation(parameters, value["streamGeneration"], generation, cast(Any,value["state"]), value["sourceMuted"], value["heldDirection"])


def connection_update(value: Any, client: Client, key: OperationKey) -> ConnectionUpdateReceipt:
    if (not _object(value, ("connection", "connectionGeneration", "streamRevision", "outcome"))
            or not _natural(value["connectionGeneration"],1) or not _natural(value["streamRevision"],1)
            or type(value["outcome"]) is not dict):
        return _bad()
    connection = connection_handle(value["connection"], client, key)
    o = value["outcome"]
    outcome: ConnectionUpdateOutcome
    if o.get("kind") == "ice-restart":
        if not _object(o,("kind", "iceGeneration")) or not _natural(o["iceGeneration"],1) or o["iceGeneration"] >= 2**53:
            return _bad()
        outcome = IceRestartOutcome(o["iceGeneration"])
    elif o.get("kind") == "quality":
        if not _object(o,("kind", "intervalMs")) or not _natural(o["intervalMs"]) or o["intervalMs"] >= 2**32:
            return _bad()
        outcome = QualityOutcome(o["intervalMs"])
    elif o.get("kind") in ("codecs", "gates"):
        gates = o["kind"] == "gates"
        if not _object(o,("kind", "action" if gates else "resourcesChanged", "streams")) or type(o["streams"]) is not list:
            return _bad()
        streams = tuple(_codec(v, value["connectionGeneration"], gates) for v in o["streams"])
        if len({v.parameters.id for v in streams}) != len(streams):
            return _bad()
        if gates:
            if type(o["action"]) is not str or o["action"] not in ("hold", "resume", "mute", "unmute"):
                return _bad()
            outcome = GateOutcome(cast(Any,o["action"]), cast(tuple[StreamGateObservation,...],streams))
        else:
            if type(o["resourcesChanged"]) is not bool:
                return _bad()
            outcome = CodecOutcome(o["resourcesChanged"], streams)
    else:
        return _bad()
    return ConnectionUpdateReceipt(connection, value["connectionGeneration"], StreamRevision(value["streamRevision"]), outcome)


def stream_selection(value: Any, client: Client, key: OperationKey) -> StreamSelectionReceipt:
    if (not _object(value,("connection", "connectionGeneration", "streamRevision", "resourcesChanged", "bindings"))
            or not _natural(value["connectionGeneration"],1) or not _natural(value["streamRevision"],1)
            or type(value["resourcesChanged"]) is not bool or value["bindings"] is not None and type(value["bindings"]) is not list):
        return _bad()
    bindings = None if value["bindings"] is None else tuple(_codec(v,value["connectionGeneration"]) for v in value["bindings"])
    if bindings is not None and len({v.parameters.id for v in bindings}) != len(bindings):
        return _bad()
    return StreamSelectionReceipt(connection_handle(value["connection"],client,key), value["connectionGeneration"], StreamRevision(value["streamRevision"]), value["resourcesChanged"], bindings)


@dataclass(frozen=True, repr=False)
class Connections:
    _client: Client
    _cid: ConversationId
    _sid: SessionId

    def by_id(self, id: ConnectionId) -> Connection:
        return Connection(self._client, self._cid, self._sid, _id(id))

    async def offer(self, connection_id: ConnectionId, streams: NonEmpty[OfferedStream], options: MutationOptions) -> Operation[Connection]:
        values = []
        for s in _nonempty(streams, lambda v:type(v) is OfferedStream):
            if not _medium(s.kind) or not _direction(s.direction):
                return _invalid()
            values.append({"id":_id(s.id), "kind":s.kind, "direction":s.direction, "codecs":_nonempty(s.codecs,_text)})
        return await self.by_id(connection_id)._mutate({"type":"offer", "streams":values}, self._client.operation_kinds.connection_offer, options)


@dataclass(frozen=True, repr=False)
class Connection:
    _client: Client
    conversation_id: ConversationId
    session_id: SessionId
    id: ConnectionId
    receipt_stream_revision: StreamRevision | None = None

    @property
    def media(self) -> MediaConnection:
        return self._client.media.connection(self.conversation_id,self.session_id,self.id)

    def __post_init__(self):
        _id(self.conversation_id);_id(self.session_id);_id(self.id)
        if self.receipt_stream_revision is not None and not _natural(self.receipt_stream_revision,1):
            _invalid()

    @property
    def streams(self) -> Streams:
        return Streams(self)

    async def replace_device(self, input: MoveInput, options: MutationOptions) -> Operation[MoveResult]:
        from .sdk_movements import replace_device
        return await replace_device(self, input, options)

    async def _read(self, action: dict, options: RequestOptions, decode: Callable[[Any],T]) -> T:
        return await self._client._helper.request({"type":"connection", "conversation_id":self.conversation_id, "session_id":self.session_id, "connection_id":self.id, "action":action}, options, decode)

    async def _mutate(self, action: dict, kind: OperationKind[T], options: MutationOptions) -> Operation[T]:
        action = {**action, "mutation":_mutation(options)}
        key = OperationKey(options.operation_id,kind.command,Scope(self.conversation_id,self.session_id,self.id))
        return await self._client._helper.request({"type":"connection", "conversation_id":self.conversation_id, "session_id":self.session_id, "connection_id":self.id, "action":action}, options.request, lambda value:self._client._from_reply(value,kind,key), key, True)

    async def refresh(self, options: RequestOptions) -> ConnectionSnapshot:
        def decode(value):
            if (not _object(value,("id", "conversationId", "sessionId", "generation", "streamRevision", "state"))
                    or (value["id"],value["conversationId"],value["sessionId"]) != (self.id,self.conversation_id,self.session_id)
                    or not _natural(value["generation"],1) or type(value["state"]) is not str
                    or value["streamRevision"] is not None and not _natural(value["streamRevision"],1)):
                return _bad()
            return ConnectionSnapshot(self.id,self.conversation_id,self.session_id,value["generation"],value["streamRevision"],value["state"])
        return await self._read({"type":"refresh"},options,decode)

    async def update(self, input: ConnectionUpdate, expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[ConnectionUpdateReceipt]:
        update: dict
        streams = lambda v:None if v is None else _nonempty(v,identifier)
        if type(input) is Mute:
            if type(input.action) is not str or input.action not in ("mute", "unmute"):
                return _invalid()
            update = {"action":input.action,"streams":_nonempty(input.streams,identifier)}
        elif type(input) is HoldConnection:
            if type(input.direction) is not str or input.direction not in ("sendonly", "recvonly", "inactive"):
                return _invalid()
            update = {"action":"hold","streams":streams(input.streams),"direction":input.direction}
        elif type(input) is ResumeConnection:
            update = {"action":"resume","streams":streams(input.streams)}
        elif type(input) is Quality:
            if not _natural(input.interval_ms) or input.interval_ms >= 2**32:
                return _invalid()
            update = {"action":"quality","interval_ms":input.interval_ms}
        elif type(input) is ConnectionCodecRenegotiate:
            update = {"action":"codec-renegotiate","streams":streams(input.streams),"codec_preferences":_nonempty(input.codec_preferences,_text)}
        else:
            return _invalid()
        return await self._mutate({"type":"update","update":update,"expected_revision":_revision(expected_stream_revision)},self._client.operation_kinds.connection_update,options)

    async def end(self, reason_code: int, reason: str, expected_stream_revision: StreamRevision | None, options: MutationOptions) -> Operation[Connection]:
        if type(reason_code) is not int or not 0 <= reason_code <= 65535 or type(reason) is not str:
            return _invalid()
        return await self._mutate({"type":"end","reason_code":reason_code,"reason":reason,"expected_revision":None if expected_stream_revision is None else _revision(expected_stream_revision)},self._client.operation_kinds.connection_end,options)


@dataclass(frozen=True, repr=False)
class Streams:
    _connection: Connection

    def by_id(self, id: StreamId) -> Stream:
        return Stream(self._connection,_id(id))

    async def subscribe(self, selectors: NonEmpty[StreamSelector], expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[StreamSelectionReceipt]:
        values: list[dict]=[]
        for s in _nonempty(selectors,lambda v:type(v) in (StreamTarget,ParticipantStreams)):
            if type(s) is StreamTarget:
                values.append({"type":"stream","stream_id":_id(s.stream)})
            elif type(s) is ParticipantStreams:
                values.append({"type":"participant","participant_id":_id(s.participant),"kinds":None if s.kinds is None else _nonempty(s.kinds,_medium)})
            else:
                return _invalid()
        return await self._connection._mutate({"type":"subscribe","selectors":values,"expected_revision":_revision(expected_stream_revision)},self._connection._client.operation_kinds.stream_subscribe,options)

    async def unsubscribe(self, streams: NonEmpty[StreamId], expected_stream_revision: StreamRevision, options: MutationOptions) -> Operation[StreamSelectionReceipt]:
        return await self._connection._mutate({"type":"unsubscribe","streams":_nonempty(streams,identifier),"expected_revision":_revision(expected_stream_revision)},self._connection._client.operation_kinds.stream_unsubscribe,options)


@dataclass(frozen=True, repr=False)
class Stream:
    _connection: Connection
    id: StreamId

    @property
    def media(self) -> MediaStream:
        return self._connection.media.stream_by_id(self.id)

    def __post_init__(self):
        _id(self.id)

    @property
    def conversation_id(self) -> ConversationId:
        return self._connection.conversation_id

    @property
    def session_id(self) -> SessionId:
        return self._connection.session_id

    @property
    def connection_id(self) -> ConnectionId:
        return self._connection.id

    async def refresh(self, options: RequestOptions) -> StreamSnapshot:
        def decode(value):
            if (not _object(value,("id","conversationId","sessionId","connectionId","medium","state","resourceRevision","direction","format","clock","ownership"))
                    or (value["id"],value["conversationId"],value["sessionId"],value["connectionId"]) != (self.id,self.conversation_id,self.session_id,self.connection_id)
                    or not _medium(value["medium"]) or type(value["state"]) is not str or not _natural(value["resourceRevision"])
                    or value["direction"] is not None and not _direction(value["direction"])
                    or type(value["ownership"]) is not str or value["ownership"] not in ("observation","local-source","local-sink","local-duplex")):
                return _bad()
            format=None;clock=None
            if value["format"] is not None:
                f=value["format"]
                if (not _object(f,("codec","clockRate","channels","width","height")) or type(f["codec"]) is not str
                        or not _natural(f["clockRate"],1) or any(f[k] is not None and not _natural(f[k],1) for k in ("channels","width","height"))):
                    return _bad()
                format=MediaFormat(f["codec"],f["clockRate"],f["channels"],f["width"],f["height"])
            if value["clock"] is not None:
                c=value["clock"]
                if (not _object(c,("sourceId","generation","unitsPerSecond","synchronized")) or type(c["sourceId"]) is not str or not c["sourceId"]
                        or not _natural(c["generation"],1) or type(c["unitsPerSecond"]) is not str
                        or not re.fullmatch(r"[1-9][0-9]{0,19}",c["unitsPerSecond"]) or int(c["unitsPerSecond"]) >= 2**64 or type(c["synchronized"]) is not bool):
                    return _bad()
                clock=SourceClock(c["sourceId"],c["generation"],int(c["unitsPerSecond"]),c["synchronized"])
            return StreamSnapshot(self.id,self.conversation_id,self.session_id,self.connection_id,value["medium"],value["state"],StreamResourceRevision(value["resourceRevision"]),value["direction"],format,clock,value["ownership"])
        return await self._connection._read({"type":"stream_refresh","stream_id":self.id},options,decode)
