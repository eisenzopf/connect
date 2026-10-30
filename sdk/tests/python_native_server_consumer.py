"""Installed wheel, original signed Rust helper/listener, explicit restart only."""
import asyncio
import hashlib
import importlib.metadata
import json
import os
from pathlib import Path
import sys
import struct
import time

import connect_sdk
from connect_sdk import (Client, ConversationId, MessageId, MutationOptions,
                        NonEmpty, OperationId, ParticipantId, RequestOptions, SdkError,
                        IdentityId, SessionId, ConnectionId, StreamId, RoleChange,
                        MediumUpgrade, CodecRenegotiate, OfferedStream, Quality,
                        Mute, HoldConnection, ResumeConnection, ConnectionCodecRenegotiate,
                        ParticipantStreams, ParticipantLeave, Cancellation,
                        CanonicalHttpClient, HttpCommandScope, SynchronousSdk, RtpPacket, SenderClockReport, StreamTarget, TextData, JsonData, BinaryData)

sys.path.insert(0,str(Path(__file__).parent))
from packet_codec import load_media, exchange_media

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
http_workers=[]
original_spawn=asyncio.create_subprocess_exec


async def observe_spawn(*args,**kwargs):
    child=await original_spawn(*args,**kwargs)
    if args==(sys.executable,'-m','connect_sdk._sdk_http_worker'):
        http_workers.append(child)
    return child


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


async def audio_video(owner,other,session,peer_session):
    global checkpoint
    checkpoint='media-av'
    async def offer(where,side):
        return await completed(where.connections.offer(ConnectionId('python-av-'+side),NonEmpty(
            OfferedStream(StreamId('python-av-'+side+'-audio'),'audio','sendrecv',NonEmpty('g.711-mu')),
            (OfferedStream(StreamId('python-av-'+side+'-video'),'video','sendrecv',NonEmpty('vp8')),)),mutation('python-av-'+side+'-offer')))
    source=await offer(session,'source');target=await offer(peer_session,'target')
    for connection,remote in ((source,'target'),(target,'source')):
        await completed(connection.streams.subscribe(NonEmpty(StreamTarget(StreamId('python-av-'+remote+'-audio')),(StreamTarget(StreamId('python-av-'+remote+'-video')),)),connection.receipt_stream_revision,mutation('python-av-'+str(connection.id)+'-select')))
    async def capture(connection,side,remote,ssrc):
        fixture=Path(spec['av_fixture_dir'])/('python-'+side)
        inputs=load_media(str(fixture)+'.mulaw',str(fixture)+'.ivf')
        return await exchange_media(connection.streams.by_id(StreamId('python-av-'+side+'-audio')).media,
            connection.streams.by_id(StreamId('python-av-'+side+'-video')).media,
            connection.streams.by_id(StreamId('python-av-'+side+'-audio')).media,
            connection.streams.by_id(StreamId('python-av-'+remote+'-video')).media,
            inputs,Path(spec['saved']).with_name('python-av-'+side),ssrc)
    results=await asyncio.gather(capture(source,'source','target',0x50410001),capture(target,'target','source',0x50420001),return_exceptions=True)
    for result in results:
        if isinstance(result,BaseException):raise result
        assert result['audio_bytes']==4800 and result['video_frames']==6
    for connection in (source,target):await completed(connection.end(1000,'complete',None,mutation(str(connection.id)+'-end')))
    emit({'phase':'av-completed','native_av_utility_capture':True})


