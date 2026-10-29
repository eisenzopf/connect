"""Installed public Python controller; independent fixture peers own native media."""
import asyncio
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import traceback

import connect_sdk
from connect_sdk import (Client, ConnectionId, ConversationId, ConversationRevision,
    ConversationRole, CandidateTarget, EndpointId, EndpointTarget, HandoffInput,
    MoveInput, MutationOptions, OperationId, ParticipantId, RequestOptions,
    SessionId, SessionRole, SdkError, TopologyRevision)

spec=json.loads(Path(sys.argv[1]).read_text())
assert Path.cwd().resolve()==Path(spec['project']).resolve()
assert Path(connect_sdk.__file__).resolve().is_relative_to(Path(sys.prefix).resolve())
assert importlib.metadata.version('connect-sdk')=='0.1.0'
assert 'PYTHONPATH' not in os.environ
options=RequestOptions(4000)
clients=[]
checkpoint='register'


def emit(value):
    print(json.dumps(value,separators=(',',':')),flush=True)


def open_client():
    # The fixture installs one matching trusted connect on its private PATH.
    client=Client(spec['config']);clients.append(client);return client


def topology(value):
    return dict(retained=[c.id for c in value.retained],admitted=[c.id for c in value.admitted],
        retired=list(value.retired),topology_revision=value.topology_revision,
        conversation_revision=value.conversation_revision)


def summary(value):
    assert all(getattr(c,'native_negotiation',None) is None for c in (*value.retained,*value.admitted))
    if spec['mode']=='handoff':
        r=value.role_change;assert r is not None
        return dict(topology=topology(value),role_change=dict(participant_revision=r.participant_revision,
            role_scope=r.role_scope,outgoing=r.outgoing,source_participant=r.source_participant,
            target_participant=r.target_participant,cleanup=r.cleanup))
    assert getattr(value.connection,'native_negotiation',None) is None
    b=value.binding
    return dict(topology=topology(value),connection_id=value.connection.id,native_owned=False,
        binding=dict(generation=b.generation,participant_id=b.participant_id,membership_epoch=b.membership_epoch,
            endpoint_id=b.endpoint_id,device_id=b.device_id))


async def stopped(operation,state):
    assert (await operation.status(options)).state==state
    try:await operation.wait(options)
    except SdkError as error:
        assert error.certainty=='recorded' and error.observation.state==state
    else:raise AssertionError('Stopped operation claimed completion')


async def run():
    global checkpoint
    reader=asyncio.StreamReader(limit=4096)
    transport,_=await asyncio.get_running_loop().connect_read_pipe(
        lambda:asyncio.StreamReaderProtocol(reader),sys.stdin)
    try:
        client=open_client()
        if spec['mode']=='move-recovery':
            checkpoint='recover';kind=client.operation_kinds.connection_replace
            operation=client.operations.restore(spec['detail']['reference'],kind)
            await stopped(operation,spec['detail']['state'])
            emit(dict(phase='stopped',state=spec['detail']['state'],certainty='recorded'))
        else:
            assert spec['mode'] in ('handoff','move')
            await client.conversations.by_id(ConversationId(spec['detail']['command']['conversation_id'])).refresh(options)
            emit(dict(phase='registered'))
        while line:=await reader.readline():
            assert len(line)<=4096
            request=json.loads(line);checkpoint=request['action']
            if checkpoint=='begin':
                c=spec['detail']['command'];i=c['intent']
                session=client.conversations.by_id(ConversationId(c['conversation_id'])).sessions.by_id(SessionId(c['session_id']))
                extensions=dict(c.get('payload_extensions',{}))
                if spec['mode']=='handoff':
                    assert i['type']=='session.handoff'
                    assert extensions.pop('consultation_record_id')==spec['detail']['consultation']['continuation']['record_id']
                    kind=client.operation_kinds.session_handoff
                    original=client.operations.restore(spec['detail']['consultation'],client.operation_kinds.session_consult)
                    role=SessionRole() if i['role_scope']=='session' else ConversationRole(ConversationRevision(i['expected_conversation_revision']))
                    operation=await session.handoff(HandoffInput(
                        via=ConnectionId(c['connection_id']),from_participant=ParticipantId(i['source_participant']),to=ParticipantId(i['target_participant']),
                        retain=ConnectionId(i['retained_connid']),outgoing=i['outgoing'],role=role,
                        expected_topology_revision=TopologyRevision(c['expected_revision']),
                        candidate=None if i.get('candidate_connid') is None else ConnectionId(i.get('candidate_connid')),consultation=original),
                        MutationOptions(OperationId(c['operation_id']),options,extensions))
                else:
                    assert i['type']=='connection.replace'
                    assert (i.get('endpoint_id') is None)!=(i.get('candidate_connid') is None)
                    kind=client.operation_kinds.connection_replace
                    target=CandidateTarget(ConnectionId(i.get('candidate_connid'))) if i.get('endpoint_id') is None else EndpointTarget(EndpointId(i.get('endpoint_id')))
                    operation=await session.connections.by_id(ConnectionId(c['connection_id'])).replace_device(
                        MoveInput(ConnectionId(i['retained_connid']),target,TopologyRevision(c['expected_revision'])),
                        MutationOptions(OperationId(c['operation_id']),options,extensions))
                    assert (await operation.status(options)).state!='completed'
                saved=operation.reference_json();key=operation.reference
                assert key.operation_id==c['operation_id']
                assert (key.scope.conversation_id,key.scope.session_id,key.scope.connection_id)==(c['conversation_id'],c['session_id'],c['connection_id'])
                emit(dict(phase='accepted',reference=saved))
            elif checkpoint=='completed':
                completion=await operation.wait(options);value=summary(completion.value)
                fresh=open_client();restored=fresh.operations.restore(saved,kind)
                replay=await restored.wait(options)
                assert replay.snapshot.record_id==completion.snapshot.record_id and summary(replay.value)==value
                await fresh.close(options)
                emit(dict(phase='completed',recovered=True,summary=value))
            elif checkpoint=='stopped':
                if request['original_revoked']:
                    try:await operation.status(options)
                    except SdkError as error:
                        assert error.certainty in ('read-only','refused') and error.kind in ('cancelled','authentication','transport-uncertain','application')
                    else:raise AssertionError('Revoked operation read accepted')
                else:await stopped(operation,request['state'])
                emit(dict(phase='stopped',state=request['state'],original_revoked=request['original_revoked']))
            else:
                assert checkpoint=='close'
                for active in clients:await active.close(options)
                pids=[int(v) for v in Path(spec['pid_file']).read_text().splitlines() if v]
                assert len(pids)==len(set(pids))
                for pid in pids:
                    assert pid>0
                    try:os.kill(pid,0)
                    except ProcessLookupError:pass
                    else:raise AssertionError('Owned helper not joined')
                emit(dict(phase='closed',helpers_joined=len(pids)));break
    except Exception as error:
        known=checkpoint if checkpoint in ('register','recover','begin','completed','stopped','close') else 'invalid'
        frames=[f for f in traceback.extract_tb(error.__traceback__) if Path(f.filename).name=='movement-consume.py']
        emit(dict(phase='failed',checkpoint=known,kind=error.kind if isinstance(error,SdkError) else 'consumer-assertion',
            certainty=error.certainty if isinstance(error,SdkError) else None,
            line=frames[-1].lineno if frames else None,error_type=type(error).__name__ if not isinstance(error,SdkError) else None))
        raise
    finally:
        for active in clients:await active.close(options)
        transport.close()


asyncio.run(run())
