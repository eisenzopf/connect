// Controlled original native APIs + real constructor/signaling/readiness/render.
// No browser/Core/F/wire qualification is claimed by these component controls.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
import {WebRtcReceiveReservationIntentOwner} from '../typescript/dist/browser_webrtc_receive_reservation.js';
import {BrowserWebRtcPublicVideoLifecycle} from '../typescript/dist/browser_webrtc_public_video_lifecycle.js';
import {BrowserDeadline} from '../typescript/dist/browser_deadline.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {canonicalJson} from '../typescript/dist/uctp_json.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
import {WEBRTC_BRIDGE_OPT_IN} from '../typescript/dist/browser_webrtc_public_video_bridge.js';

const tick=()=>new Promise(resolve=>setImmediate(resolve));
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return{promise,resolve,reject};};
const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'old',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'};
const scope={recordId:'record',operationId:'move',targetConnectionId:'target',audioStreamId:'audio',previewStreamId:'camera-a',publicStreamId:'public-b'};
const intent={profile:'connect/webrtc-public-video-reservation/1',phase:'intent',reservation_id:'reservation',nonce:'reservation-nonce',preview_stream_id:'camera-a',preview_role:'receive',public_stream_id:'public-b'};
const binding={stream_id:'public-b',preview_stream_id:'camera-a',mid:'2',stream_generation:1,physical_writer_incarnation:'writer',connection_generation:1,primary_ssrc:44,payload_type:96,clock_rate_hz:90000,codec:'vp8'};
const sdp=answer=>['v=0','o=- 1 1 IN IP4 127.0.0.1','s=original reserved B','c=IN IP4 127.0.0.1','t=0 0',
    'm=audio 9 UDP/TLS/RTP/SAVPF 111','a=mid:0','a=sendrecv','a=rtpmap:111 opus/48000/2',
    'm=video 9 UDP/TLS/RTP/SAVPF 96','a=mid:1','a=sendrecv','a=rtpmap:96 VP8/90000',
    'm=video 9 UDP/TLS/RTP/SAVPF 96','a=mid:2',answer?'a=sendonly':'a=recvonly','a=rtpmap:96 VP8/90000',''].join('\r\n');

