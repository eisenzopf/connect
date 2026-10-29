"""Typed packet I/O on the original Client's separately owned binary channel."""
from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Callable, TypeVar

from ._sdk_helper import _SdkHelper
from ._sdk_media_channel import Channel, error, open_channel, owned, wait
from .sdk_media_types import (ClockPacket, LocalMediaSubmission, MediaBinding, MediaGap,
    MediaNegotiation, MediaPacket, MediaStats, NegotiatedMediaStream, RtpPacket, SenderClockReport,
    invalid, unsigned)
from .sdk_types import (ConnectionId, ConversationId, RequestOptions, SessionId, StreamId,
                        StreamRevision, SdkError, SdkCancelledError, cancelled, identifier)

T = TypeVar('T')


def shape(value, keys):
    if type(value) is not dict or set(value) != set(keys):
        raise ValueError()
    return value


def decimal(value):
    if type(value) is not str or not value or len(value) > 20:
        raise ValueError()
    parsed = int(value)
    if str(parsed) != value or not 0 <= parsed < 2**64:
        raise ValueError()
    return parsed


def ordinary(kind, payload):
    if kind != 0x81 or payload:
        raise ValueError()


def negotiation(binding: MediaBinding, kind, value, payload):
    ordinary(kind,payload)
    shape(value,('type','binding','source_clock','stream_revision','streams'))
    if (value['type'] != 'negotiation' or value['binding'] != binding._wire()
            or type(value['source_clock']) is not bool or type(value['stream_revision']) is not int
            or not 0 < value['stream_revision'] <= 9_007_199_254_740_991 or type(value['streams']) is not list):
        raise ValueError()
    streams=[]
    for raw in value['streams']:
        shape(raw,('id','medium','direction','codec','stream_local_id'))
        streams.append(NegotiatedMediaStream(raw['id'],raw['medium'],raw['direction'],raw['codec'],raw['stream_local_id']))
    if len({s.id for s in streams}) != len(streams):
        raise ValueError()
    return MediaNegotiation(binding,value['source_clock'],StreamRevision(value['stream_revision']),tuple(streams))


def submitted(binding, stream, sequence, kind, value, payload):
    ordinary(kind,payload)
    shape(value,('type','binding','stream_id','boundary','datagram_sequence'))
    if (value['type'] != 'submitted' or value['binding'] != binding._wire() or value['stream_id'] != stream
            or value['boundary'] != 'local-media-queue' or type(value['datagram_sequence']) is not int
            or value['datagram_sequence'] != sequence):
        raise ValueError()
    return LocalMediaSubmission(sequence)


def packet(binding, stream, kind, value, payload) -> MediaPacket:
    if kind != 0x82:
        raise ValueError()
    shape(value,('type','binding','stream_id','packet'))
    if value['type'] != 'packet' or value['binding'] != binding._wire() or value['stream_id'] != stream:
        raise ValueError()
    item=value['packet']
    if type(item) is not dict:
        raise ValueError()
    if item.get('type') == 'rtp':
        shape(item,('type','datagram_sequence','header'))
        result=RtpPacket(item['datagram_sequence'],payload)
        h=result.header
        shape(item['header'],('payload_type','sequence_number','timestamp','ssrc','marker'))
        if (type(item['header']['marker']) is not bool
                or any(type(item['header'][k]) is not int for k in ('payload_type','sequence_number','timestamp','ssrc'))
                or item['header'] != {'payload_type':h.payload_type,'sequence_number':h.sequence_number,
                    'timestamp':h.timestamp,'ssrc':h.ssrc,'marker':h.marker}):
            raise ValueError()
        return result
    if payload:
        raise ValueError()
    if item.get('type') == 'clock':
        shape(item,('type','datagram_sequence','report'))
        report=shape(item['report'],('ssrc','ntp','rtp','packets','octets','cname'))
        return ClockPacket(item['datagram_sequence'],SenderClockReport(report['ssrc'],decimal(report['ntp']),
            report['rtp'],report['packets'],report['octets'],report['cname']))
    if item.get('type') == 'gap':
        shape(item,('type','dropped_packets'))
        return MediaGap(decimal(item['dropped_packets']))
    raise ValueError()


def stats(kind,value,payload):
    ordinary(kind,payload)
    counters=('received_datagrams','rtp_packets','clock_reports','malformed_datagrams','unknown_handles',
              'overflow_drops','control_drops','binding_change_drops')
    shape(value,('type',*counters,'queued_packets','queued_bytes'))
    if (value['type'] != 'stats' or type(value['queued_packets']) is not int or not 0 <= value['queued_packets'] <= 256
            or type(value['queued_bytes']) is not int or not 0 <= value['queued_bytes'] <= 4*1024*1024):
        raise ValueError()
    return MediaStats(*(decimal(value[k]) for k in counters),value['queued_packets'],value['queued_bytes'])


@dataclass
class _Slot:
    flight: asyncio.Task | None = None
    busy: bool = False


