"""Installed public Python controls; fixture peers retain physical media custody."""
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import traceback

import connect_sdk
from connect_sdk import (Client, CanonicalHttpClient, HttpCommandScope, ConsultInput,
                        ConversationId, SessionId, ConnectionId, ParticipantId, NonEmpty,
                        TopologyRevision, OperationId, MutationOptions, RequestOptions, SdkError)

spec=json.loads(Path(sys.argv[1]).read_text())
assert Path.cwd().resolve()==Path(spec['project']).resolve()
assert Path(connect_sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert importlib.metadata.version('connect-sdk')=='0.1.0'
assert 'PYTHONPATH' not in os.environ
options=RequestOptions(4000)
clients=[]
workers=[]
original_spawn=asyncio.create_subprocess_exec
checkpoint='submit'


async def observe_spawn(*args,**kwargs):
    child=await original_spawn(*args,**kwargs)
    if args==(sys.executable,'-m','connect_sdk._sdk_http_worker'):
        workers.append(child)
    return child


def emit(value):
    print(json.dumps(value,separators=(',',':')),flush=True)


def opened():
    client=Client(spec['config'])
    clients.append(client)
    return client


def joined():
    pids=[int(line) for line in Path(spec['pid_file']).read_text().splitlines()]
    assert len(set(pids))==len(pids)
    for pid in pids+[worker.pid for worker in workers]:
        try:
            os.kill(pid,0)
        except ProcessLookupError:
            pass
        else:
            raise AssertionError('Original helper or HTTP worker still exists')
    assert all(worker.returncode is not None for worker in workers)
    return len(pids)


async def run():
    global checkpoint
    reader=asyncio.StreamReader(limit=8192)
    protocol=asyncio.StreamReaderProtocol(reader)
    transport,_=await asyncio.get_running_loop().connect_read_pipe(lambda:protocol,sys.stdin)
    try:
        async with CanonicalHttpClient(spec['http'],spec['credential']) as http:
            client=opened()
            d=spec['detail']
            if spec['mode']=='consult':
                session=client.conversations.by_id(ConversationId(d['cid'])).sessions.by_id(SessionId(d['sid']))
                operation=await session.consult(ConsultInput(ConnectionId(d['via']),ConnectionId(d['retain']),
                    NonEmpty(ParticipantId(d['participants'][0]),tuple(ParticipantId(p) for p in d['participants'][1:])),
                    d['hold'],TopologyRevision(d['revision'])),MutationOptions(OperationId(d['operation_id']),options))
                saved=operation.reference_json()
                snapshot=await operation.status(options)
                assert snapshot.state!='completed' and snapshot.record_id is not None
                record=snapshot.record_id
                assert operation.reference.scope.conversation_id==d['cid']
                assert operation.reference.scope.session_id==d['sid']
                assert operation.reference.scope.connection_id==d['via']
                emit({'phase':'submitted','reference':saved})
            else:
                assert spec['mode']=='return'
                original=client.operations.restore(d['reference'],client.operation_kinds.session_consult)
                checkpoint='return-submit'
                operation=await original.return_to_retained(TopologyRevision(d['revision']),
                    MutationOptions(OperationId('return-private-native'),options))
                saved=operation.reference_json()
                assert saved['continuation']['retained_connection_id']==d['reference']['continuation']['retained_connection_id']
                assert saved['continuation']['consultation_record_id']==d['reference']['continuation']['record_id']
                checkpoint='return-wait'
                complete=await operation.wait(options)
                assert complete.value.id==original.reference.scope.session_id and complete.snapshot.record_id is not None
                record=complete.snapshot.record_id
                checkpoint='return-recover'
                recovered=client.operations.restore(saved,client.operation_kinds.session_consult_return)
                assert (await recovered.wait(options)).snapshot.record_id==record
                emit({'phase':'returned','reference':saved,'original_record_recovered':True})
            while True:
                line=await reader.readline()
                assert 0<len(line)<=4096
                request=json.loads(line)
                checkpoint=request['action']
                if checkpoint=='completed':
                    assert spec['mode']=='consult'
                    complete=await operation.wait(options)
                    assert complete.snapshot.record_id==record and complete.value.topology_revision==request['revision']
                    assert complete.value.session.id==d['sid']
                    assert sorted(c.id for c in complete.value.retained)==sorted([d['via'],d['retain']])
                    assert [c.id for c in complete.value.admitted]==['private-consultant']
                    assert complete.value.retired==()
                    assert not hasattr(complete.value.admitted[0],'native_negotiation')
                    try:
                        await operation.return_to_retained(TopologyRevision(request['revision']),
                            MutationOptions(OperationId('python-return-without-grant'),options))
                    except SdkError as error:
                        assert error.certainty=='refused' and error.problem.code==403
                        assert error.operation.operation_id=='python-return-without-grant'
                        assert error.operation.command=='session.consult.return'
                    else:
                        raise AssertionError('Return without current authority accepted')
                    fresh=opened()
                    restored=fresh.operations.restore(saved,fresh.operation_kinds.session_consult)
                    replay=await restored.wait(options)
                    assert replay.snapshot.record_id==record and replay.value.topology_revision==request['revision']
                    assert [c.id for c in replay.value.admitted]==['private-consultant']
                    assert sorted(c.id for c in replay.value.retained)==sorted([d['via'],d['retain']])
                    await fresh.close(options)
                    emit({'phase':'completed','record_id':record,'fresh_observation':True,'return_without_grant_refused':True})
                elif checkpoint=='revoked':
                    try:
                        await operation.status(options)
                    except SdkError as error:
                        assert error.certainty in ('refused','read-only')
                        assert (error.kind in ('authentication','cancelled','transport-uncertain')
                            or error.certainty=='refused' and error.kind=='application')
                    else:
                        raise AssertionError('Revoked native read accepted')
                    scope=operation.reference.scope
                    try:
                        await http.request('operation.get',{'operation_id':operation.reference.operation_id},
                            HttpCommandScope(scope.conversation_id,scope.session_id,scope.connection_id),options)
                    except SdkError as error:
                        assert error.kind=='authentication'
                    else:
                        raise AssertionError('Current independent HTTP authority accepted')
                    emit({'phase':'revoked','current_authority_refused':True,'http_current_authority_refused':True})
                else:
                    assert checkpoint=='close'
                    for active in clients:
                        await active.close(options)
                    await http.close(options)
                    emit({'phase':'closed','mode':spec['mode'],'helpers_joined':joined(),'http_workers_joined':len(workers)})
                    break
    finally:
        for active in clients:
            await active.close(options)
        transport.close()


try:
    asyncio.create_subprocess_exec=observe_spawn
    asyncio.run(run())
except Exception as error:
    local=[frame for frame in traceback.extract_tb(error.__traceback__) if frame.filename==__file__]
    emit({'phase':'failed','checkpoint':checkpoint if checkpoint in ('submit','return-submit','return-wait','return-recover','completed','revoked','close') else 'invalid',
          'kind':error.kind if isinstance(error,SdkError) else 'consumer-assertion',
          'certainty':error.certainty if isinstance(error,SdkError) else None,
          'line':local[-1].lineno if local else None,'error_type':type(error).__name__})
    raise