async function scenario(run,options={}){
    const saved=new Map(),peers=[],workers=[],elements=[],contexts=[],worklets=[],closeListeners=new Set(),eventListeners=new Set();
    const privateSink=deferred(),publicSink=deferred(),privateSeen=deferred(),publicAttempt=deferred(),publicCaptured=deferred(),decodedObserved=deferred(),receiverStarted=deferred();
    const originalAbort=new AbortController();
    let now=10,revision=0,phaseName,committed=false,privateFrames=1,privateBFrames=0,publicFrames=0,statusState,
        phaseChange=options.phaseChange,publicCalls=0,privateCalls=0,statusCalls=0,wholeQueries=0,receiverQueries=0,blockedStats;
    const requests=[],canvasDraws=[];
    const selectedBinding={...binding,physical_writer_incarnation:options.bridge?'a'.repeat(32):binding.physical_writer_incarnation};
    const bridgeSeen=deferred();let bridgeFeeding=false,bridgeMarker,bridgeCalls=0;
    const bridgeSnapshot=()=>({opt_in:{...WEBRTC_BRIDGE_OPT_IN},feeding:bridgeFeeding?{profile:WEBRTC_BRIDGE_OPT_IN.profile,phase:'feeding',reservation_id:intent.reservation_id,operation_record_id:root.recordId,operation_id:root.operationId,target_connection_id:root.targetConnectionId,target_generation:2,binding:{...selectedBinding},bridge_readiness:{nonce:'bridge-nonce',expires_at:Math.floor(Date.now()/1000)+5,decoder_startup:'fresh-b-receiver-precut-keyframe'}}:null,decoded_ready:bridgeMarker??null});
    // Immutable original Store challenge, never renewed on a metadata read.
    const bridgeExpiry=Math.floor(Date.now()/1000)+5;
    const witness=profile=>({profile,operation_record_id:'record',operation_id:'move',reservation_id:'reservation',source_connection_id:'old',target_connection_id:'target',stream_id:'public-b',stream_generation:1,connection_generation:1,target_generation:2,source_cut_id:'cut',original_frame:{realm:'b'.repeat(32),ordinal:'8',last_old_ordinal:'7'},floor:{carrier:'webrtc-rtp',physical_writer_incarnation:selectedBinding.physical_writer_incarnation,primary_ssrc:44,first_public_rtp_timestamp:21000}});
    class Track{constructor(kind,index){this.kind=kind;this.id=`original-${index}`;this.readyState='live';this.stops=0;}stop(){this.stops++;this.readyState='ended';}}
    class Stream{constructor(tracks){this.tracks=tracks;}getTracks(){return this.tracks;}getAudioTracks(){return this.tracks.filter(t=>t.kind==='audio');}getVideoTracks(){return this.tracks.filter(t=>t.kind==='video');}}
    let codecPending=options.codecPending;
    const receiverReport=()=>new Map([
        ['vp8',{id:'vp8',type:'codec',mimeType:'video/VP8',payloadType:selectedBinding.payload_type,clockRate:90000}],
        ['b',{id:'b',type:'inbound-rtp',kind:'video',mid:'2',trackIdentifier:'original-2',codecId:codecPending?undefined:'vp8',ssrc:selectedBinding.primary_ssrc,framesDecoded:publicFrames,keyFramesDecoded:options.omitBridgeKeys?undefined:(publicFrames>0?1:0)}],
    ]);
    class Peer extends EventTarget{
        connectionState='new';iceConnectionState='new';iceGatheringState='complete';signalingState='stable';localDescription=null;remoteDescription=null;transceivers=[];closes=0;
        constructor(configuration){super();this.configuration=configuration;peers.push(this);}
        getConfiguration(){return this.configuration??{};}
        addTransceiver(kind,settings){const index=this.transceivers.length,track=new Track(kind,index);
            const tx={mid:null,direction:settings.direction,currentDirection:null,
                sender:{track:null,transform:null,getParameters:()=>({encodings:settings.sendEncodings??[]}),async setParameters(){},async replaceTrack(track){this.track=track;}},
                receiver:{track,jitterBufferTarget:0,getStats:()=>{receiverQueries++;receiverStarted.resolve();return blockedStats?.promise??Promise.resolve(receiverReport());}},setCodecPreferences(){}};
            this.transceivers.push(tx);return tx;}
        getTransceivers(){return this.transceivers;}getReceivers(){return this.transceivers.map(tx=>tx.receiver);}
        async createOffer(){return{type:'offer',sdp:sdp(false)};}
        async setLocalDescription(value){this.localDescription=value;this.signalingState='have-local-offer';}
        async setRemoteDescription(value){this.remoteDescription=value;this.signalingState='stable';
            for(const[index,tx]of this.transceivers.entries()){tx.mid=String(index);tx.currentDirection=tx.direction;this.ontrack?.({transceiver:tx,receiver:tx.receiver,track:tx.receiver.track});}
            this.connectionState=this.iceConnectionState='connected';this.dispatchEvent(new Event('connectionstatechange'));}
        async getStats(){wholeQueries++;return new Map([
            ['audio',{type:'inbound-rtp',kind:'audio',mid:'0',trackIdentifier:options.omitTrackIdentifiers?undefined:'original-0',totalSamplesReceived:1920,concealedSamples:0}],
            ['a',{type:'inbound-rtp',kind:'video',mid:'1',trackIdentifier:options.omitTrackIdentifiers?undefined:'original-1',framesDecoded:privateFrames,keyFramesDecoded:privateFrames>0?1:0}],
            ['b',{type:'inbound-rtp',kind:'video',mid:'2',trackIdentifier:options.omitTrackIdentifiers?undefined:'original-2',framesDecoded:options.bridge?publicFrames:privateBFrames,keyFramesDecoded:(options.bridge?publicFrames:privateBFrames)>0?1:0,packetsReceived:options.bridge?publicFrames:privateBFrames}],
        ]);}
        close(){this.closes++;this.connectionState='closed';this.dispatchEvent(new Event('connectionstatechange'));}
    }
    class Node{gain={value:1};connect(){}disconnect(){this.disconnected=true;}}
    class Context extends EventTarget{sampleRate=48000;state='running';destination={};audioWorklet={async addModule(){}};closes=0;
        constructor(){super();contexts.push(this);}createMediaStreamSource(){const node=new Node();node.context=this;return node;}createGain(){return new Node();}
        async resume(){this.state='running';}async suspend(){this.state='suspended';}async close(){this.closes++;this.state='closed';}}
    class Worklet extends Node{port={onmessage:null,postMessage(){},close(){this.closed=true;}};constructor(){super();worklets.push(this);}}
    class Worker{constructor(){workers.push(this);}terminate(){this.terminated=true;}}
    let nextCallback=0;
    class Element{constructor(kind){this.kind=kind;this.srcObject=null;this.muted=false;this.readyState=2;this.videoWidth=640;this.videoHeight=360;this.callbacks=new Map();this.presented=0;this.pauses=0;elements.push(this);}
        play(){this.paused=false;if(options.bridgePlayPending&&this.srcObject?.getVideoTracks()[0]?.id==='original-2'){this.firstPlay??=deferred();this.firstPlayPending??=true;return this.firstPlay.promise;}return this.rejectPlay?Promise.reject(Error('controlled original play rejection')):Promise.resolve();}
        pause(){this.paused=true;this.pauses++;if(this.firstPlayPending){this.firstPlayPending=false;this.firstPlay.reject(new DOMException('Original media play stopped','AbortError'));}}
        requestVideoFrameCallback(fn){const id=++nextCallback;this.callbacks.set(id,fn);
            const idTrack=this.srcObject?.getVideoTracks()[0]?.id;
            if(idTrack==='original-1')privateSink.resolve(this);if(idTrack==='original-2')publicSink.resolve(this);return id;}
        cancelVideoFrameCallback(id){this.callbacks.delete(id);}}
    const document=Object.assign(new EventTarget(),{visibilityState:'visible',createElement:kind=>new Element(kind)});
    const values={RTCPeerConnection:Peer,RTCRtpSender:{getCapabilities:kind=>({codecs:[{mimeType:kind==='audio'?'audio/opus':'video/VP8',clockRate:kind==='audio'?48000:90000,channels:kind==='audio'?2:0}]})},RTCRtpScriptTransform:class{},Worker,MediaStream:Stream,AudioContext:Context,AudioWorkletNode:Worklet,document,performance:{now:()=>now,timeOrigin:1794700000000}};
    const phase=()=>({profile:'connect/webrtc-public-video-phase/1',phase:phaseName,reservation_id:intent.reservation_id,
        operation_record_id:root.recordId,operation_id:root.operationId,target_connection_id:root.targetConnectionId,target_generation:2,binding:{...selectedBinding},
        ...(phaseName!=='negotiated-never-fed'?{source_cut_id:'cut',public_readiness:{nonce:'public-nonce',decoder_startup:'fresh-receiver-first-decoded-frame'}}:{})});
    const receipt=()=>{const ready={stream_revision:4,streams:['audio','camera-a','public-b'],participant_revision:2};return{activation_state:'ready',connection_id:'target',replaces:'old',generation:2,endpoint_id:'endpoint',device_id:'device',retained_connection_ids:['retained'],admitted_connection_ids:['target'],retired_connection_ids:['old'],readiness:ready,physical_readiness:{...ready}};};
    const record=(connection,state)=>{const observed=phaseName?(phaseChange?phaseChange(phase()):phase()):undefined;return envelope('operation.updated',{operation_id:'move',record_id:'record',command_type:'connection.replace',revision:options.sameRevision?1:++revision,state,native_replacement_prepared:true,
        ...(state==='preparing'?{webrtc_video_receive_reservation:{...intent}}:{}),
        ...(observed!==undefined?{webrtc_public_video_phase:observed}:{}),
        ...(options.bridge?{webrtc_public_video_bridge:(()=>{const v=bridgeSnapshot();if(v.feeding)v.feeding.bridge_readiness.expires_at=bridgeExpiry;return v;})()}:{}),
        ...(options.bridge&&['cut-bound-post-f','decoded-ready'].includes(phaseName)?{receiver_video_post_cut:witness('connect-receiver-video-post-cut-v1')}:{ }),
        ...(options.bridge&&state==='completed'?{receiver_video_release:witness('connect-receiver-video-release-v1')}:{ }),
        ...(state==='completed'?{result:options.resultChange?.(receipt())??receipt()}:{}),
    },{cid:'room',sid:'session',connid:connection,operation_id:'move'});};
    const client={closed:false,controlTransport:'websocket',deviceId:'device',onClose(fn){closeListeners.add(fn);return()=>closeListeners.delete(fn);},onEvent(fn){eventListeners.add(fn);return()=>eventListeners.delete(fn);},
        async requestReplacementContinuation(type,body,sentScope){requests.push({type,body:structuredClone(body),scope:structuredClone(sentScope)});
            if(type==='operation.get'){statusCalls++;if(phaseName==='decoded-ready')decodedObserved.resolve();return[record('old',committed?'completed':statusState??'preparing')];}
            assert.equal(type,'connection.offer');const marker=body.replacement_preparation;
            if(marker.profile==='connect-replacement-preparation-v1')return[record('target','preparing')];
            if(marker.profile==='connect-replacement-media-v1'){
                phaseName='negotiated-never-fed';return[envelope('connection.answer',{substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp:sdp(true)},capabilities:{},
                    streams_answered:[{id:'audio',kind:'audio',direction:'sendrecv',codec:'opus'},{id:'camera-a',kind:'video',direction:'sendrecv',codec:'vp8'},{id:'public-b',kind:'video',direction:'sendonly',codec:'vp8'}],
                    replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record',readiness:{nonce:'private-nonce',source_generation:1,target_generation:2,stream_revision:2}},
                },{cid:'room',sid:'session',connid:'target',operation_id:'move'})];}
            if(marker.profile==='connect-replacement-ready-v1'){
                privateCalls++;assert.equal(marker.readiness.audio.stream_id,'audio');assert.equal(marker.readiness.video.stream_id,'camera-a');
                if(options.bridge){assert.deepEqual({...body.webrtc_public_video_bridge_opt_in},WEBRTC_BRIDGE_OPT_IN);bridgeFeeding=true;}else phaseName='cut-bound-post-f';privateSeen.resolve();return[record('target','preparing')];}
            if(marker.profile==='connect-replacement-public-video-bridge-ready-v1'){bridgeCalls++;bridgeMarker=structuredClone(marker);assert.equal(marker.bridge_readiness.decoded_rtp_timestamp,12000);phaseName='cut-bound-post-f';bridgeSeen.resolve();return[record('target','preparing')];}
            assert.equal(marker.profile,'connect-replacement-public-video-ready-v1');publicCalls++;publicAttempt.resolve();
            if(options.public503&&publicCalls===1)throw new ConnectError('refused','controlled preeffect storage refusal',{code:503,category:'storage',reason:'storage_unavailable'});
            phaseName='decoded-ready';publicCaptured.resolve();
            if(options.lostPublicReply)throw new ConnectError('transport','controlled original reply loss');
            return[record('target','preparing')];
        },
    };
    let begun,workError;
    for(const[name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});}
    try{
        const owner=new WebRtcReceiveReservationIntentOwner(scope),selected=owner.observe(intent,root),reservation={owner,intent:selected};
        begun=BrowserRtcConnection.beginReplacement(client,{conversationId:'room',sessionId:'session',connectionId:'target',operationId:'move',audio:'audio',video:'camera-a',receiveVideo:'public-b'},'participant',{},root,{timeoutMs:1000,signal:originalAbort.signal},reservation);
        const wait=promise=>Promise.race([promise,begun.completed.then(()=>{throw Error('completed before requested fixture stage');})]);
        const frame=element=>{if(element.firstPlayPending){element.firstPlayPending=false;element.firstPlay.resolve();}now++;const callbacks=[...element.callbacks.values()];element.callbacks.clear();assert(callbacks.length>0);element.presented++;for(const callback of callbacks)callback(now,{width:640,height:360,mediaTime:element.presented/30,presentedFrames:element.presented,presentationTime:now,rtpTimestamp:9000+element.presented*3000});};
        await run({begun,client,reservation,peers,elements,contexts,worklets,requests,canvasDraws,wait,frame,
            async privateInput(beforePump){const element=await wait(privateSink.promise);beforePump?.();worklets[0].port.onmessage({data:{epoch:0,timestampUs:1000,samples:new Float32Array(960)}});frame(element);return element;},
            publicElement:()=>wait(publicSink.promise),
            async publicInput(){const element=await wait(publicSink.promise);publicFrames=1;frame(element);return element;},
            bridgeSeen:()=>wait(bridgeSeen.promise),privateSeen:()=>wait(privateSeen.promise),publicAttempt:()=>wait(publicAttempt.promise),publicCaptured:()=>wait(publicCaptured.promise),decodedObserved:()=>wait(decodedObserved.promise),receiverStarted:()=>wait(receiverStarted.promise),
            advance(value){now+=value;},expire(){now=5000;},commit(){committed=true;phaseName='activated';},unknown(){statusState='unknown';},
            phaseChange(fn){phaseChange=fn;},setPrivateFrames(value){privateFrames=value;},setPrivateBFrames(value){privateBFrames=value;},blockReceiver(){blockedStats=deferred();},releaseReceiver(){blockedStats?.resolve(receiverReport());blockedStats=undefined;},
            completeCodecReport(){codecPending=false;},
            abort(){originalAbort.abort();},closeClient(){client.closed=true;for(const fn of [...closeListeners])fn();},
            canvas:{getContext(){return{drawImage(source){canvasDraws.push(source);}};}},
            counts:()=>({privateCalls,publicCalls,statusCalls,wholeQueries,receiverQueries,bridgeCalls}),
            async renderFrame(work,element){for(;;){if(element.callbacks.size){frame(element);return;}await Promise.race([tick(),work.then(()=>{throw Error('render finished without actual frame');})]);}},
        });
    }catch(error){workError=error;}finally{
        blockedStats?.resolve(receiverReport());
        if(begun){await begun.owner.shutdown();await begun.completed.catch(()=>{});}
        const closed=peers.every(peer=>peer.connectionState==='closed'&&peer.closes===1&&peer.transceivers.every(tx=>tx.receiver.track.stops===1));
        const decoderClean=elements.every(element=>element.srcObject===null&&element.callbacks.size===0)&&contexts.every(context=>context.closes===1)&&worklets.every(node=>node.port.closed);
        const listenersGone=closeListeners.size===0&&eventListeners.size===0,workersGone=workers.every(worker=>worker.terminated);
        for(const[name,descriptor]of saved){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}
        assert(closed&&decoderClean&&listenersGone&&workersGone,'All original native/decoder/callback/listener owners clean before behavior outcome');
    }
    if(workError)throw workError;
}

