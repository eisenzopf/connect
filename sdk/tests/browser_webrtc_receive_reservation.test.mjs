import test from 'node:test';
import assert from 'node:assert/strict';
import {WebRtcReceiveReservationIntentOwner, originalWebRtcReceiveIntent} from '../typescript/dist/browser_webrtc_receive_reservation.js';
import {receiveRolesForOriginalIntent} from '../typescript/dist/browser_webrtc_receive_roles.js';

const scope=()=>({recordId:'record',operationId:'move',targetConnectionId:'target',audioStreamId:'audio',previewStreamId:'camera-a',publicStreamId:'public-b'});
const wire=()=>({profile:'connect/webrtc-public-video-reservation/1',phase:'intent',reservation_id:'reservation',nonce:'original-nonce',preview_stream_id:'camera-a',preview_role:'receive',public_stream_id:'public-b'});
const protocol=error=>error?.code==='protocol';
function intentFixture(){const expected=scope(),owner=new WebRtcReceiveReservationIntentOwner(expected);return{expected,owner,intent:owner.observe(wire(),expected)};}
function nativeFixture(){
 const value=intentFixture();let reads=0;
 const tx=kind=>({direction:'sendrecv',currentDirection:null,mid:null,receiver:{track:{kind,readyState:'live',id:'native-'+kind}},sender:{track:null}});
 const transceivers={audio:tx('audio'),preview:tx('video'),public:tx('video')};transceivers.public.direction='recvonly';
 const actual=Object.values(transceivers),peer={connectionState:'new',signalingState:'stable',localDescription:null,remoteDescription:null,getTransceivers(){reads++;return actual.slice();}};
 return{...value,peer,actual,transceivers,get reads(){return reads;},create(){return receiveRolesForOriginalIntent(peer,value.expected,value.intent,transceivers);},
  negotiate(){for(const[index,item]of actual.entries()){item.mid=String(index);item.currentDirection=item.direction;}peer.connectionState='connected';},
  event(role){const transceiver=transceivers[role];return{transceiver,receiver:transceiver.receiver,track:transceiver.receiver.track};}};
}

test('same original intent roundtrips exact role fields and captures immutable scalars once',()=>{
 const expected=scope(),input=wire(),owner=new WebRtcReceiveReservationIntentOwner(expected);
 const first=owner.observe(input,expected);assert.deepEqual(first,input);assert.ok(Object.isFrozen(first));
 input.nonce='caller-change';assert.equal(first.nonce,'original-nonce');
 assert.equal(owner.observe(wire(),expected),first);assert.ok(originalWebRtcReceiveIntent(first,expected));
 assert.equal(originalWebRtcReceiveIntent({...first},expected),false,'A structural object is not this parsed original');
 const maximum=wire();maximum.reservation_id='a'.repeat(128);assert.equal(new WebRtcReceiveReservationIntentOwner(expected).observe(maximum,expected).reservation_id.length,128);
});

test('intent rejects negotiated cut authority unknown missing and malformed fields without accessor evaluation',()=>{
 for(const[field,value]of [['profile','other'],['phase','negotiated-never-fed'],['preview_role','recvonly'],['preview_stream_id','foreign-a'],['public_stream_id','camera-a'],['reservation_id','a'.repeat(129)],['nonce','bad\nnonce'],['nonce','é']]){
  const changed=wire();changed[field]=value;assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(changed,scope()),protocol,field);
 }
 for(const field of ['source_cut_id','mid','target_primary_ssrc','stream_generation','connection_generation','physical_writer_incarnation','ready','decoded_frames','permission','audience','track_id']){
  const changed={...wire(),[field]:'forged'};assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(changed,scope()),protocol,field);
 }
 for(const field of Object.keys(wire())){const changed=wire();delete changed[field];assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(changed,scope()),protocol,field);}
 for(const value of [null,false,[],{},'intent'])assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(value,scope()),protocol);
 let reads=0;const getter=wire();Object.defineProperty(getter,'nonce',{get(){reads++;return'original-nonce';}});
 assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(getter,scope()),protocol);assert.equal(reads,0);
 const symbol=wire();symbol[Symbol('authority')]=true;assert.throws(()=>new WebRtcReceiveReservationIntentOwner(scope()).observe(symbol,scope()),protocol);
});

