"""Typed reliable native data, with captured bytes and explicit queue admission."""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import ClassVar, Literal, TypeAlias, NoReturn

from ._sdk_helper import _encode, _parse
from .sdk_types import ConnectionId, Json, MessageId, OperationId, ParticipantId, SdkError, freeze, identifier

MAX_DATA_BYTES = 65536


def invalid() -> NoReturn:
    raise SdkError('validation','not-submitted')


@dataclass(frozen=True,repr=False)
class TextData:
    value: str
    kind: ClassVar[Literal['text']] = 'text'
    _bytes: bytes = field(init=False,compare=False)

    def __post_init__(self):
        try:
            if type(self.value) is not str:
                invalid()
            raw=self.value.encode('utf-8')
            if len(raw)>MAX_DATA_BYTES:
                invalid()
        except UnicodeError:
            invalid()
        object.__setattr__(self,'_bytes',raw)

    def __repr__(self):
        return f'<Connect text data bytes={len(self._bytes)}>'


@dataclass(frozen=True,repr=False)
class JsonData:
    value: Json
    kind: ClassVar[Literal['json']] = 'json'
    _bytes: bytes = field(init=False,compare=False)

    def __post_init__(self):
        try:
            captured,raw=_encode(self.value,MAX_DATA_BYTES)
        except (ValueError,TypeError,UnicodeError,RecursionError):
            invalid()
        object.__setattr__(self,'value',freeze(captured))
        object.__setattr__(self,'_bytes',raw)

    def __repr__(self):
        return f'<Connect JSON data bytes={len(self._bytes)}>'


@dataclass(frozen=True,repr=False)
class BinaryData:
    value: bytes
    kind: ClassVar[Literal['binary']] = 'binary'

    def __post_init__(self):
        if type(self.value) is not bytes or len(self.value)>MAX_DATA_BYTES:
            invalid()

    @property
    def _bytes(self) -> bytes:
        return self.value

    def __repr__(self):
        return f'<Connect binary data bytes={len(self.value)}>'


DataPayload: TypeAlias = TextData | JsonData | BinaryData


def received_payload(format,raw:bytes) -> DataPayload:
    if format=='text':
        return TextData(raw.decode('utf-8',errors='strict'))
    if format=='json':
        # Retain the validated received representation, including exponent
        # spelling, rather than rejecting it when another serializer expands it.
        value=_parse(raw,maximum_bytes=MAX_DATA_BYTES)
        result=object.__new__(JsonData)
        object.__setattr__(result,'value',freeze(value))
        object.__setattr__(result,'_bytes',raw)
        return result
    if format=='binary':
        return BinaryData(raw)
    raise ValueError()


@dataclass(frozen=True,repr=False)
class ReceivedData:
    message_id: MessageId
    operation_id: OperationId
    from_participant: ParticipantId
    payload: DataPayload

    def __post_init__(self):
        if (any(not identifier(v) for v in (self.message_id,self.operation_id,self.from_participant))
                or type(self.payload) not in (TextData,JsonData,BinaryData)):
            invalid()


@dataclass(frozen=True)
class DataAcceptance:
    message_id: MessageId
    connection_id: ConnectionId
    boundary: ClassVar[Literal['core-queue-admission']] = 'core-queue-admission'


@dataclass(frozen=True)
class DataStats:
    received_frames: int
    accepted_messages: int
    malformed_frames: int
    unowned_frames: int
    overflow_drops: int
    binding_change_drops: int
    queued_messages: int
    queued_bytes: int