test('actual reserved constructor privately decodes A/audio then adopts same warmed B only after original completed Activated',()=>scenario(async f=>{
    await f.begun.prepared;assert.deepEqual(f.peers[0].transceivers.map(tx=>tx.direction),['sendrecv','sendrecv','recvonly']);
    await f.privateInput();await f.privateSeen();const picture=await f.publicInput();await f.publicCaptured();
    assert.equal(f.canvasDraws.length,0);assert.equal(f.counts().privateCalls,1);assert.equal(f.counts().publicCalls,1);
    const original=f.begun.owner.signaling.originalPublicVideoReplacement();
    const status=await original.reservedStatus({timeoutMs:100});
    assert.ok(Object.isFrozen(status)&&Object.isFrozen(status.webrtc_public_video_phase.binding));
    assert.throws(()=>{status.state='completed';},TypeError);
    assert.throws(()=>{status.webrtc_public_video_phase.binding.mid='1';},TypeError);
    const forged=structuredClone(status);forged.state='completed';forged.result={activation_state:'ready'};
    assert.throws(()=>original.reservedCompleted(forged),{code:'protocol'});
    forged.state='preparing';forged.webrtc_public_video_phase.phase='cut-bound-post-f';
    const marker=f.requests.find(row=>row.body.replacement_preparation?.profile==='connect-replacement-public-video-ready-v1').body.replacement_preparation;
    await assert.rejects(original.reservedPublicReady(marker,{timeoutMs:100},forged),{code:'protocol'});
    assert.equal(f.counts().publicCalls,1,'A mutable copied status cannot authorize another continuation');
    const stream=picture.srcObject;f.commit();await f.begun.completed;assert.equal(picture.srcObject,stream);assert.equal(picture.callbacks.size,0);
    const completed=await original.reservedStatus({timeoutMs:100});
    assert.ok(Object.isFrozen(completed.result.readiness.streams));
    assert.throws(()=>completed.result.readiness.streams.push('foreign'),TypeError);
    f.expire();const rendered=f.begun.owner.render({audio:false,video:{'public-b':f.canvas}},{timeoutMs:200});
    await f.renderFrame(rendered,picture);const handle=await rendered;assert.deepEqual(f.canvasDraws,[picture]);handle.close();await handle.closed;
    assert.equal(picture.srcObject,stream,'Renderer returns same decoder to completed connection ownership');
    const retry=f.begun.owner.render({audio:false,video:{'public-b':f.canvas}},{timeoutMs:200});await f.renderFrame(retry,picture);const again=await retry;again.close();await again.closed;
    assert.equal(f.canvasDraws[1],picture);assert.equal(f.counts().publicCalls,1);assert.equal(f.peers[0].transceivers[1].sender.track,null,'Camera sender is not replaced by decoder adoption');
}));

