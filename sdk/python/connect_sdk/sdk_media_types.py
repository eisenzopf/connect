"""Captured negotiated parameters and exact RTP/clock packets; no device capture."""
from __future__ import annotations

from dataclasses import dataclass, field
import struct
from .sdk_video_feedback import VideoReceiveEpoch, capture_video_epoch
from typing import ClassVar, Literal, TypeAlias

from .sdk_types import (ConnectionId, ConversationId, Direction, Json, Medium,
                        SessionId, StreamId, StreamRevision, OperationId, SdkError, freeze, identifier)


def invalid():
    raise SdkError('validation','not-submitted')


def unsigned(value, bits=32):
    if type(value) is not int or not 0 <= value < 2**bits:
        invalid()
    return value


@dataclass(frozen=True, repr=False)
class MediaBinding:
    conversation_id: ConversationId
    session_id: SessionId
    connection_id: ConnectionId

    def __post_init__(self):
        if any(not identifier(v) for v in (self.conversation_id,self.session_id,self.connection_id)):
            invalid()

    def _wire(self):
        return {'conversation_id':self.conversation_id,'session_id':self.session_id,'connection_id':self.connection_id}


@dataclass(frozen=True)
class RtpHeader:
    payload_type: int
    sequence_number: int
    timestamp: int
    ssrc: int
    marker: bool


def capture_rtp_video_epoch(packet, value):
    epoch=capture_video_epoch(value)
    if (type(packet) is not RtpPacket or packet.video_epoch is not None or packet.header.ssrc != epoch.ssrc
            or ((packet.datagram_sequence-epoch.first_sequence) & 0xffffffff) >= 0x80000000
            or (packet.datagram_sequence==epoch.first_sequence and packet.header.timestamp!=epoch.first_timestamp)):
        raise ValueError()
    object.__setattr__(packet,'_video_epoch',epoch)

@dataclass(frozen=True, repr=False)
class RtpPacket:
    datagram_sequence: int
    data: bytes
    kind: ClassVar[Literal['rtp']] = 'rtp'
    _video_epoch: VideoReceiveEpoch | None = field(init=False,default=None)
    _header: RtpHeader = field(init=False)
    _payload_start: int = field(init=False)
    _payload_end: int = field(init=False)

    def __post_init__(self):
        unsigned(self.datagram_sequence)
        if type(self.data) is not bytes or not 12 <= len(self.data) <= 16396 or self.data[0] >> 6 != 2:
            invalid()
        start = 12 + (self.data[0] & 15)*4
        if start > len(self.data):
            invalid()
        if self.data[0] & 0x10:
            if start + 4 > len(self.data):
                invalid()
            start += 4 + struct.unpack('!H',self.data[start+2:start+4])[0]*4
        end = len(self.data)
        if self.data[0] & 0x20:
            padding = self.data[-1]
            if padding == 0:
                invalid()
            end -= padding
        if start > end:
            invalid()
        _, flags, seq, timestamp, ssrc = struct.unpack('!BBHII',self.data[:12])
        object.__setattr__(self,'_header',RtpHeader(flags & 127,seq,timestamp,ssrc,bool(flags & 128)))
        object.__setattr__(self,'_payload_start',start)
        object.__setattr__(self,'_payload_end',end)

    @property
    def video_epoch(self) -> VideoReceiveEpoch | None:
        return self._video_epoch

    @property
    def header(self) -> RtpHeader:
        return self._header

    @property
    def payload(self) -> bytes:
        return self.data[self._payload_start:self._payload_end]

    def __repr__(self):
        return f'<Connect RTP packet sequence={self.datagram_sequence} bytes={len(self.data)}>'


@dataclass(frozen=True, repr=False)
class SenderClockReport:
    ssrc: int
    ntp: int
    rtp: int
    packets: int
    octets: int
    cname: str

    def __post_init__(self):
        for v in (self.ssrc,self.rtp,self.packets,self.octets):
            unsigned(v)
        unsigned(self.ntp,64)
        try:
            valid=type(self.cname) is str and 0 < len(self.cname.encode('utf-8')) <= 255
        except UnicodeError:
            valid=False
        if not valid:
            invalid()

    def _wire(self):
        return {'ssrc':self.ssrc,'ntp':str(self.ntp),'rtp':self.rtp,'packets':self.packets,
                'octets':self.octets,'cname':self.cname}


@dataclass(frozen=True, repr=False)
class ClockPacket:
    datagram_sequence: int
    report: SenderClockReport
    kind: ClassVar[Literal['clock']] = 'clock'

    def __post_init__(self):
        unsigned(self.datagram_sequence)
        if type(self.report) is not SenderClockReport:
            invalid()


@dataclass(frozen=True)
class MediaGap:
    dropped_packets: int
    kind: ClassVar[Literal['gap']] = 'gap'

    def __post_init__(self):
        if unsigned(self.dropped_packets,64)==0:
            invalid()


MediaPacket: TypeAlias = RtpPacket | ClockPacket | MediaGap


@dataclass(frozen=True, repr=False)
class NativeVideoFeedback:
    profile: Literal['rtcp-pli-epoch/1']
    original_operation: OperationId
    original_offer: str
    stream_generation: int
    connection_generation: int

    def __post_init__(self):
        if (self.profile != 'rtcp-pli-epoch/1' or not identifier(self.original_operation)
                or not identifier(self.original_offer)
                or any(type(v) is not int or not 0 < v <= 9_007_199_254_740_991
                       for v in (self.stream_generation,self.connection_generation))):
            invalid()


@dataclass(frozen=True, repr=False)
class NegotiatedMediaStream:
    id: StreamId
    medium: Medium
    direction: Direction
    codec: Json
    stream_local_id: int | None
    video_feedback: NativeVideoFeedback | None = None

    def __post_init__(self):
        if (not identifier(self.id) or self.medium not in ('audio','video','screen','data')
                or self.direction not in ('sendonly','recvonly','sendrecv','inactive')):
            invalid()
        if self.stream_local_id is not None and unsigned(self.stream_local_id,16)==0:
            invalid()
        if self.video_feedback is not None and (type(self.video_feedback) is not NativeVideoFeedback
                or self.medium != 'video' or self.stream_local_id is None):
            invalid()
        object.__setattr__(self,'codec',freeze(self.codec))


@dataclass(frozen=True, repr=False)
class MediaNegotiation:
    binding: MediaBinding
    source_clock: bool
    stream_revision: StreamRevision
    streams: tuple[NegotiatedMediaStream,...]


@dataclass(frozen=True)
class LocalMediaSubmission:
    datagram_sequence: int
    boundary: Literal['local-media-queue'] = 'local-media-queue'


@dataclass(frozen=True)
class MediaStats:
    received_datagrams: int
    rtp_packets: int
    clock_reports: int
    malformed_datagrams: int
    unknown_handles: int
    overflow_drops: int
    control_drops: int
    binding_change_drops: int
    queued_packets: int
    queued_bytes: int