test('original Root and reservation identity cannot disappear remint change mapping or revive after close',()=>{
 const {owner,expected,intent}=intentFixture();
 for(const field of ['recordId','operationId','targetConnectionId'])assert.throws(()=>owner.observe(wire(),{...expected,[field]:'foreign'}),protocol);
 for(const field of ['reservation_id','nonce','preview_stream_id','public_stream_id'])assert.throws(()=>owner.observe({...wire(),[field]:'replacement'},expected),protocol);
 assert.throws(()=>owner.observe(undefined,expected),protocol);assert.equal(owner.observe(wire(),expected),intent);
 assert.equal(originalWebRtcReceiveIntent(intent,{...expected,targetConnectionId:'foreign'}),false);
 assert.throws(()=>new WebRtcReceiveReservationIntentOwner({...expected,publicStreamId:expected.audioStreamId}),protocol);
 owner.close();owner.close();assert.throws(()=>owner.observe(wire(),expected),{code:'closed'});assert.equal(originalWebRtcReceiveIntent(intent,expected),false);
});

test('absent intent leaves ordinary native path untouched and an unsigned structural option cannot enable B',()=>{
 const expected=scope(),owner=new WebRtcReceiveReservationIntentOwner(expected);assert.equal(owner.observe(undefined,expected),undefined);
 const peer={get connectionState(){assert.fail('Default path touched native peer');}};
 assert.equal(receiveRolesForOriginalIntent(peer,expected,undefined,undefined),undefined);
 assert.throws(()=>receiveRolesForOriginalIntent(peer,expected,wire(),undefined),protocol);
});

test('duplex A shares its camera ID while exact separate recvonly B gets its original native role',()=>{
 const f=nativeFixture(),roles=f.create();assert.equal(roles.binding('public'),undefined);f.negotiate();
 for(const role of ['audio','preview','public'])assert.equal(roles.accept(f.event(role)),role);
 const a=roles.binding('preview'),b=roles.binding('public');
 assert.equal(a.streamId,f.expected.previewStreamId);assert.equal(b.streamId,f.expected.publicStreamId);
 assert.equal(a.transceiver,f.transceivers.preview);assert.equal(b.transceiver,f.transceivers.public);assert.notEqual(a.transceiver,b.transceiver);
 assert.equal(a.mid,'1');assert.equal(b.mid,'2');assert.ok(Object.isFrozen(b));assert.equal(roles.accept(f.event('public')),'public');
 assert.equal('ready'in b,false);assert.equal('source_cut_id'in b,false);assert.equal('permission'in b,false,'Native association is not a media grant');
 roles.close();
});

test('foreign peers mixed receivers tracks directions MIDs and changed original B cannot be substituted',()=>{
 for(const mutate of [f=>f.actual.pop(),f=>{f.transceivers.public=f.transceivers.preview;},f=>{f.transceivers.public.direction='sendrecv';},f=>{f.peer.remoteDescription={type:'answer'};},f=>{f.peer.connectionState='connected';}]){
  const f=nativeFixture();mutate(f);assert.throws(()=>f.create(),protocol);
 }
 for(const mutate of [f=>{f.transceivers.public.mid='1';},f=>{f.transceivers.public.mid='bad mid';},f=>{f.transceivers.public.currentDirection='sendonly';},f=>{f.actual[2]={...f.transceivers.public};}]){
  const f=nativeFixture(),roles=f.create();f.negotiate();mutate(f);assert.throws(()=>roles.accept(f.event('public')),protocol);roles.close();
 }
 const f=nativeFixture(),roles=f.create();f.negotiate();const event=f.event('public');
 assert.throws(()=>roles.accept({...event,receiver:f.transceivers.preview.receiver}),protocol);
 assert.throws(()=>roles.accept({...event,track:{...event.track}}),protocol,'Equal track IDs do not establish pointer identity');
 roles.accept(event);f.transceivers.public.receiver.track={...event.track};assert.throws(()=>roles.binding('public'),protocol);roles.close();
});

test('retirement clears owned role references rejects late callbacks and never operates native resources',()=>{
 const f=nativeFixture();let operations=0;for(const item of f.actual){item.stop=()=>operations++;item.receiver.track.stop=()=>operations++;}
 const roles=f.create();f.negotiate();roles.accept(f.event('public'));const oldEvent=f.event('public');
 roles.close();roles.close();assert.equal(roles.peer,undefined);assert.equal(roles.slots.size,0);
 assert.throws(()=>roles.accept(oldEvent),{code:'closed'});assert.throws(()=>roles.binding('public'),{code:'closed'});assert.equal(operations,0);
 const closed=nativeFixture(),other=closed.create();closed.peer.connectionState='closed';assert.throws(()=>other.accept(closed.event('public')),protocol);other.close();
 const retired=nativeFixture(),owned=retired.create();retired.owner.close();assert.throws(()=>owned.binding('public'),protocol);assert.throws(()=>retired.create(),protocol);owned.close();
});