test('reserved public readiness replays identical frozen marker only after current original cut-bound read and never reruns private decode',()=>scenario(async f=>{
    await f.privateInput();await f.privateSeen();await f.publicInput();await f.publicAttempt();await tick();f.advance(100);await f.publicCaptured();
    const sends=f.requests.filter(row=>row.body.replacement_preparation?.profile==='connect-replacement-public-video-ready-v1');
    assert.equal(sends.length,2);assert.equal(canonicalJson(sends[0].body),canonicalJson(sends[1].body));assert.equal(f.counts().privateCalls,1);
    assert(f.requests.slice(f.requests.indexOf(sends[0])+1,f.requests.indexOf(sends[1])).some(row=>row.type==='operation.get'));
    f.commit();await f.begun.completed;
},{public503:true,sameRevision:true}));

test('decoded-ready and uncertain original status recover by reads only without resubmitting a public marker',()=>scenario(async f=>{
    await f.privateInput();await f.privateSeen();await f.publicInput();await f.publicCaptured();await f.decodedObserved();f.unknown();f.advance(100);
    await tick();f.commit();await f.begun.completed;assert.equal(f.counts().publicCalls,1);assert.equal(f.counts().privateCalls,1);
},{lostPublicReply:true}));

test('expired original unadopted B lease cannot use late completed status or fresh render budget to mint another observation',()=>scenario(async f=>{
    await f.privateInput();await f.privateSeen();const picture=await f.publicInput();await f.publicCaptured();f.expire();
    await assert.rejects(f.begun.completed,{code:'timeout'});assert.equal(picture.srcObject,null);f.commit();
    await assert.rejects(f.begun.owner.render({audio:false,video:{'public-b':f.canvas}},{timeoutMs:200}),{code:'unsupported-media-profile'});
    assert.equal(f.canvasDraws.length,0);assert.equal(f.counts().publicCalls,1);
    const original=f.begun.owner.signaling.originalPublicVideoReplacement();
    assert.throws(()=>new BrowserWebRtcPublicVideoLifecycle(original,f.peers[0],f.begun.owner.receiveRoles,scope,new BrowserDeadline(1000),new AbortController().signal,work=>work),{code:'conflict'});
}));

