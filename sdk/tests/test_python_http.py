"""Real local canonical HTTP requests, immutable results and process ownership."""
import asyncio
from dataclasses import FrozenInstanceError
import json
import os
from unittest.mock import patch

from connect_sdk import (CanonicalHttpClient,Cancellation,ConversationId,HttpCommandScope,OperationId,RequestOptions,SdkCancelledError,SdkError)
from _python_http_fixture import OPTIONS,PythonHttpFixture,batch,frame,reply

READ=HttpCommandScope(ConversationId('room'))
MUTATION=HttpCommandScope(ConversationId('room'),operation_id=OperationId('original'))


async def stall(value,reader,writer):
    writer.write(b'HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\n\r\n{');await writer.drain()
    await reader.read()


class TypedPythonHttp(PythonHttpFixture):
    async def test_captured_current_route_bearer_original_intent_and_immutable_reply(self):
        async def respond(value,reader,writer):
            await reply(writer,batch(value,[frame(value,payload={'details':{'private':'preserved'}})]))
        client=self.client(await self.server(respond));payload={'text':'original','extensions':{'optional':True}}
        pending=asyncio.create_task(client.request('message.send',payload,MUTATION,OPTIONS));await asyncio.sleep(0)
        payload['text']='changed';payload['extensions']['optional']=False
        values=await pending;self.assertEqual(self.requests[0]['line'],'POST /v2/commands HTTP/1.1')
        self.assertEqual(self.requests[0]['headers']['authorization'],'Bearer synthetic-private-http-credential')
        self.assertEqual(self.requests[0]['value']['payload'],{'text':'original','extensions':{'optional':True}})
        self.assertEqual(self.requests[0]['value']['operation_id'],'original')
        self.assertIsInstance(values,tuple);self.assertEqual(values[0].value['payload']['details']['private'],'preserved')
        with self.assertRaises(TypeError):values[0].value['payload']['details']['private']='changed'
        exported=values[0].to_json();exported['payload']['details']['private']='changed'
        self.assertEqual(values[0].value['payload']['details']['private'],'preserved')
        self.assertNotIn('preserved',repr(values[0]));self.assertNotIn('private',repr(client))
        self.assertTrue(self.helpers[0]._joined);self.assertEqual(client._active,set())

    async def test_configuration_rejects_implicit_remote_transport_and_unsafe_origins_before_io(self):
        for endpoint,transport in [('http://example.com','loopback-http'),('https://example.com','loopback-http'),('http://127.0.0.1','remote-https'),
                                   ('http://127.0.0.1/path','loopback-http'),('http://127.0.0.1/?query=1','loopback-http'),('http://user:pass@127.0.0.1','loopback-http'),
                                   ('http://127.0.0.1/#x','loopback-http'),('http://127.0.0.1:0','loopback-http'),(' http://127.0.0.1','loopback-http')]:
            with self.assertRaises(SdkError):CanonicalHttpClient(endpoint,'credential',transport)
        for credential in ['', 'bad\r\nheader','has space','x'*16385]:
            with self.assertRaises(SdkError):CanonicalHttpClient('http://127.0.0.1',credential)
        self.assertEqual(self.helpers,[]);self.assertEqual(self.requests,[])

    async def test_local_finite_policy_bounds_and_cancellation_start_no_worker(self):
        client=self.client(await self.server());signal=Cancellation();signal.cancel()
        await self.assert_error('cancelled','not-submitted',client.request('message.send',{},MUTATION,RequestOptions(1500,signal)),'original')
        for kind,payload in [('auth.refresh',{}),('connection.offer',{}),('conversation.subscribe',{})]:
            await self.assert_error('unsupported','not-submitted',client.request(kind,payload,MUTATION,OPTIONS))
        await self.assert_error('validation','not-submitted',client.request('message.send',{'body':'x'*65536},MUTATION,OPTIONS),'original')
        await self.assert_error('validation','not-submitted',client.request('message.send',{},READ,OPTIONS))
        self.assertEqual(self.helpers,[]);self.assertEqual(self.requests,[])

    async def test_explicit_replay_has_fresh_physical_metadata_and_one_original_logical_id(self):
        client=self.client(await self.server())
        for _ in range(2):await client.request('message.send',{'text':'same'},MUTATION,OPTIONS)
        self.assertEqual(len(self.requests),2)
        self.assertNotEqual(self.requests[0]['value']['id'],self.requests[1]['value']['id'])
        self.assertEqual([r['value']['operation_id'] for r in self.requests],['original','original'])
        self.assertEqual(len(self.helpers),2);self.assertTrue(all(h._joined for h in self.helpers))

    async def test_structured_status_refusals_keep_original_scope_and_actual_problem(self):
        status=409;reason='operation_conflict'
        async def respond(value,reader,writer):
            await reply(writer,{'profile':'connect/2','problem':{'code':status,'category':'state','reason':reason}},status)
        client=self.client(await self.server(respond))
        for status,kind in [(401,'authentication'),(403,'authentication'),(409,'conflict'),(488,'unsupported'),(501,'unsupported'),(400,'validation'),(413,'validation'),(503,'application')]:
            error=await self.assert_error(kind,'refused',client.request('message.send',{},MUTATION,OPTIONS),'original')
            self.assertEqual(error.problem.code,status);self.assertEqual(error.operation.scope.conversation_id,'room')
        status=409;reason='resync_required'
        await self.assert_error('resync-required','read-only',client.request('conversation.subscribe',{'live':False},READ,OPTIONS))
        self.assertEqual(len(self.requests),9)

    async def test_terminal_error_frames_are_typed_and_read_errors_remain_read_only(self):
        async def respond(value,reader,writer):
            await reply(writer,batch(value,[frame(value,'error',{'code':403,'category':'auth','reason':'forbidden'})]),403)
        client=self.client(await self.server(respond))
        await self.assert_error('authentication','read-only',client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))
        await self.assert_error('authentication','refused',client.request('message.send',{},MUTATION,OPTIONS),'original')

    async def test_redirect_and_wrong_content_type_never_change_the_captured_destination(self):
        async def respond(value,reader,writer):
            await reply(writer,b'',302,{'location':'http://127.0.0.1:1/private'})
        client=self.client(await self.server(respond))
        await self.assert_error('protocol','uncertain',client.request('message.send',{},MUTATION,OPTIONS),'original')
        self.assertEqual(len(self.requests),1)
        async def text(value,reader,writer):await reply(writer,batch(value),headers={'content-type':'text/plain'})
        other=self.client(await self.server(text))
        await self.assert_error('protocol','read-only',other.request('operation.get',{'operation_id':'original'},READ,OPTIONS))

    async def test_malformed_batch_correlation_scope_terminal_order_and_unsigned_profile_are_rejected(self):
        mode='request'
        async def respond(value,reader,writer):
            result=batch(value);r=result['envelopes'][0]
            if mode=='request':result['request_id']='foreign'
            if mode=='correlation':r['in_reply_to']='foreign'
            if mode=='scope':r['cid']='foreign'
            if mode=='logical':r['operation_id']='foreign'
            if mode=='duplicate':result['envelopes'].append(dict(r))
            if mode=='after-ack':result['envelopes'].append({**r,'id':'extra'})
            if mode=='no-ack':r['type']='operation.updated'
            if mode=='empty':result['envelopes']=[]
            if mode=='count':result['envelopes']=[{**r,'id':f'e-{i}'} for i in range(129)]
            if mode=='signature':r['signature']={'alg':'EdDSA','sig':'private'}
            if mode=='extra':result['private']='unrecognized'
            if mode=='profile':result['profile']='connect/1'
            if mode=='revision':r['expected_revision']=True
            if mode=='problem-status':result={'profile':'connect/2','problem':{'code':403,'category':'auth','reason':'forbidden'}}
            await reply(writer,result)
        client=self.client(await self.server(respond))
        for mode in ['request','correlation','scope','logical','duplicate','after-ack','no-ack','empty','count','signature','extra','profile','revision','problem-status']:
            await self.assert_error('protocol','uncertain',client.request('message.send',{},MUTATION,OPTIONS),'original')
        self.assertEqual(len(self.requests),14)

    async def test_json_unicode_duplicate_nonfinite_and_body_bounds_stay_private(self):
        raw=b''
        async def respond(value,reader,writer):await reply(writer,raw)
        client=self.client(await self.server(respond))
        for raw in [b'\xff',b'{"profile":"connect/2","profile":"connect/2"}',b'{"profile":"connect/2","number":NaN}',b'{"profile":"connect/2","text":"\\ud800"}',b'x'*(2097152+1)]:
            await self.assert_error('protocol','read-only',client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))

    async def test_finite_journal_binds_every_repeated_scope_cursor_and_preserves_optional_fields(self):
        wrong=False
        async def respond(value,reader,writer):
            data={'event_id':'event','cursor':'foreign' if wrong else 'cursor','conversation_id':'room','session_id':'session','connection_id':None,'operation_id':'original'}
            event=frame(value,'conversation.event',{'event_id':'event','cursor':'cursor','data':data},sid='session',operation_id='original')
            await reply(writer,batch(value,[event,frame(value,id='terminal')]))
        client=self.client(await self.server(respond));values=await client.request('conversation.subscribe',{'live':False},READ,OPTIONS)
        self.assertEqual(values[0].value['sid'],'session');self.assertEqual(values[-1].value['type'],'ack')
        wrong=True
        await self.assert_error('protocol','read-only',client.request('conversation.subscribe',{'live':False},READ,OPTIONS))

    async def test_deadline_and_cancel_close_original_socket_and_keep_uncertain_mutation_key(self):
        client=self.client(await self.server(stall))
        await self.assert_error('deadline','uncertain',client.request('message.send',{},MUTATION,RequestOptions(400)),'original')
        self.assertEqual(len(self.requests),1);self.assertTrue(self.helpers[-1]._joined)
        signal=Cancellation();pending=asyncio.create_task(client.request('message.send',{},MUTATION,RequestOptions(2000,signal)))
        await self.sent(2);signal.cancel();await self.assert_error('cancelled','uncertain',pending,'original')
        self.assertTrue(self.helpers[-1]._joined);self.assertEqual(len(self.requests),2)

    async def test_cancelled_task_remains_cancelled_and_an_independent_request_completes(self):
        async def respond(value,reader,writer):
            if value['type']=='message.send':await stall(value,reader,writer)
            else:await reply(writer,batch(value))
        client=self.client(await self.server(respond));pending=asyncio.create_task(client.request('message.send',{},MUTATION,OPTIONS))
        await self.sent();pending.cancel('private cancellation')
        with self.assertRaises(SdkCancelledError) as observed:await pending
        self.assertEqual((observed.exception.kind,observed.exception.certainty,observed.exception.operation.operation_id),('cancelled','uncertain','original'))
        self.assertTrue(pending.cancelled());self.assertTrue(self.helpers[0]._joined)
        self.assertEqual((await client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))[-1].value['type'],'ack')

    async def test_active_limit_close_join_and_closed_client_never_start_additional_work(self):
        client=self.client(await self.server(stall))
        tasks=[asyncio.create_task(client.request('operation.get',{'operation_id':'original'},READ,RequestOptions(4000))) for _ in range(16)]
        await asyncio.sleep(0)
        await self.assert_error('application','not-submitted',client.request('message.send',{},MUTATION,OPTIONS),'original')
        await client.close(OPTIONS)
        for task in tasks:await self.assert_error('cancelled','read-only',task)
        await client.close(OPTIONS);self.assertEqual(client._active,set())
        await self.assert_error('configuration','not-submitted',client.request('message.send',{},MUTATION,OPTIONS),'original')
        self.assertLessEqual(len(self.requests),16)

    async def test_unsent_deadline_has_no_http_effect_and_joins_late_spawn(self):
        client=self.client(await self.server())
        original=asyncio.create_subprocess_exec
        async def late(*args,**kwargs):
            await asyncio.sleep(.05);return await original(*args,**kwargs)
        with patch('asyncio.create_subprocess_exec',side_effect=late):
            await self.assert_error('deadline','not-submitted',client.request('message.send',{},MUTATION,RequestOptions(20)),'original')
        self.assertEqual(self.requests,[]);self.assertTrue(all(h._process is None or h._joined for h in self.helpers))

    async def test_exact_two_mib_reply_and_one_mib_frame_survive_the_private_owner_wrapper(self):
        sizes=[]
        async def respond(value,reader,writer):
            first=frame(value,'operation.updated',{'data':''})
            last=frame(value,payload={'details':{'data':''}},id='terminal')
            encode=lambda v:json.dumps(v,ensure_ascii=False,separators=(',',':')).encode()
            first['payload']['data']='x'*(1024*1024-len(encode(first)))
            result=batch(value,[first,last]);last['payload']['details']['data']='x'*(2097152-len(encode(result)))
            sizes.extend([len(encode(first)),len(encode(result))]);await reply(writer,result)
        client=self.client(await self.server(respond));values=await client.request('operation.get',{'operation_id':'original'},READ,RequestOptions(4000))
        self.assertEqual(sizes,[1048576,2097152]);self.assertEqual(len(values),2)
        self.assertGreater(len(values[0].value['payload']['data']),1000000)
        self.assertGreater(len(values[1].value['payload']['details']['data']),1000000)
        self.assertTrue(self.helpers[0]._joined)

    async def test_individual_frame_limit_is_not_widened_by_the_two_mib_batch_limit(self):
        async def respond(value,reader,writer):
            await reply(writer,batch(value,[frame(value,payload={'data':'x'*1048576})]))
        client=self.client(await self.server(respond))
        await self.assert_error('protocol','read-only',client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))

    async def test_canonical_safe_integer_and_depth_limits_are_enforced_before_io_and_on_replies(self):
        async def respond(value,reader,writer):
            await reply(writer,batch(value,[frame(value,payload={'data':9007199254740992})]))
        client=self.client(await self.server(respond));deep={}
        for _ in range(34):deep={'nested':deep}
        for payload in [{'data':9007199254740992},{'data':1e20},{'data':deep}]:
            await self.assert_error('validation','not-submitted',client.request('message.send',payload,MUTATION,OPTIONS),'original')
        self.assertEqual(self.helpers,[])
        await self.assert_error('protocol','read-only',client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))

    async def test_dense_objects_keep_the_documented_value_node_budget(self):
        async def respond(value,reader,writer):
            await reply(writer,batch(value,[frame(value,payload={'data':{f'k{i:05}':False for i in range(60000)}})]))
        client=self.client(await self.server(respond))
        values=await client.request('operation.get',{'operation_id':'original'},READ,RequestOptions(4000))
        self.assertEqual(len(values[0].value['payload']['data']),60000)
        exported=values[0].to_json()
        self.assertEqual(len(exported['payload']['data']),60000)
        exported['payload']['data']['k00000']=True
        self.assertFalse(values[0].value['payload']['data']['k00000'])

    async def test_foreign_event_loop_rejection_keeps_original_intent_without_new_worker(self):
        client=self.client(await self.server());await client.request('operation.get',{'operation_id':'original'},READ,OPTIONS)
        async def foreign():
            try:await client.request('message.send',{},MUTATION,OPTIONS)
            except SdkError as error:return error
            self.fail('foreign loop accepted')
        error=await asyncio.to_thread(lambda:asyncio.run(foreign()))
        self.assertEqual((error.kind,error.certainty,error.operation.operation_id),('configuration','not-submitted','original'))
        self.assertEqual(len(self.helpers),1);self.assertEqual(len(self.requests),1)

    async def test_deadline_terminates_and_joins_a_worker_blocked_inside_its_resolver(self):
        endpoint=await self.server();marker=self.directory/'resolver-entered'
        hook=self.directory/'sitecustomize.py'
        # Child-only fixture hook: block before the actual resolver and never
        # call it. No DNS query or nonlocal endpoint is used by this rehearsal.
        hook.write_text('import socket,time\nfrom pathlib import Path\ndef blocked(*args,**kwargs):\n Path('+repr(str(marker))+').write_text("entered")\n time.sleep(60)\n raise OSError("synthetic resolver blocked")\nsocket.getaddrinfo=blocked\n')
        environment={'PYTHONPATH':str(self.directory)+os.pathsep+os.environ.get('PYTHONPATH','')}
        with patch.dict('os.environ',environment):
            client=self.client(endpoint.replace('127.0.0.1','localhost'))
            await self.assert_error('deadline','uncertain',client.request('message.send',{},MUTATION,RequestOptions(500)),'original')
        self.assertTrue(marker.exists());self.assertEqual(self.requests,[])
        self.assertTrue(self.helpers[0]._joined);self.assertEqual(client._active,set())

    async def test_context_closes_pending_work_and_proxy_environment_does_not_route_requests(self):
        endpoint=await self.server()
        with patch.dict('os.environ',{'HTTP_PROXY':'http://127.0.0.1:1','HTTPS_PROXY':'http://127.0.0.1:1','ALL_PROXY':'http://127.0.0.1:1'}):
            async with self.client(endpoint) as client:
                self.assertEqual((await client.request('operation.get',{'operation_id':'original'},READ,OPTIONS))[-1].value['type'],'ack')
        self.assertEqual(len(self.requests),1)