async function constructedRoles(run){
 const {BrowserRtcConnection}=await import('../typescript/dist/browser_sdk_webrtc.js'),{CanonicalWebRtcSignaling}=await import('../typescript/dist/browser_webrtc_signaling.js');
 const saved=new Map(),peers=[],workers=[],owners=[],listeners=new Set();
 class Track{constructor(kind){this.kind=kind;this.id=`track-${kind}-${peers.length}`;this.readyState='live';}stop(){this.readyState='ended';}}
 class Peer extends EventTarget{
  connectionState='new';iceConnectionState='new';signalingState='stable';localDescription=null;remoteDescription=null;transceivers=[];closes=0;
  constructor(configuration){super();this.configuration=configuration;peers.push(this);}
  getConfiguration(){return this.configuration??{};}getTransceivers(){return this.transceivers;}
  addTransceiver(kind,options){const sender={track:null,transform:null,getParameters:()=>({encodings:options.sendEncodings??[]}),async setParameters(){},async replaceTrack(track){this.track=track;}};const value={direction:options.direction,currentDirection:null,mid:null,sender,receiver:{track:new Track(kind),jitterBufferTarget:0},setCodecPreferences(codecs){this.codecs=codecs;}};this.transceivers.push(value);return value;}
  close(){this.closes++;this.connectionState='closed';}
 }
 class Worker{constructor(){workers.push(this);}terminate(){this.terminated=true;}}
 const values={RTCPeerConnection:Peer,RTCRtpSender:{getCapabilities:kind=>({codecs:[{mimeType:kind==='audio'?'audio/opus':'video/VP8',clockRate:kind==='audio'?48000:90000,channels:kind==='audio'?2:0}]})},Worker,RTCRtpScriptTransform:class{}};
 for(const[name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,value});}
 const client={closed:false,controlTransport:'websocket',deviceId:'device',onClose(fn){listeners.add(fn);return()=>listeners.delete(fn);},onEvent(fn){listeners.add(fn);return()=>listeners.delete(fn);}};
 const original={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'old',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'};
 const input={conversationId:'room',sessionId:'session',connectionId:'target',operationId:'move',audio:'audio',video:'camera-a',receiveVideo:'public-b'};
 const make=()=>{const owner=new WebRtcReceiveReservationIntentOwner(scope()),intent=owner.observe(wire(),scope());return{owner,intent};};
 const answer=()=>['v=0','o=- 1 1 IN IP4 127.0.0.1','s=constructor role model','t=0 0','m=audio 9 UDP/TLS/RTP/SAVPF 111','a=mid:0','a=sendrecv','a=rtpmap:111 opus/48000/2','m=video 9 UDP/TLS/RTP/SAVPF 96','a=mid:1','a=sendrecv','a=rtpmap:96 VP8/90000','m=video 9 UDP/TLS/RTP/SAVPF 96','a=mid:2','a=sendonly','a=rtpmap:96 VP8/90000',''].join('\r\n');
 const negotiate=peer=>{for(const[index,tx]of peer.transceivers.entries()){tx.mid=String(index);tx.currentDirection=tx.direction;}peer.localDescription={type:'offer',sdp:'owned local offer'};peer.remoteDescription={type:'answer',sdp:answer()};peer.signalingState='stable';peer.connectionState=peer.iceConnectionState='connected';for(const tx of peer.transceivers)peer.ontrack?.({transceiver:tx,receiver:tx.receiver,track:tx.receiver.track});};
 const construct=(reservation=make())=>{const owner=new BrowserRtcConnection(client,input,'participant',{},original,reservation);owners.push(owner);return{owner,reservation,peer:peers.at(-1)};};
 try{await run({BrowserRtcConnection,CanonicalWebRtcSignaling,client,input,original,make,construct,negotiate,answer,peers,workers,owners});}
 finally{for(const owner of owners)await owner.shutdown();assert.ok(peers.every(peer=>peer.connectionState==='closed'&&peer.closes===1));assert.ok(workers.every(worker=>worker.terminated));assert.equal(listeners.size,0);for(const[name,value]of saved)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];}
}