test('private A readiness cannot aggregate B decoded counters or receive a public marker before original A evidence',()=>scenario(async f=>{
    await f.privateInput(()=>{f.setPrivateFrames(0);f.setPrivateBFrames(99);});await assert.rejects(f.begun.completed,{code:'unsupported-media-profile'});
    assert.equal(f.counts().privateCalls,0);assert.equal(f.counts().publicCalls,0);assert.equal(f.canvasDraws.length,0);
},{omitTrackIdentifiers:true}));

test('completed Root with missing or changed Activated B proof refuses before any public draw',async()=>{
    for(const change of [value=>({...value,binding:{...value.binding,mid:'1'}}),()=>undefined,
        value=>({...value,operation_id:'foreign'}),value=>({...value,source_cut_id:'different-cut'}),
        value=>({...value,public_readiness:{...value.public_readiness,nonce:'private-nonce'}})])
        await scenario(async f=>{
            await f.privateInput();await f.privateSeen();await f.publicInput();await f.publicCaptured();
            f.phaseChange(value=>value.phase==='activated'?change(value):value);f.commit();
            await assert.rejects(f.begun.completed,{code:'protocol'});assert.equal(f.canvasDraws.length,0);
        });
});

test('original cancellation joins a pending B native report without retaining a decoder or sending public readiness',()=>scenario(async f=>{
    await f.privateInput();await f.privateSeen();f.blockReceiver();const picture=await f.publicInput();await f.receiverStarted();
    f.abort();await assert.rejects(f.begun.completed,{code:'cancelled'});
    assert.equal(picture.srcObject,null);assert.equal(picture.callbacks.size,0);assert.equal(f.counts().publicCalls,0);
    const samples=f.counts().receiverQueries;f.releaseReceiver();await tick();assert.equal(f.counts().receiverQueries,samples);
    assert.equal(f.canvasDraws.length,0);
}));

