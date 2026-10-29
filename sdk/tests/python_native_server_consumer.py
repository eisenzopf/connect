"""Installed wheel, original signed Rust helper/listener, explicit restart only."""
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import sys

import connect_sdk
from connect_sdk import (Client, ConversationId, MessageId, MutationOptions,
                        NonEmpty, OperationId, ParticipantId, RequestOptions, SdkError)

spec=json.loads(Path(sys.argv[1]).read_text())
assert Path.cwd().resolve()==Path(spec['project']).resolve()
assert Path(connect_sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert importlib.metadata.version('connect-sdk')=='0.1.0'
assert 'PYTHONPATH' not in os.environ
options=RequestOptions(4000)
mutation=lambda id:MutationOptions(OperationId(id),options)
cid=ConversationId(spec['cid'])
recipient=NonEmpty(ParticipantId(spec['recipient']))
checkpoint='initial'


def emit(value):
    print(json.dumps(value,separators=(',',':')),flush=True)


async def command(expected):
    # Keep the input reader owned by this process and bound every host phase.
    line=await asyncio.wait_for(reader.readline(),15)
    assert line==expected.encode()+b'\n'


def client():
    return Client(spec['config'])


async def exchange(owner,kind,payload):
    envelope={'v':2,'id':'python-native-physical','ts':'2026-10-28T00:00:00Z',
              'type':kind,'cid':cid,'payload':payload}
    frames=await owner.raw.request(owner.raw.validate(envelope),options)
    assert all('signature' in frame.value for frame in frames)
    return [frame.to_json() for frame in frames]


async def journal(owner,after):
    frames=await exchange(owner,'conversation.subscribe',{'after':after,'live':False,'limit':63})
    facts=[v['payload']['data'] for v in frames if v['type']=='conversation.event']
    selected=[f for f in facts if f.get('operation_id')=='python-native-send'
              and f['data']['kind']=='message' and f['data']['data']['id']=='python-native-message']
    assert len(selected)==1
    fact=selected[0]
    assert fact['conversation_id']==cid
    following=await exchange(owner,'conversation.subscribe',{'after':fact['cursor'],'live':False,'limit':63})
    assert all(v.get('payload',{}).get('event_id')!=fact['event_id'] for v in following)
    return hashlib.sha256(fact['event_id'].encode()).hexdigest()


async def send(owner,text='synthetic installed Python native message',*,forbidden=False):
    room=owner.conversations.by_id(cid)
    return await room.send(MessageId('python-native-forbidden-message' if forbidden else 'python-native-message'),
        recipient,text,mutation('python-native-forbidden-send' if forbidden else 'python-native-send'))


async def run():
    global checkpoint,reader
    reader=asyncio.StreamReader(limit=4096)
    protocol=asyncio.StreamReaderProtocol(reader)
    transport,_=await asyncio.get_running_loop().connect_read_pipe(lambda:protocol,sys.stdin)
    try:
        async with client() as first:
            capabilities=await first.capabilities.refresh(options)
            assert capabilities.runtime_role=='python' and capabilities.selected_transport=='quic'
            try:
                await first.capabilities.require('fixture-unsupported-capability',options)
            except SdkError as error:
                assert (error.kind,error.certainty)==('unsupported','read-only')
            else:
                raise AssertionError('Unsupported capability accepted')
            snapshot=await exchange(first,'conversation.get',{})
            cursor=next(v['payload']['cursor'] for v in snapshot if v['type']=='conversation.snapshot')
            operation=await send(first)
            saved=operation.reference_json()
            complete=await operation.wait(options)
            assert complete.value.message_id=='python-native-message'
            record=complete.snapshot.record_id
            assert record is not None
            again=await (await send(first)).wait(options)
            assert again.snapshot.record_id==record
            try:
                await send(first,'changed intent')
            except SdkError as error:
                assert (error.kind,error.certainty)==('conflict','refused')
                assert error.operation.operation_id=='python-native-send'
            else:
                raise AssertionError('Changed intent accepted')
            event_hash=await journal(first,cursor)
            # Persist only the public reference/cursor; reconstruction grants no IO.
            Path(spec['saved']).write_text(json.dumps({'reference':saved,'cursor':cursor,'record':record,'event':event_hash}))
        emit({'phase':'stopped','record_id':record,'event_id_sha256':event_hash,
              'same_id_replay':True,'changed_intent_refused':True,'signed_finite_journal':True,'typed_capability_refusal':True})
        checkpoint='restart'
        await command('restart')
        saved=json.loads(Path(spec['saved']).read_text())
        async with client() as fresh:
            restored=fresh.operations.restore(saved['reference'],fresh.operation_kinds.message_send)
            recovered=await restored.wait(options)
            assert recovered.snapshot.record_id==saved['record']
            assert recovered.value.message_id=='python-native-message'
            assert (await (await send(fresh)).wait(options)).snapshot.record_id==saved['record']
            assert await journal(fresh,saved['cursor'])==saved['event']
        emit({'phase':'resumed','record_id':saved['record'],'event_id_sha256':saved['event'],
              'original_reference_recovered':True,'same_cause_after_restart':True})
        checkpoint='revocation'
        await command('revoked')
        for action in ('read','replay','new'):
            async with client() as refused:
                try:
                    if action=='read':
                        original=refused.operations.restore(saved['reference'],refused.operation_kinds.message_send)
                        await original.status(options)
                    else:
                        await send(refused,forbidden=action=='new')
                except SdkError as error:
                    assert error.kind=='authentication'
                    assert error.certainty==('read-only' if action=='read' else 'not-submitted')
                else:
                    raise AssertionError('Revoked authority accepted')
        emit({'phase':'completed','current_read_refused':True,'current_replay_refused':True,'current_new_mutation_refused':True})
    finally:
        transport.close()


try:
    asyncio.run(run())
except Exception as error:
    emit({'phase':'failed','checkpoint':checkpoint,'kind':error.kind if isinstance(error,SdkError) else 'assertion' if isinstance(error,AssertionError) else 'unexpected',
          'certainty':error.certainty if isinstance(error,SdkError) else None})
    raise
