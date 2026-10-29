"""Owned language channel checks; synthetic child scope, no native media claims."""
import asyncio
import base64
import json
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python'))
from connect_sdk import (Client, ConversationId, SessionId, ConnectionId, StreamId, RtpPacket,
    SenderClockReport, ClockPacket, MediaGap, Cancellation, RequestOptions, SdkError, SdkCancelledError)

FIXTURE=Path(__file__).parent/'fixtures/sdk_media_helper.py'
OPTIONS=RequestOptions(1000)


class PythonMedia(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temporary=tempfile.TemporaryDirectory()
        self.root=Path(self.temporary.name)
        self.config=self.root/'client.json';self.release=self.root/'release'
        self.config.write_text(json.dumps({'pids':str(self.root/'pids'),'capture':str(self.root/'capture'),
                                          'release':str(self.release)}))
        self.clients=[]

    def client(self,mode='normal'):
        value=json.loads(self.config.read_text());value['mode']=mode;self.config.write_text(json.dumps(value))
        client=Client(self.config,FIXTURE);self.clients.append(client)
        return client

    def media(self,client):
        return client.conversations.by_id(ConversationId('room')).sessions.by_id(SessionId('session')).connections.by_id(ConnectionId('connection')).media

    def stream(self,client):
        return self.media(client).stream_by_id(StreamId('audio'))

    def captures(self):
        path=self.root/'capture'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    async def seen(self,action):
        async with asyncio.timeout(1):
            while not any(r['channel']=='media' and r['request']['command']['action']==action for r in self.captures()):
                await asyncio.sleep(.005)

    async def asyncTearDown(self):
        self.release.touch()
        for client in self.clients:
            await client.close(RequestOptions(2000))
            assert all(task.done() for task in client._helper._tasks)
        path=self.root/'pids'
        if path.exists():
            for pid in path.read_text().splitlines():
                try:os.kill(int(pid),0)
                except ProcessLookupError:pass
                else:self.fail('Original media helper still exists')
        self.temporary.cleanup()

    async def test_exact_rtp_extensions_padding_clock_and_decimal_counters_use_binary_channel(self):
        client=self.client();stream=self.stream(client)
        selected=await self.media(client).refresh(OPTIONS)
        self.assertEqual(selected.streams[0].codec['params']['sample_rate'],8000)
        with self.assertRaises(TypeError):selected.streams[0].codec['params']['sample_rate']=16000
        # CSRC + extension + padding; every original byte must survive.
        raw=struct.pack('!BBHII',0xb1,0x80,65535,2**32-1,9)+struct.pack('!IHHI',23,0x1000,1,0xabcdef01)+b'identifiable'+b'\0\0\0\4'
        original=RtpPacket(2**32-1,raw)
        accepted=await stream.send_rtp(original,OPTIONS)
        self.assertEqual(accepted.boundary,'local-media-queue')
        received=await stream.receive_packet(OPTIONS)
        self.assertIsInstance(received,RtpPacket)
        self.assertEqual(received,original)
        self.assertEqual(received.payload,b'identifiable')
        report=SenderClockReport(9,2**64-1,2,3,4,'synthetic-source')
        await stream.send_clock(2,report,OPTIONS)
        stats=await client.media.stats(OPTIONS)
        self.assertEqual(stats.received_datagrams,2**64-1)
        facts=self.captures()
        sent=next(r for r in facts if r['channel']=='media' and r['request']['command']['action']=='send_rtp')
        self.assertEqual(base64.b64decode(sent['payload']),raw)
        self.assertTrue(all('payload' not in r['action'] for r in facts if r['channel']=='control'))

    async def test_cancelled_read_preserves_one_owned_packet_for_next_wait_and_control_stays_live(self):
        client=self.client('receive-held');stream=self.stream(client)
        signal=Cancellation()
        original=asyncio.create_task(stream.receive_packet(RequestOptions(1000,signal)))
        await self.seen('receive');signal.cancel()
        with self.assertRaises(SdkError) as caught:await original
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('cancelled','read-only'))
        await client.capabilities.refresh(OPTIONS)
        self.release.touch()
        result=await stream.receive_packet(OPTIONS)
        self.assertEqual(result.payload,b'synthetic-packet')
        self.assertEqual(sum(r['channel']=='media' and r['request']['command']['action']=='receive' for r in self.captures()),1)

    async def test_cancelled_task_keeps_the_original_reader_owned_for_another_handle(self):
        client=self.client('receive-held');first=self.stream(client)
        original=asyncio.create_task(first.receive_packet(OPTIONS));await self.seen('receive');original.cancel()
        with self.assertRaises(SdkCancelledError) as caught:await original
        self.assertEqual(caught.exception.certainty,'read-only')
        self.release.touch()
        self.assertEqual((await self.stream(client).receive_packet(OPTIONS)).payload,b'synthetic-packet')

    async def test_sent_wait_cancellation_is_uncertain_without_restarting_control_or_replaying_bytes(self):
        client=self.client('send-held');stream=self.stream(client);signal=Cancellation()
        raw=struct.pack('!BBHII',0x80,0,1,160,2)+bytes([127])*160
        original=asyncio.create_task(stream.send_rtp(RtpPacket(1,raw),RequestOptions(1000,signal)))
        await self.seen('send_rtp');signal.cancel()
        with self.assertRaises(SdkError) as caught:await original
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('cancelled','uncertain'))
        await client.capabilities.refresh(OPTIONS)
        self.release.touch()
        await asyncio.sleep(.02)
        self.assertEqual(sum(r['channel']=='media' and r['request']['command']['action']=='send_rtp' for r in self.captures()),1)
        self.assertEqual(len((self.root/'pids').read_text().splitlines()),1)

    async def test_never_submitted_cancellation_and_invalid_rtp_start_no_helper(self):
        client=self.client();signal=Cancellation();signal.cancel()
        raw=struct.pack('!BBHII',0x80,0,1,160,2)+b'original'
        with self.assertRaises(SdkError) as caught:
            await self.stream(client).send_rtp(RtpPacket(1,raw),RequestOptions(1000,signal))
        self.assertEqual(caught.exception.certainty,'not-submitted')
        for body in [b'',raw[:10],bytes([0])+raw[1:],raw+b'x'*16396]:
            with self.assertRaises(SdkError):RtpPacket(1,body)
        self.assertFalse((self.root/'pids').exists())

    async def test_helper_without_media_ipc_version_refuses_before_attachment_and_control_stays_usable(self):
        client=self.client('old-helper')
        with self.assertRaises(SdkError) as caught:await client.media.stats(OPTIONS)
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('unsupported','read-only'))
        self.assertNotIn('native-programmatic-media',(await client.capabilities.refresh(OPTIONS)).features)
        self.assertFalse(any(r['channel']=='control' and r['action']['type']=='media_attach' for r in self.captures()))

    async def test_clock_gap_and_wide_counters_stay_distinct_exact_typed_results(self):
        clock=self.client('clock');result=await self.stream(clock).receive_packet(OPTIONS)
        self.assertIsInstance(result,ClockPacket);self.assertEqual(result.report.ntp,2**64-1)
        await clock.close(RequestOptions(2000))
        gap=self.client('gap');result=await self.stream(gap).receive_packet(OPTIONS)
        self.assertIsInstance(result,MediaGap);self.assertEqual(result.dropped_packets,2**64-1)

    async def test_malformed_packet_header_fences_media_only_and_keeps_control_usable(self):
        client=self.client('bad-header')
        with self.assertRaises(SdkError) as caught:await self.stream(client).receive_packet(OPTIONS)
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('protocol','read-only'))
        await client.capabilities.refresh(OPTIONS)

    async def test_helper_death_wakes_pending_media_and_original_tasks_join_on_close(self):
        client=self.client('die-media')
        with self.assertRaises(SdkError) as caught:await self.stream(client).receive_packet(OPTIONS)
        self.assertEqual(caught.exception.kind,'transport-uncertain')
        await client.close(RequestOptions(2000))
        channel=client.media._owner.opening.result()
        self.assertTrue(all(task.done() for task in channel.tasks))

    async def test_window_limit_refuses_seventeenth_send_and_close_joins_all_blocked_originals(self):
        client=self.client('send-held');stream=self.stream(client)
        await self.media(client).refresh(OPTIONS)
        raw=struct.pack('!BBHII',0x80,0,1,160,2)+b'bounded-packet'
        sends=[asyncio.create_task(stream.send_rtp(RtpPacket(i,raw),OPTIONS)) for i in range(16)]
        try:
            async with asyncio.timeout(1):
                while sum(r['channel']=='media' and r['request']['command']['action']=='send_rtp' for r in self.captures())<16:
                    await asyncio.sleep(.005)
            with self.assertRaises(SdkError) as caught:await stream.send_rtp(RtpPacket(17,raw),OPTIONS)
            self.assertEqual((caught.exception.kind,caught.exception.certainty),('application','not-submitted'))
            await client.capabilities.refresh(OPTIONS)
            await client.close(RequestOptions(2000))
            settled=await asyncio.gather(*sends,return_exceptions=True)
            self.assertTrue(all(isinstance(v,SdkError) and v.certainty=='uncertain' for v in settled))
        finally:
            await asyncio.gather(*sends,return_exceptions=True)

    async def test_fragmented_headers_and_bodies_preserve_one_original_binary_packet(self):
        client=self.client('fragmented')
        packet=await self.stream(client).receive_packet(OPTIONS)
        self.assertEqual(packet.payload,b'synthetic-packet')
        self.assertIn('native-programmatic-media',(await client.capabilities.refresh(OPTIONS)).features)

    async def test_foreign_binding_reply_refuses_without_disrupting_control(self):
        client=self.client('foreign-reply');raw=struct.pack('!BBHII',0x80,0,1,160,2)+b'original'
        with self.assertRaises(SdkError) as caught:await self.stream(client).send_rtp(RtpPacket(1,raw),OPTIONS)
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('protocol','uncertain'))
        await client.capabilities.refresh(OPTIONS)


if __name__=='__main__':unittest.main()