test('original client retirement after B readiness revokes pending lease and never grants late Activated rendering',()=>scenario(async f=>{
    await f.privateInput();await f.privateSeen();const picture=await f.publicInput();await f.publicCaptured();
    f.closeClient();await assert.rejects(f.begun.completed,error=>['closed','cancelled'].includes(error.code));
    assert.equal(picture.srcObject,null);assert.equal(picture.callbacks.size,0);f.commit();
    await assert.rejects(f.begun.owner.render({audio:false,video:{'public-b':f.canvas}},{timeoutMs:200}),{code:'closed'});
    assert.equal(f.counts().publicCalls,1);assert.equal(f.canvasDraws.length,0);
}));


test('explicit signed bridge prearms hidden B before private Ready and retains that decoder through exact post-F and final receiver release',()=>scenario(async f=>{
    await f.begun.prepared;
    const picture=await f.publicElement();assert.equal(f.counts().privateCalls,0);assert.equal(f.canvasDraws.length,0);assert.equal(picture.firstPlayPending,true);
    assert.equal(f.elements.filter(e=>e.srcObject?.getVideoTracks()[0]?.id==='original-2').length,1);
    await f.privateInput();await f.privateSeen();assert.equal(picture.firstPlayPending,true,'media-dependent B play cannot block original private Ready');
    assert.equal(await f.publicInput(),picture);await f.bridgeSeen();await tick();
    assert.equal(f.counts().bridgeCalls,1);assert.equal(f.counts().privateCalls,1);assert.equal(f.counts().publicCalls,0,'cached pre-F RTP 12000 cannot satisfy postcut floor21000');
    f.frame(picture);await tick();assert.equal(f.counts().publicCalls,0); //15000 delayed private callback
    f.frame(picture);await tick();assert.equal(f.counts().publicCalls,0); //18000 still beforefloor
    f.frame(picture);await f.publicCaptured();assert.equal(f.counts().publicCalls,1);assert.equal(f.canvasDraws.length,0);
    const privateB=f.requests.filter(r=>r.type==='connection.offer'&&r.body.replacement_preparation.profile==='connect-replacement-public-video-bridge-ready-v1');
    assert.equal(privateB.length,1);assert.equal(privateB[0].body.replacement_preparation.bridge_readiness.decoded_rtp_timestamp,12000);
    assert.equal(privateB[0].body.replacement_preparation.bridge_readiness.binding.primary_ssrc,44);
    assert.ok(f.requests.filter(r=>r.type==='connection.offer').every(r=>canonicalJson(r.body.webrtc_public_video_bridge_opt_in)===canonicalJson(WEBRTC_BRIDGE_OPT_IN)));
    f.commit();await f.begun.completed;
    const renderer=f.begun.owner.render({audio:false,video:{'public-b':f.canvas}},{timeoutMs:500});await f.renderFrame(renderer,picture);const lease=await renderer;
    assert.equal(f.canvasDraws[0],picture);assert.equal(f.elements.filter(e=>e.srcObject?.getVideoTracks()[0]?.id==='original-2').length,1);
    assert.equal(f.counts().privateCalls,1);assert.equal(f.counts().bridgeCalls,1);assert.equal(f.counts().publicCalls,1);lease.close();await lease.closed;
},{bridge:true,omitBridgeKeys:true,bridgePlayPending:true}));

