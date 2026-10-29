"""Copied into a clean external venv; use public SDKs against local fake HTTP."""
import asyncio
import importlib.metadata
import importlib.resources
import json
import os
from pathlib import Path
import socket
import sys
from unittest.mock import patch

import connect_sdk
from connect_sdk import (CanonicalHttpClient, ConversationId, HttpCommandScope,
                        OperationId, RequestOptions, SynchronousSdk)

root=Path(sys.prefix).resolve()
assert Path(connect_sdk.__file__).resolve().is_relative_to(root)
assert importlib.metadata.version('connect-sdk')=='0.1.0'
assert importlib.resources.files('connect_sdk').joinpath('py.typed').is_file()
assert os.environ.get('PYTHONPATH') is None
options=RequestOptions(4000)
credential='synthetic-installed-control'
processes=[]
original_spawn=asyncio.create_subprocess_exec


async def capture(*args,**kwargs):
    assert args==(sys.executable,'-m','connect_sdk._sdk_http_worker')
    child=await original_spawn(*args,**kwargs)
    processes.append(child)
    return child


def endpoint():
    listener=socket.socket();listener.bind(('127.0.0.1',0));listener.listen()
    return listener,'http://127.0.0.1:'+str(listener.getsockname()[1])


async def journey(client,listener):
    requests=[];handlers=set()
    async def respond(reader,writer):
        task=asyncio.current_task();handlers.add(task)
        try:
            lines=(await reader.readuntil(b'\r\n\r\n')).decode('ascii').split('\r\n')
            headers={k.lower():v.strip() for k,v in (line.split(':',1) for line in lines[1:] if line)}
            assert lines[0]=='POST /v2/commands HTTP/1.1'
            assert headers['authorization']=='Bearer '+credential
            value=json.loads(await reader.readexactly(int(headers['content-length'])))
            requests.append(value)
            frame={'v':2,'id':'ack-'+str(len(requests)),'ts':'2026-10-28T00:00:00Z','type':'ack','payload':{},'in_reply_to':value['id'],
                   **{k:value[k] for k in ('cid','operation_id') if k in value}}
            raw=json.dumps({'profile':'connect/2','request_id':value['id'],'envelopes':[frame]},separators=(',',':')).encode()
            writer.write(b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: '+str(len(raw)).encode()+b'\r\nConnection: close\r\n\r\n'+raw)
            await writer.drain()
        finally:
            writer.close();await writer.wait_closed();handlers.discard(task)
    server=await asyncio.start_server(respond,sock=listener)
    try:
        scope=HttpCommandScope(ConversationId('room'),operation_id=OperationId('original'))
        payload={'msg_id':'message','to':['participant'],'content_type':'text/plain','body':'synthetic','body_encoding':'utf8','label':'connect-messages','attachments':[],'in_reply_to_msg':None}
        for _ in range(2):
            frames=await client.request('message.send',payload,scope,options)
            assert frames[-1].value['type']=='ack'
            exported=frames[-1].to_json();exported['payload']['private']='changed'
            assert 'private' not in frames[-1].value['payload']
        await client.request('operation.get',{'operation_id':'original'},HttpCommandScope(ConversationId('room')),options)
        assert requests[0]['operation_id']==requests[1]['operation_id']=='original'
        assert requests[0]['payload']==requests[1]['payload']==payload
        assert len({r['id'] for r in requests})==3
    finally:
        server.close();await server.wait_closed()
        if handlers:await asyncio.wait_for(asyncio.gather(*tuple(handlers)),4)


async def asynchronous():
    listener,origin=endpoint()
    async with CanonicalHttpClient(origin,credential) as client:
        await journey(client,listener)


with patch('asyncio.create_subprocess_exec',side_effect=capture):
    asyncio.run(asynchronous())
    listener,origin=endpoint()
    with SynchronousSdk(CanonicalHttpClient(origin,credential)) as sdk:
        sdk.run(lambda client:journey(client,listener))
assert len(processes)==6
for child in processes:
    assert child.returncode is not None
    try:os.kill(child.pid,0)
    except ProcessLookupError:pass
    else:raise AssertionError('Original worker still exists')
print(json.dumps({'profile':'connect-python-installed-http/1','passed':True,'distribution_version':'0.1.0','python_version':sys.version.split()[0],
                  'isolated_installed_import':True,'typed_marker_installed':True,'async_and_sync_control':True,'fresh_physical_same_original_id':True,
                  'owned_workers_joined':6,'requests':6,'native_server_qualified':False,'full_sdk_qualified':False}))
