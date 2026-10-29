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
                        NonEmpty, OperationId, ParticipantId, RequestOptions, SdkError,
                        IdentityId, SessionId, ConnectionId, StreamId, RoleChange,
                        MediumUpgrade, CodecRenegotiate, OfferedStream, Quality,
                        Mute, HoldConnection, ResumeConnection, ConnectionCodecRenegotiate,
                        ParticipantStreams, ParticipantLeave)

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


async def completed(operation):
    return (await (await operation).wait(options)).value


async def refused(action,kind=None):
    try:
        await action
    except SdkError as error:
        assert error.certainty in ('refused','read-only')
        if kind is not None:
            assert error.kind==kind
    else:
        raise AssertionError('Expected current control refusal')


async def controls(owner):
    """Public installed handles; independent host checks the resulting Store."""
    global checkpoint
    checkpoint='participants'
    room=await completed(owner.conversations.create([IdentityId(spec['peer_identity'])],
        'persistent',mutation('python-control-create')))
    initial=await room.refresh(options)
    peer=next(p for p in initial.participants if p.identity_id==spec['peer_identity'])
    own=next(p for p in initial.participants if p.id!=peer.id)
    member=room.participants.by_id(peer.id)
    await completed(member.remove(initial.revision,mutation('python-control-member-reset')))
    await refused(member.refresh(options),'receipt-unavailable')
    reset=await room.refresh(options)
    added=await room.participants.add(peer.identity_id,peer.kind,'participant',reset.revision,
        mutation('python-control-member-add'))
    saved=added.reference_json()
    member=(await added.wait(options)).value
    assert member.id==peer.id
    epoch=(await member.refresh(options)).membership_epoch
    current=await room.refresh(options)
    await completed(member.update('observer',current.revision,mutation('python-control-member-role')))
    assert (await member.refresh(options)).role=='observer'
    await refused(member.update('participant',reset.revision,mutation('python-control-member-stale')))
    current=await room.refresh(options)
    await completed(member.remove(current.revision,mutation('python-control-member-remove')))
    current=await room.refresh(options)
    member=await completed(room.participants.add(peer.identity_id,peer.kind,'participant',current.revision,
        mutation('python-control-member-return')))
    assert member.id==peer.id and (await member.refresh(options)).membership_epoch>epoch
    await refused(owner.operations.restore(saved,owner.operation_kinds.conversation_participant_add).status(options))
    emit({'phase':'participants-completed','participant_epoch_advanced':True,'stale_revision_refused':True})
    checkpoint='sessions'
    capabilities={'audio':['g.711-mu','g.711-a'],'video':['vp8'],'screen':['vp8'],'data':['reliable-ordered']}
    solo=await completed(room.sessions.invite(SessionId('python-control-solo'),[],'voice',
        'python-control',capabilities,mutation('python-control-solo')))
    assert (await solo.refresh(options)).state=='accepted'
    async def update_session(value,id):
        current=await solo.refresh(options)
        return await completed(solo.update(value,current.topology_revision,mutation(id)))
    await update_session(RoleChange(own.id,'observer'),'python-control-session-role')
    await update_session(RoleChange(own.id,'participant'),'python-control-session-role-return')
    await update_session(MediumUpgrade(NonEmpty('audio',('video',))),'python-control-session-upgrade')
    current=await solo.refresh(options)
    assert 'video' in current.media and current.revision!=current.topology_revision
    await refused(solo.end(1000,'stale topology',current.revision,mutation('python-control-session-stale')))
    await update_session(CodecRenegotiate(NonEmpty('g.711-mu',('vp8',))),'python-control-session-codecs')
    assert (await solo.refresh(options)).codec_preferences==('g.711-mu','vp8')
    async with Client(spec['peer_config']) as other:
        await other.capabilities.require('incoming-invitations',options)
        incoming=other.incoming_invitations(options)
        try:
            hint_wait=asyncio.create_task(anext(incoming))
            try:
                current=await solo.refresh(options)
                await completed(solo.invite_participants(NonEmpty(peer.id),'voice+video','python-control',
                    capabilities,current.topology_revision,mutation('python-control-extend')))
                hint=await hint_wait
            finally:
                if not hint_wait.done():
                    hint_wait.cancel()
                await asyncio.gather(hint_wait,return_exceptions=True)
            assert hint.operation_id!='python-control-extend'
            assert hint.source.correlation_id==hint.operation_id
            assert hint.from_participant==own.id and hint.to_participant==peer.id
            assert hint.medium=='voice+video' and dict(hint.capabilities)=={k:tuple(v) for k,v in capabilities.items()}
            peer_solo=hint.session
            assert peer_solo.id==solo.id and peer_solo.conversation_id==room.id
        finally:
            await incoming.close()
        await other.capabilities.require('incoming-invitations',options)
        current=await peer_solo.refresh(options)
        await completed(peer_solo.accept(capabilities,current.topology_revision,mutation('python-control-accept')))
        emit({'phase':'sessions-ready','incoming_decision_completed':True})
        checkpoint='connections'
        async def offer(session,id,stream,operation):
            return await completed(session.connections.offer(ConnectionId(id),NonEmpty(OfferedStream(
                StreamId(stream),'audio','sendrecv',NonEmpty('g.711-mu',('g.711-a',)))),mutation(operation)))
        native=await offer(solo,'python-control-native','python-control-audio','python-control-offer')
        remote=await offer(peer_solo,'python-control-peer','python-control-peer-audio','python-control-peer-offer')
        assert native.receipt_stream_revision>0 and (await native.refresh(options)).stream_revision is None
        audio=native.streams.by_id(StreamId('python-control-audio'))
        assert (await audio.refresh(options)).ownership=='observation'
        revision=native.receipt_stream_revision
        async def update_connection(value,id):
            nonlocal revision
            receipt=await completed(native.update(value,revision,mutation(id)))
            assert receipt.connection.id==native.id and receipt.connection_generation>0
            revision=receipt.stream_revision
            return receipt.outcome
        quality=await update_connection(Quality(250),'python-control-quality')
        assert quality.interval_ms==250
        muted=await update_connection(Mute('mute',NonEmpty(audio.id)),'python-control-mute')
        assert all(s.source_muted for s in muted.streams)
        await update_connection(Mute('unmute',NonEmpty(audio.id)),'python-control-unmute')
        held=await update_connection(HoldConnection('inactive',NonEmpty(audio.id)),'python-control-hold')
        assert all(s.state=='paused' for s in held.streams) and (await audio.refresh(options)).state=='paused'
        await update_connection(ResumeConnection(NonEmpty(audio.id)),'python-control-resume')
        codecs=await update_connection(ConnectionCodecRenegotiate(NonEmpty('g.711-a'),NonEmpty(audio.id)),
            'python-control-codecs')
        assert all(s.parameters.codec for s in codecs.streams)
        selected=await completed(native.streams.subscribe(NonEmpty(ParticipantStreams(peer.id,NonEmpty('audio'))),
            revision,mutation('python-control-subscribe')))
        assert selected.connection.id==native.id
        unselected=await completed(native.streams.unsubscribe(NonEmpty(StreamId('python-control-peer-audio')),
            selected.stream_revision,mutation('python-control-unsubscribe')))
        await completed(native.end(1000,'complete',unselected.stream_revision,mutation('python-control-native-end')))
        await completed(remote.end(1000,'complete',None,mutation('python-control-peer-end')))
        emit({'phase':'native-completed','native_controls_completed':True,'stream_selection_completed':True})
        checkpoint='session-decisions'
        await update_session(ParticipantLeave(peer.id),'python-control-peer-leave')
        current=await solo.refresh(options)
        await completed(solo.end(1000,'complete',current.topology_revision,mutation('python-control-session-end')))
        assert (await solo.refresh(options)).state=='ended'
        for decision in ('reject','cancel'):
            pending=await completed(room.sessions.invite(SessionId('python-control-'+decision),[peer.id],
                'voice','python-control',capabilities,mutation('python-control-'+decision+'-invite')))
            deciding=other.conversations.by_id(room.id).sessions.by_id(pending.id) if decision=='reject' else pending
            current=await deciding.refresh(options)
            result=await completed((deciding.reject if decision=='reject' else deciding.cancel_invitation)(
                486 if decision=='reject' else 487,decision,current.topology_revision,mutation('python-control-'+decision)))
            assert result.state==('rejected' if decision=='reject' else 'cancelled') and result.session.id==pending.id
    emit({'phase':'controls-completed','conversation_id':room.id,
          'incoming_source_sha256':hashlib.sha256(hint.operation_id.encode()).hexdigest(),
          'participant_epoch_advanced':True,'stale_revisions_refused':True,'incoming_original_hint':True,
          'session_decisions_completed':True,'native_controls_completed':True,'stream_selection_completed':True})


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
            await controls(first)
            checkpoint='message-events'
            snapshot=await exchange(first,'conversation.get',{})
            cursor=next(v['payload']['cursor'] for v in snapshot if v['type']=='conversation.snapshot')
            events=first.conversations.by_id(cid).events(None,options)
            await events.ready()
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
            try:
                for _ in range(256):
                    fact=await anext(events)
                    if fact.operation_id=='python-native-send' and fact.data.get('kind')=='message':
                        assert fact.data['data']['id']=='python-native-message'
                        assert fact.conversation_id==cid
                        break
                else:
                    raise AssertionError('Typed message event missing')
            finally:
                await events.close()
            event_hash=await journal(first,cursor)
            assert event_hash==hashlib.sha256(fact.event_id.encode()).hexdigest()
            # Persist only the public reference/cursor; reconstruction grants no IO.
            Path(spec['saved']).write_text(json.dumps({'reference':saved,'cursor':cursor,'record':record,'event':event_hash}))
        emit({'phase':'stopped','record_id':record,'event_id_sha256':event_hash,
              'same_id_replay':True,'changed_intent_refused':True,'signed_finite_journal':True,
              'typed_message_event':True,'typed_capability_refusal':True})
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
    import traceback
    frames=traceback.extract_tb(error.__traceback__)
    local=[frame for frame in frames if frame.filename==__file__]
    emit({'phase':'failed','checkpoint':checkpoint,'kind':error.kind if isinstance(error,SdkError) else 'assertion' if isinstance(error,AssertionError) else 'unexpected',
          'certainty':error.certainty if isinstance(error,SdkError) else None,
          'source_line':local[-1].lineno if local else None,'error_type':type(error).__name__})
    raise