test('explicit bridge keeps pre-F B hidden and original cancellation closes decoder without private re-observation or public Ready',()=>scenario(async f=>{
    await f.begun.prepared;const picture=await f.publicElement();await f.privateInput();await f.privateSeen();assert.equal(await f.publicInput(),picture);await f.bridgeSeen();await tick();
    assert.equal(f.counts().publicCalls,0);assert.equal(f.canvasDraws.length,0);f.abort();await assert.rejects(f.begun.completed);
    assert.equal(f.canvasDraws.length,0);assert.equal(f.counts().privateCalls,1);assert.equal(f.counts().bridgeCalls,1);assert.equal(picture.srcObject,null);
},{bridge:true}));

test('pre-cut bridge waits for real B callback and transient initial codec linkage',()=>scenario(async f=>{
    await f.begun.prepared;
    const picture=await f.publicElement();
    await f.privateInput();await f.privateSeen();
    await tick();
    assert.equal(f.counts().receiverQueries,0,'no B stats inspection before exact callback');
    assert.equal(await f.publicInput(),picture);
    await f.receiverStarted();await tick();
    assert.equal(f.counts().bridgeCalls,0,'incomplete codec linkage cannot sign Ready or retire B');
    assert.equal(picture.callbacks.size,1);
    f.completeCodecReport();await f.bridgeSeen();
    assert.equal(f.counts().bridgeCalls,1);
    assert.equal(f.counts().publicCalls,0);
    f.abort();await assert.rejects(f.begun.completed);
},{bridge:true,codecPending:true,bridgePlayPending:true}));
