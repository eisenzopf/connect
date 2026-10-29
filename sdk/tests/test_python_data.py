"""Reliable data through an owned synthetic helper; no native-server claim."""
import asyncio
import base64
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0,str(Path(__file__).resolve().parents[1]/'python'))
from connect_sdk import (Client, ConversationId, SessionId, ConnectionId, StreamId,
    MessageId, OperationId, ParticipantId, NonEmpty, MutationOptions, RequestOptions,
    Cancellation, SdkError, SdkCancelledError, TextData, JsonData, BinaryData, ReceivedData)

FIXTURE=Path(__file__).parent/'fixtures/sdk_media_helper.py'
OPTIONS=RequestOptions(1000)
AUDIENCE=NonEmpty(ParticipantId('recipient'))


class PythonData(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.temporary=tempfile.TemporaryDirectory();self.root=Path(self.temporary.name)
        self.config=self.root/'client.json';self.release=self.root/'release';self.clients=[]
        self.config.write_text(json.dumps({'pids':str(self.root/'pids'),'capture':str(self.root/'capture'),'release':str(self.release)}))

    def client(self,mode='normal'):
        value=json.loads(self.config.read_text());value['mode']=mode;self.config.write_text(json.dumps(value))
        client=Client(self.config,FIXTURE);self.clients.append(client);return client

    def stream(self,client):
        return client.media.connection(ConversationId('room'),SessionId('session'),ConnectionId('connection')).stream_by_id(StreamId('application'))

    def mutation(self,id='data-operation',options=OPTIONS):
        return MutationOptions(OperationId(id),options)

    def captures(self):
        p=self.root/'capture';return [json.loads(line) for line in p.read_text().splitlines()] if p.exists() else []

    async def seen(self,action,count=1):
        async with asyncio.timeout(1):
            while sum(r['channel']=='media' and r['request']['command']['action']==action for r in self.captures())<count:
                await asyncio.sleep(.005)

    async def asyncTearDown(self):
        self.release.touch()
        for client in self.clients:
            await client.close(RequestOptions(2000))
            assert all(task.done() for task in client._helper._tasks)
            if client.media._owner.opening is not None and not client.media._owner.opening.exception():
                assert all(t.done() for t in client.media._owner.opening.result().tasks)
        p=self.root/'pids'
        if p.exists():
            for pid in p.read_text().splitlines():
                try:os.kill(int(pid),0)
                except ProcessLookupError:pass
                else:self.fail('Original data helper still exists')
        self.temporary.cleanup()

    async def test_three_full_64k_payloads_empty_values_and_frozen_json_keep_original_control_recovery(self):
        client=self.client();stream=self.stream(client)
        source={'data':'x'*(65536-11)};captured=JsonData(source);source['data']='changed'
        values=[TextData('🦀'+'x'*65532),captured,BinaryData(bytes(range(256))*256),TextData(''),BinaryData(b''),JsonData(None)]
        for i,payload in enumerate(values):
            with self.subTest(format=payload.kind,index=i):
                operation=await stream.send_data(MessageId(f'message-{i}'),AUDIENCE,payload,self.mutation(f'operation-{i}'))
                key=operation.reference._key
                self.assertEqual(key.scope.connection_id,'connection')
                saved=operation.reference_json()
                for wrong in [client.operation_kinds.message_send,client.operation_kinds.connection_offer]:
                    with self.assertRaises(SdkError):client.operations.restore(saved,wrong)
                restored=client.operations.restore(saved,client.operation_kinds.data_send)
                completed=await restored.wait(OPTIONS)
                self.assertEqual(completed.snapshot.key,key)
                self.assertEqual(completed.value.message_id,f'message-{i}')
                self.assertEqual(completed.value.boundary,'core-queue-admission')
                self.assertNotIn('payload',saved)
                received=await stream.receive_data(OPTIONS)
                self.assertIsInstance(received,ReceivedData)
                self.assertEqual((received.message_id,received.operation_id,received.from_participant),(f'message-{i}',f'operation-{i}','sender'))
                self.assertEqual(received.payload,payload)
        sent=[r for r in self.captures() if r['channel']=='media' and r['request']['command']['action']=='send_data']
        self.assertEqual([len(base64.b64decode(r['payload'])) for r in sent[:3]],[65536]*3)
        self.assertTrue(all('payload' not in r['action'] for r in self.captures() if r['channel']=='control'))
        stats=await client.media.data_stats(OPTIONS);self.assertEqual(stats.accepted_messages,2**64-1)
        self.assertNotIn('xxxxx',repr(captured));self.assertNotIn('private',repr(TextData('private')))

    async def test_invalid_payloads_audience_metadata_and_precancel_keep_unsent_key_without_starting_helper(self):
        for constructor,value in [(TextData,'x'*65537),(TextData,'\ud800'),(BinaryData,b'x'*65537),(BinaryData,bytearray(b'x')),
                                  (JsonData,{'x':'x'*65536}),(JsonData,float('inf')),(JsonData,object())]:
            with self.assertRaises(SdkError):constructor(value)
        client=self.client();stream=self.stream(client)
        for audience in [NonEmpty(ParticipantId('same'),[ParticipantId('same')]),NonEmpty(ParticipantId('invalid space'))]:
            with self.assertRaises(SdkError) as caught:await stream.send_data(MessageId('message'),audience,BinaryData(b''),self.mutation())
            self.assertEqual((caught.exception.certainty,caught.exception.operation.operation_id),('not-submitted','data-operation'))
        options=MutationOptions(OperationId('oversize-metadata'),OPTIONS,{'extension':'x'*17000})
        with self.assertRaises(SdkError) as caught:await stream.send_data(MessageId('message'),AUDIENCE,BinaryData(b''),options)
        self.assertEqual(caught.exception.operation.operation_id,'oversize-metadata')
        signal=Cancellation();signal.cancel()
        with self.assertRaises(SdkError) as caught:await stream.send_data(MessageId('message'),AUDIENCE,BinaryData(b''),self.mutation(options=RequestOptions(1000,signal)))
        self.assertEqual((caught.exception.certainty,caught.exception.operation.operation_id),('not-submitted','data-operation'))
        self.assertFalse((self.root/'pids').exists())
        ordinary={'schema':'connect-operation-reference/1','operation_id':'same','command':'message.send','scope':{'conversation_id':'room'}}
        with self.assertRaises(SdkError):client.operations.restore(ordinary,client.operation_kinds.data_send)

    async def test_cancelled_data_receive_keeps_original_queue_item_for_cloned_handle(self):
        client=self.client('data-receive-held');signal=Cancellation();stream=self.stream(client)
        task=asyncio.create_task(stream.receive_data(RequestOptions(1000,signal)));await self.seen('receive_data')
        with self.assertRaises(SdkError) as caught:await self.stream(client).receive_data(OPTIONS)
        self.assertEqual(caught.exception.kind,'conflict')
        signal.cancel()
        with self.assertRaises(SdkError) as caught:await task
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('cancelled','read-only'))
        await client.capabilities.refresh(OPTIONS);self.release.touch()
        result=await self.stream(client).receive_data(OPTIONS)
        self.assertEqual(result.message_id,'original-message')
        self.assertEqual(sum(r['channel']=='media' and r['request']['command']['action']=='receive_data' for r in self.captures()),1)

    async def test_sent_data_signal_and_task_cancel_preserve_original_ids_without_replay_or_restart(self):
        for cancel_task in [False,True]:
            client=self.client('data-send-held');self.release.unlink(missing_ok=True)
            signal=Cancellation();opid='task-cancel' if cancel_task else 'signal-cancel'
            task=asyncio.create_task(self.stream(client).send_data(MessageId('message'),AUDIENCE,BinaryData(b'original'),self.mutation(opid,RequestOptions(1000,signal))))
            await self.seen('send_data',2 if cancel_task else 1)
            if cancel_task:task.cancel()
            else:signal.cancel()
            try:await task
            except (SdkError,SdkCancelledError) as failure:
                self.assertEqual((failure.kind,failure.certainty,failure.operation.operation_id),('cancelled','uncertain',opid))
            else:self.fail('Cancelled data wait succeeded')
            await client.capabilities.refresh(OPTIONS);self.release.touch()
            async with asyncio.timeout(1):
                while client.media._owner.opening.result().pending:await asyncio.sleep(.005)
            self.assertEqual(len([r for r in self.captures() if r['channel']=='media' and r['request']['command']['action']=='send_data']),2 if cancel_task else 1)
            await client.close(RequestOptions(2000))

    async def test_sent_deadline_retains_original_intent_and_late_response_cannot_trigger_retry(self):
        client=self.client('data-send-deadline')
        # Open independently so the short caller budget measures the sent wait.
        await client.media.data_stats(OPTIONS)
        with self.assertRaises(SdkError) as caught:await self.stream(client).send_data(MessageId('message'),AUDIENCE,TextData('original'),self.mutation(options=RequestOptions(40)))
        self.assertEqual((caught.exception.kind,caught.exception.certainty,caught.exception.operation.operation_id),('deadline','uncertain','data-operation'))
        await client.capabilities.refresh(OPTIONS);self.release.touch()
        async with asyncio.timeout(1):
            while client.media._owner.opening.result().pending:await asyncio.sleep(.005)
        self.assertEqual(sum(r['channel']=='media' and r['request']['command']['action']=='send_data' for r in self.captures()),1)

    async def test_remote_refusal_and_recorded_snapshot_preserve_original_key_and_do_not_fence_control(self):
        for mode in ['data-refused','data-recorded']:
            client=self.client(mode)
            with self.assertRaises(SdkError) as caught:await self.stream(client).send_data(MessageId('message'),AUDIENCE,TextData('private'),self.mutation())
            error=caught.exception
            self.assertEqual(error.operation.operation_id,'data-operation');self.assertEqual(error.problem.code,403)
            self.assertEqual(error.certainty,'recorded' if mode=='data-recorded' else 'refused')
            if mode=='data-recorded':
                self.assertEqual(error.observation['recordId'],'original-record')
                with self.assertRaises(TypeError):error.observation['state']='completed'
            await client.media.data_stats(OPTIONS);await client.capabilities.refresh(OPTIONS);await client.close(RequestOptions(2000))

    async def test_foreign_reference_error_or_observation_fences_media_with_original_uncertainty(self):
        for mode in ['data-foreign-reference','data-foreign-error','data-foreign-observation']:
            client=self.client(mode)
            with self.assertRaises(SdkError) as caught:await self.stream(client).send_data(MessageId('message'),AUDIENCE,TextData('private'),self.mutation())
            self.assertEqual((caught.exception.kind,caught.exception.certainty,caught.exception.operation.operation_id),('protocol','uncertain','data-operation'))
            await client.capabilities.refresh(OPTIONS);await client.close(RequestOptions(2000))

    async def test_bad_received_formats_and_foreign_binding_fence_media_read_only(self):
        for mode in ['data-bad-utf8','data-bad-json','data-foreign-binding']:
            client=self.client(mode)
            with self.assertRaises(SdkError) as caught:await self.stream(client).receive_data(OPTIONS)
            self.assertEqual((caught.exception.kind,caught.exception.certainty),('protocol','read-only'))
            await client.capabilities.refresh(OPTIONS);await client.close(RequestOptions(2000))
        client=self.client('data-json-representation')
        value=await self.stream(client).receive_data(OPTIONS)
        self.assertEqual(value.payload,JsonData({'key':0.00001}))
        self.assertEqual(value.payload._bytes,b'{ "key": 0.00001 }')

    async def test_sixteen_data_requests_bound_window_and_close_joins_original_uncertain_keys(self):
        client=self.client('data-send-held');stream=self.stream(client);await client.media.data_stats(OPTIONS)
        tasks=[asyncio.create_task(stream.send_data(MessageId(f'message-{i}'),AUDIENCE,BinaryData(b'original'),self.mutation(f'operation-{i}'))) for i in range(16)]
        try:
            await self.seen('send_data',16)
            with self.assertRaises(SdkError) as caught:await stream.send_data(MessageId('message-17'),AUDIENCE,BinaryData(b'bounded'),self.mutation('operation-17'))
            self.assertEqual((caught.exception.kind,caught.exception.certainty,caught.exception.operation.operation_id),('application','not-submitted','operation-17'))
            await client.capabilities.refresh(OPTIONS);await client.close(RequestOptions(2000))
            outcomes=await asyncio.gather(*tasks,return_exceptions=True)
            self.assertTrue(all(isinstance(v,SdkError) and v.certainty=='uncertain' and v.operation.operation_id==f'operation-{i}' for i,v in enumerate(outcomes)))
        finally:await asyncio.gather(*tasks,return_exceptions=True)

    async def test_cancelled_completed_data_slots_cannot_hide_more_than_one_megabyte(self):
        client=self.client('data-receive-held')
        await self.stream(client).send_data(MessageId('message'),AUDIENCE,BinaryData(b'x'*65536),self.mutation())
        signal=Cancellation()
        streams=[client.media.connection(ConversationId('room'),SessionId('session'),ConnectionId('connection')).stream_by_id(StreamId(f'data-{i}')) for i in range(17)]
        tasks=[asyncio.create_task(s.receive_data(RequestOptions(1000,signal))) for s in streams[:16]]
        await self.seen('receive_data',16);signal.cancel()
        outcomes=await asyncio.gather(*tasks,return_exceptions=True)
        self.assertTrue(all(isinstance(v,SdkError) and v.certainty=='read-only' for v in outcomes))
        self.release.touch()
        async with asyncio.timeout(1):
            while client.media._owner.opening.result().pending or not all(s.flight.done() for s in client.media._owner.slots.values()):
                await asyncio.sleep(.005)
        with self.assertRaises(SdkError) as caught:await streams[16].receive_data(OPTIONS)
        self.assertEqual((caught.exception.kind,caught.exception.certainty),('application','read-only'))
        self.assertEqual(sum(r['channel']=='media' and r['request']['command']['action']=='receive_data' for r in self.captures()),16)
        self.assertEqual(len((await streams[0].receive_data(OPTIONS)).payload.value),65536)
        self.assertEqual(len((await streams[16].receive_data(OPTIONS)).payload.value),65536)

    async def test_helper_death_wakes_sent_data_with_original_key_and_joined_actors(self):
        client=self.client('die-media')
        with self.assertRaises(SdkError) as caught:await self.stream(client).send_data(MessageId('message'),AUDIENCE,BinaryData(b'original'),self.mutation())
        self.assertEqual((caught.exception.kind,caught.exception.certainty,caught.exception.operation.operation_id),('transport-uncertain','uncertain','data-operation'))
        await client.close(RequestOptions(2000))


if __name__=='__main__':unittest.main()