class _Owner:
    def __init__(self,helper:_SdkHelper):
        self.helper=helper
        self.loop=None
        self.opening:asyncio.Task[Channel] | None=None
        self.detaching:asyncio.Task | None=None
        self.closed=False
        self.slots:dict[tuple,_Slot]={}

    def bind(self):
        loop=asyncio.get_running_loop()
        if ((self.loop is not None and self.loop is not loop)
                or self.helper._loop is not None and self.helper._loop is not loop):
            raise error('configuration')
        self.loop=loop

    async def channel(self,options:RequestOptions):
        self.bind()
        if self.closed:
            raise error('configuration')
        if cancelled(options.signal):
            raise error('cancelled')
        if self.opening is None:
            self.opening=owned(asyncio.create_task(open_channel(self.helper)))
        return await wait(self.opening,options)

    async def request(self,command:dict,payload:bytes,options:RequestOptions,decode:Callable[...,T],mutating=False) -> T:
        if type(options) is not RequestOptions:
            raise error('validation',mutating)
        started=asyncio.get_running_loop().time()
        try:
            channel=await self.channel(options)
        except SdkCancelledError:
            raise SdkCancelledError(error('cancelled',mutating)) from None
        except SdkError as failure:
            if mutating:
                raise SdkError(failure.kind,'not-submitted',problem=failure.problem) from None
            raise
        remaining=int(options.timeout_ms-(asyncio.get_running_loop().time()-started)*1000)
        if remaining <= 0:
            raise error('deadline',mutating)
        response=await channel.exchange(command,payload,RequestOptions(remaining,options.signal),mutating)
        try:
            return decode(*response)
        except (SdkError,ValueError,TypeError,KeyError,UnicodeError,RecursionError):
            channel.fence('protocol')
            raise error('protocol',mutating,mutating) from None

    async def receive(self,binding:MediaBinding,stream:StreamId,options:RequestOptions) -> MediaPacket:
        if type(options) is not RequestOptions:
            raise error('validation')
        self.bind()
        if self.closed or cancelled(options.signal):
            raise error('configuration' if self.closed else 'cancelled')
        key=(binding.conversation_id,binding.session_id,binding.connection_id,stream)
        slot=self.slots.get(key)
        if slot is None:
            if len(self.slots)>=256:
                raise error('application')
            slot=_Slot();self.slots[key]=slot
        if slot.busy:
            raise error('conflict')
        slot.busy=True
        def forget():
            if not slot.busy and slot.flight is None and self.slots.get(key) is slot:
                self.slots.pop(key,None)
        try:
            if slot.flight is None:
                slot.flight=owned(asyncio.create_task(self.request({'action':'receive','binding':binding._wire(),
                    'stream_id':stream},b'',RequestOptions(options.timeout_ms),lambda *v:packet(binding,stream,*v))))
                def failed(done):
                    if (done.cancelled() or done.exception() is not None) and slot.flight is done:
                        slot.flight=None;forget()
                slot.flight.add_done_callback(failed)
            original=slot.flight
            result=await wait(original,options)
            if slot.flight is original:
                slot.flight=None
            return result
        finally:
            slot.busy=False
            forget()

    async def close(self,options:RequestOptions):
        if type(options) is not RequestOptions:
            raise error('validation')
        if self.loop is None:
            self.closed=True
            return
        self.bind()
        self.closed=True
        if self.detaching is None:
            async def detach():
                if self.opening is None:
                    return
                try:
                    channel=await self.opening
                except (SdkError,OSError,TimeoutError,ValueError):
                    return
                channel.mark_closing()
                try:
                    def closed(value):
                        if value != {'profile':'connect-sdk-media/2','closed':True}:
                            raise error('protocol')
                    if not self.helper._closed:
                        await self.helper.request({'type':'media_detach'},RequestOptions(4000),closed)
                finally:
                    await channel.close()
                    flights=[s.flight for s in self.slots.values() if s.flight is not None]
                    if flights:
                        await asyncio.gather(*flights,return_exceptions=True)
                    self.slots.clear()
            self.detaching=owned(asyncio.create_task(detach()))
        await wait(self.detaching,options,closing=True)


@dataclass(frozen=True,init=False,repr=False)
class Media:
    _owner:_Owner

    def __init__(self,helper:_SdkHelper):
        object.__setattr__(self,'_owner',_Owner(helper))

    def connection(self,conversation_id:ConversationId,session_id:SessionId,connection_id:ConnectionId) -> MediaConnection:
        return MediaConnection(self._owner,MediaBinding(conversation_id,session_id,connection_id))

    async def stats(self,options:RequestOptions) -> MediaStats:
        return await self._owner.request({'action':'stats'},b'',options,stats)

    async def close(self,options:RequestOptions) -> None:
        await self._owner.close(options)


@dataclass(frozen=True,repr=False)
class MediaConnection:
    _owner:_Owner
    binding:MediaBinding

    async def refresh(self,options:RequestOptions) -> MediaNegotiation:
        return await self._owner.request({'action':'describe','binding':self.binding._wire()},b'',options,
            lambda *v:negotiation(self.binding,*v))

    def stream_by_id(self,stream_id:StreamId) -> MediaStream:
        if not identifier(stream_id):
            invalid()
        return MediaStream(self._owner,self.binding,stream_id)


@dataclass(frozen=True,repr=False)
class MediaStream:
    _owner:_Owner
    binding:MediaBinding
    id:StreamId

    async def send_rtp(self,packet:RtpPacket,options:RequestOptions) -> LocalMediaSubmission:
        if type(packet) is not RtpPacket:
            invalid()
        return await self._owner.request({'action':'send_rtp','binding':self.binding._wire(),'stream_id':self.id,
            'datagram_sequence':packet.datagram_sequence},packet.data,options,
            lambda *v:submitted(self.binding,self.id,packet.datagram_sequence,*v),True)

    async def send_clock(self,sequence:int,report:SenderClockReport,options:RequestOptions) -> LocalMediaSubmission:
        unsigned(sequence)
        if type(report) is not SenderClockReport:
            invalid()
        return await self._owner.request({'action':'send_clock','binding':self.binding._wire(),'stream_id':self.id,
            'datagram_sequence':sequence,'report':report._wire()},b'',options,
            lambda *v:submitted(self.binding,self.id,sequence,*v),True)

    async def receive_packet(self,options:RequestOptions) -> MediaPacket:
        return await self._owner.receive(self.binding,self.id,options)