async def reliable_data(owner,other,session,peer_session,own,peer):
    global checkpoint
    # Caller-owned intent is saved before each publication. This is synthetic
    # media/private fixture state, never a reusable grant or a delivery receipt.
    capabilities={'audio':['g.711-mu'],'video':['vp8'],'screen':['vp8'],'data':['reliable-ordered']}
    checkpoint='data-invite'
    session=await completed(owner.conversations.by_id(session.conversation_id).sessions.invite(
        SessionId('python-data-session'),[peer.id],'mixed','synthetic reliable data',capabilities,mutation('python-data-invite')))
    peer_session=other.conversations.by_id(session.conversation_id).sessions.by_id(session.id)
    current=await peer_session.refresh(options)
    checkpoint='data-accept'
    await completed(peer_session.accept(capabilities,current.topology_revision,mutation('python-data-accept')))
    async with Client(spec['config']) as recovery:
        for format in ('text','json','binary'):
            if format=='text':payload=TextData('🦀'+'x'*65532);raw=payload.value.encode('utf-8')
            elif format=='json':
                payload=JsonData({'data':'x'*(65536-11)})
                raw=json.dumps({'data':'x'*(65536-11)},separators=(',',':')).encode()
            else:payload=BinaryData(bytes(range(256))*256);raw=payload.value
            assert len(raw)==65536
            source_id=StreamId('python-data-'+format+'-source-stream')
            target_id=StreamId('python-data-'+format+'-target-stream')
            async def offer(where,side):
                return await completed(where.connections.offer(ConnectionId('python-data-'+format+'-'+side),
                    NonEmpty(OfferedStream(source_id if side=='source' else target_id,'data','sendrecv',NonEmpty(format))),mutation('python-data-'+format+'-'+side+'-offer')))
            checkpoint='data-offer'
            source=await offer(session,'source');target=await offer(peer_session,'target')
            checkpoint='data-subscribe'
            for connection,remote_id in ((source,target_id),(target,source_id)):
                await completed(connection.streams.subscribe(NonEmpty(StreamTarget(remote_id)),
                    connection.receipt_stream_revision,mutation('python-data-'+format+'-'+str(connection.id)+'-select')))
            sending=source.streams.by_id(source_id).media;receiving=target.streams.by_id(source_id).media
            return_sender=target.streams.by_id(target_id).media;return_receiver=source.streams.by_id(target_id).media
            intent=Path(spec['saved']).with_name('python-data-'+format+'-intent.json')
            intent.with_suffix('.bin').write_bytes(raw)
            intent.write_text(json.dumps({'format':format,'message_id':'python-data-'+format+'-message',
                'operation_id':'python-data-'+format+'-send','recipient':peer.id,'connection_id':source.id}))
            message=MessageId('python-data-'+format+'-message');original=mutation('python-data-'+format+'-send')
            checkpoint='data-send'
            operation=await sending.send_data(message,NonEmpty(peer.id),payload,original)
            completion=await operation.wait(options)
            assert completion.value.boundary=='core-queue-admission' and completion.value.message_id==message
            saved=operation.reference_json()
            intent.with_suffix('.reference.json').write_text(json.dumps(saved))
            checkpoint='data-receive'
            delivered=await receiving.receive_data(options)
            assert (delivered.message_id,delivered.operation_id,delivered.from_participant,delivered.payload)==(message,original.operation_id,own.id,payload)
            checkpoint='data-recovery'
            observed=await recovery.operations.restore(saved,recovery.operation_kinds.data_send).wait(options)
            assert observed.snapshot.record_id==completion.snapshot.record_id and observed.value==completion.value
            try:await recovery.media.connection(session.conversation_id,session.id,source.id).refresh(options)
            except SdkError as error:assert (error.kind,error.certainty)==('unsupported','read-only')
            else:raise AssertionError('Saved metadata acquired foreign native custody')
            checkpoint='data-replay'
            replay=await completed(sending.send_data(message,NonEmpty(peer.id),payload,original))
            assert replay==completion.value
            try:await receiving.receive_data(RequestOptions(80))
            except SdkError as error:assert (error.kind,error.certainty)==('deadline','read-only')
            else:raise AssertionError('Original replay published a duplicate')
            try:await sending.send_data(MessageId('python-data-'+format+'-changed'),NonEmpty(peer.id),payload,original)
            except SdkError as error:
                assert (error.kind,error.certainty,error.operation.operation_id)==('authentication','refused',original.operation_id)
            else:raise AssertionError('Changed original intent accepted')
            checkpoint='data-return'
            reverse=mutation('python-data-'+format+'-return')
            intent.with_suffix('.return.json').write_text(json.dumps({'message_id':'python-data-'+format+'-return-message',
                'operation_id':reverse.operation_id,'recipient':own.id,'connection_id':target.id}))
            await completed(return_sender.send_data(MessageId('python-data-'+format+'-return-message'),NonEmpty(own.id),payload,reverse))
            returned=await return_receiver.receive_data(options)
            assert (returned.operation_id,returned.from_participant,returned.payload)==(reverse.operation_id,peer.id,payload)
            checkpoint='data-end'
            for connection in (source,target):
                await completed(connection.end(1000,'complete',None,mutation('python-data-'+str(connection.id)+'-end')))
        assert (await owner.media.data_stats(options)).queued_bytes==0
        assert (await other.media.data_stats(options)).queued_bytes==0
    current=await session.refresh(options)
    await completed(session.end(1000,'complete',current.topology_revision,mutation('python-data-session-end')))
    emit({'phase':'data-completed','native_data_typed_round_trip':True,'native_data_full_64k':True,
          'native_data_original_recovery':True,'native_data_one_effect_replay':True,
          'native_data_changed_intent_refused':True,'native_data_foreign_custody_refused':True})


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
        assert native.receipt_stream_revision>0 and (await native.refresh(options)).stream_revision==native.receipt_stream_revision
        audio=native.streams.by_id(StreamId('python-control-audio'))
        selected_media=await native.media.refresh(options)
        assert selected_media.binding.connection_id==native.id
        assert [s.id for s in selected_media.streams]==['python-control-audio']
        assert selected_media.streams[0].stream_local_id is not None
        native_stream=native.streams.by_id(StreamId('python-control-audio')).media
        pcmu=bytes(0x80 if n%16<8 else 0 for n in range(160))
        original_packet=RtpPacket(1,struct.pack('!BBHII',0x80,0,1,160,0x70594d31)+pcmu)
        admitted=await native_stream.send_rtp(original_packet,options)
        assert admitted.boundary=='local-media-queue' and admitted.datagram_sequence==1
        seconds=time.time()+2208988800
        ntp=(int(seconds)<<32)+int((seconds-int(seconds))*2**32)
        report=SenderClockReport(0x70594d31,ntp,160,1,160,'synthetic-python-media')
        if selected_media.source_clock:
            clock=await native_stream.send_clock(2,report,options)
            assert clock.boundary=='local-media-queue' and clock.datagram_sequence==2
        else:
            try:
                await native_stream.send_clock(2,report,options)
            except SdkError as error:
                assert (error.kind,error.certainty)==('unsupported','not-submitted')
            else:
                raise AssertionError('Unnegotiated clock accepted')
        media_stats=await owner.media.stats(options)
        assert media_stats.queued_packets<=256 and media_stats.queued_bytes<=4*1024*1024
        await audio_video(owner,other,solo,peer_solo)
        await reliable_data(owner,other,solo,peer_solo,own,peer)
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
          'session_decisions_completed':True,'native_controls_completed':True,'stream_selection_completed':True,
          'native_media_negotiation':True,'native_rtp_local_queue':True,'native_clock_local_queue':selected_media.source_clock,
          'native_clock_unnegotiated_refused':not selected_media.source_clock})


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
            assert (await operation.wait_until_ready(options)).state=='completed'
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
            checkpoint='operation-decisions'
            local=Cancellation()
            local.cancel()
            try:
                await operation.wait(RequestOptions(4000,local))
            except SdkError as error:
                assert (error.kind,error.certainty)==('cancelled','read-only')
            else:
                raise AssertionError('Cancelled local wait accepted')
            decision=await completed(operation.cancel(mutation('python-native-cancel')))
            assert decision.target=='python-native-send' and decision.decision.operation_id=='python-native-cancel'
            await refused(operation.reconcile('completed','python-native-invented-evidence',
                mutation('python-native-unattributed-reconcile')),'authentication')
            assert (await operation.status(options)).record_id==record
            assert (await operation.status(options)).state=='completed'
            async with CanonicalHttpClient(spec['http'],spec['credential']) as http:
                frames=await http.request('operation.get',{'operation_id':'python-native-send'},HttpCommandScope(cid),options)
                durable=next(f.value['payload'] for f in frames if f.value['type']=='operation.updated')
                assert durable['record_id']==record and durable['state']=='completed'
                history=await http.request('message.history',{'limit':63},HttpCommandScope(cid),options)
                assert len([f for f in history if f.value['type']=='message.send'
                    and f.value['payload']['msg_id']=='python-native-message'])==1
            queued=await first.conversations.by_id(cid).receipts(MessageId('python-native-message'),options)
            assert len(queued)==1 and queued[0].state=='queued' and queued[0].provenance=='journal'
            # Persist only the public reference/cursor; reconstruction grants no IO.
            Path(spec['saved']).write_text(json.dumps({'reference':saved,'cursor':cursor,'record':record,'event':event_hash}))
        emit({'phase':'stopped','record_id':record,'event_id_sha256':event_hash,
              'same_id_replay':True,'changed_intent_refused':True,'signed_finite_journal':True,
              'typed_message_event':True,'typed_capability_refusal':True,'local_wait_cancelled_read_only':True,
              'durable_cancel_original_preserved':True,'unattributed_reconciliation_refused':True,
              'canonical_http_same_original_record':True,'receipt_queued_without_delivery':True})
        checkpoint='receipts'
        await command('receipts')
        async with client() as receipts:
            async def observed(expected):
                facts=await receipts.conversations.by_id(cid).receipts(MessageId('python-native-message'),options)
                assert len(facts)==1
                fact=facts[0]
                assert fact.participant_id==recipient.first and fact.membership_epoch>0
                assert (fact.channel,fact.state,fact.provenance)==('native',expected,'client_reported')
                return {'membership_epoch':fact.membership_epoch,
                        'delivery_id_sha256':hashlib.sha256(fact.delivery_id.encode()).hexdigest(),
                        'observed_at':fact.observed_at}
            delivered=await observed('delivered')
            emit({'phase':'receipts-observed','attributed_delivery':True,**delivered})
            await command('read-receipt')
            read=await observed('read')
        emit({'phase':'read-observed','attributed_read':True,**read})
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
        def synchronous_recovery():
            async def recover(owner):
                original=owner.operations.restore(saved['reference'],owner.operation_kinds.message_send)
                assert (await original.wait(options)).snapshot.record_id==saved['record']
                assert (await owner.conversations.by_id(cid).refresh(options)).id==cid
                return True
            with SynchronousSdk(client()) as synchronous:
                assert synchronous.run(recover)
        await asyncio.to_thread(synchronous_recovery)
        emit({'phase':'resumed','record_id':saved['record'],'event_id_sha256':saved['event'],
              'original_reference_recovered':True,'same_cause_after_restart':True,'synchronous_original_recovered':True})
        checkpoint='revocation'
        await command('revoked')
        for action in ('read','receipts','replay','new'):
            async with client() as revoked_owner:
                try:
                    if action=='read':
                        original=revoked_owner.operations.restore(saved['reference'],revoked_owner.operation_kinds.message_send)
                        await original.status(options)
                    elif action=='receipts':
                        await revoked_owner.conversations.by_id(cid).receipts(MessageId('python-native-message'),options)
                    else:
                        await send(revoked_owner,forbidden=action=='new')
                except SdkError as error:
                    assert error.kind=='authentication'
                    assert error.certainty==('read-only' if action in ('read','receipts') else 'not-submitted')
                else:
                    raise AssertionError('Revoked authority accepted')
        assert len(http_workers)==2
        for worker in http_workers:
            assert worker.returncode is not None
            try:
                os.kill(worker.pid,0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError('Original HTTP worker still exists')
        emit({'phase':'completed','current_read_refused':True,'current_replay_refused':True,
              'current_new_mutation_refused':True,'current_receipts_refused':True,'http_workers_joined':2})
    finally:
        transport.close()


try:
    asyncio.create_subprocess_exec=observe_spawn
    asyncio.run(run())
except Exception as error:
    import traceback
    frames=traceback.extract_tb(error.__traceback__)
    local=[frame for frame in frames if frame.filename==__file__]
    emit({'phase':'failed','checkpoint':checkpoint,'kind':error.kind if isinstance(error,SdkError) else 'assertion' if isinstance(error,AssertionError) else 'unexpected',
          'certainty':error.certainty if isinstance(error,SdkError) else None,
          'source_line':local[-1].lineno if local else None,'error_type':type(error).__name__})
    raise