test('actual optional RTC constructor owns audio duplex A and never-fed B with exact offer echo and original ontrack pointers',()=>constructedRoles(async f=>{
 const {owner,reservation,peer}=f.construct();
 assert.deepEqual(peer.transceivers.map(tx=>tx.direction),['sendrecv','sendrecv','recvonly']);
 assert.equal(peer.transceivers[1].sender.track,null);assert.equal(peer.transceivers[2].sender.track,null);assert.ok(peer.transceivers[1].sender.transform);assert.equal(peer.transceivers[2].sender.transform,null);
 const template=owner.signaling.template;
 assert.deepEqual(template.streams_offered.map(s=>[s.id,s.direction]),[['audio','sendrecv'],['camera-a','sendrecv'],['public-b','recvonly']]);
 assert.deepEqual({...template.webrtc_video_receive_reservation},wire());assert.notStrictEqual(template.webrtc_video_receive_reservation,reservation.intent);
 const ordinary=new f.BrowserRtcConnection(f.client,f.input,'participant',{webrtc_video_receive_reservation:wire()},f.original);f.owners.push(ordinary);
 assert.equal(f.peers.at(-1).transceivers.length,2,'unsigned configuration cannot enable another receive MID');
 assert.equal('webrtc_video_receive_reservation'in ordinary.signaling.template,false);
 assert.deepEqual(ordinary.signaling.template.streams_offered.map(s=>[s.id,s.direction]),[['audio','sendrecv'],['camera-a','sendonly'],['public-b','recvonly']]);
 assert.throws(()=>f.construct(reservation),protocol,'same original token cannot remint a second physical');assert.equal(peer.connectionState,'new');assert.equal(reservation.owner.owns(reservation.intent,scope()),true,'duplicate refusal must not retire the first owner');
 f.negotiate(peer);owner.receiveRoles.validateAnswer(peer.remoteDescription.sdp);
 assert.equal(owner.remote.get('camera-a'),peer.transceivers[1].receiver.track);assert.equal(owner.remote.get('public-b'),peer.transceivers[2].receiver.track);
 assert.notStrictEqual(owner.remote.get('camera-a'),owner.remote.get('public-b'));
 for(const changed of [f.answer().replace('a=mid:2','a=mid:1'),f.answer().replace('a=sendonly','a=sendrecv'),f.answer().replace(/a=rtpmap:96 VP8\/90000/g,'a=rtpmap:96 H264/90000'),f.answer()+'m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n']){
  const original=peer.remoteDescription;peer.remoteDescription={type:'answer',sdp:changed};assert.throws(()=>owner.receiveRoles.validateAnswer(changed));peer.remoteDescription=original;
 }
 await assert.rejects(()=>owner.render({audio:false},{timeoutMs:100}),{code:'unsupported-media-profile'});
 peer.ontrack({transceiver:peer.transceivers[2],receiver:peer.transceivers[2].receiver,track:{...peer.transceivers[2].receiver.track}});
 assert.equal(peer.connectionState,'closed');assert.equal(reservation.owner.owns(reservation.intent,scope()),false);
}));

test('reserved B preparation cannot submit legacy A readiness or render from an old completion',()=>constructedRoles(async f=>{
 const oldOffer=f.CanonicalWebRtcSignaling.prototype.offer,oldComplete=f.CanonicalWebRtcSignaling.prototype.completeReplacement;let legacyReady=0;
 f.CanonicalWebRtcSignaling.prototype.offer=function(){f.negotiate(this.peer);return Promise.resolve();};
 f.CanonicalWebRtcSignaling.prototype.completeReplacement=async()=>{legacyReady++;throw Error('legacy ready must never run');};
 try{
  const reservation=f.make(),begun=f.BrowserRtcConnection.beginReplacement(f.client,f.input,'participant',{},f.original,{timeoutMs:300},reservation);f.owners.push(begun.owner);
  await begun.prepared;await assert.rejects(begun.completed,{code:'unsupported-media-profile'});
  assert.equal(legacyReady,0);assert.equal(begun.owner.diagnostics.closed,true);assert.equal(reservation.owner.owns(reservation.intent,scope()),false);
  assert.equal(f.peers[0].transceivers.length,3);
  const abort=new AbortController();abort.abort();const untouched=f.make(),count=f.peers.length;
  assert.throws(()=>f.BrowserRtcConnection.beginReplacement(f.client,f.input,'participant',{},f.original,{timeoutMs:100,signal:abort.signal},untouched),{code:'cancelled'});
  assert.equal(f.peers.length,count);untouched.owner.close();
 }finally{f.CanonicalWebRtcSignaling.prototype.offer=oldOffer;f.CanonicalWebRtcSignaling.prototype.completeReplacement=oldComplete;}
}));
