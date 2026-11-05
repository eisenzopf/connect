import assert from 'node:assert/strict';
import {test} from 'node:test';
import {fixture} from './fixtures/browser_control_peer.mjs';
import {BrowserClient} from '../typescript/dist/browser_client.js';
import {BrowserRtcConnection} from '../typescript/dist/browser_sdk_webrtc.js';
const sdp='v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=Original replacement\r\nt=0 0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=sendrecv\r\na=mid:audio\r\na=rtpmap:111 opus/48000/2\r\n';
const tick=()=>new Promise(resolve=>setImmediate(resolve));
class Track{kind='audio';readyState='live';id='rtc-audio';stop(){this.readyState='ended';}}
async function scenario(run,{mode='complete',timeout=250}={}) {
 const saved=new Map(),peers=[],worklets=[],contexts=[];
 class Stream{constructor(tracks){this.tracks=tracks;}getTracks(){return this.tracks;}getAudioTracks(){return this.tracks;}getVideoTracks(){return[];}}
 class Peer extends EventTarget {
  connectionState='new';iceConnectionState='new';iceGatheringState='complete';transceivers=[];
  constructor(configuration){super();this.configuration=configuration;peers.push(this);}
  getConfiguration(){return this.configuration??{};}
  addTransceiver(){const value={sender:{track:null,async replaceTrack(t){this.track=t;}},receiver:{track:new Track(),jitterBufferTarget:0},setCodecPreferences(){}};this.transceivers.push(value);return value;}
  getTransceivers(){return this.transceivers;}
  async createOffer(){return{type:'offer',sdp};}
  async setLocalDescription(value){this.localDescription=value;}
  async setRemoteDescription(value){this.remoteDescription=value;this.ontrack?.({track:this.transceivers[0].receiver.track,transceiver:this.transceivers[0]});this.connectionState='connected';this.iceConnectionState='connected';this.dispatchEvent(new Event('connectionstatechange'));}
  async getStats(){return new Map([['audio',{type:'inbound-rtp',kind:'audio',trackIdentifier:'rtc-audio',totalSamplesReceived:1920,concealedSamples:0}]]);}
  close(){this.connectionState='closed';this.dispatchEvent(new Event('connectionstatechange'));}
 }
 class Node{gain={value:1};connect(){}disconnect(){}}
 class Context{sampleRate=48000;destination={};audioWorklet={async addModule(){}};closed=false;constructor(){contexts.push(this);}createMediaStreamSource(){return new Node();}createGain(){return new Node();}async resume(){}async close(){this.closed=true;}}
 class Worklet extends Node{port={onmessage:null,closed:false,postMessage(){},close(){this.closed=true;}};constructor(){super();worklets.push(this);}}
 const document=Object.assign(new EventTarget(),{visibilityState:'visible',createElement(){return{muted:false,srcObject:null,async play(){},pause(){}};}});
 const values={RTCPeerConnection:Peer,RTCRtpSender:{getCapabilities:()=>({codecs:[{mimeType:'audio/opus',clockRate:48000,channels:2}]})},MediaStream:Stream,AudioContext:Context,AudioWorkletNode:Worklet,document};
 for(const[name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,writable:true,value});}
 let committed=false,revision=0,readyAttempts=0,lateReply;
 const record=(state)=>({operation_id:'move',record_id:'record',command_type:'connection.replace',revision:++revision,state,native_replacement_prepared:true,...(state==='completed'?{result:{activation_state:'ready',connection_id:'target',replaces:'source',generation:2,endpoint_id:'endpoint',device_id:'wss_device',retained_connection_ids:['retained'],admitted_connection_ids:['target'],retired_connection_ids:['source'],readiness:{stream_revision:3,streams:['audio']},physical_readiness:{stream_revision:3,streams:['audio']}}}:{})});
 try{await fixture(async f=>{
  const client=await BrowserClient.open(f.options);let source,target;
  try{
   source=BrowserRtcConnection.begin(client,{conversationId:'room',sessionId:'session',connectionId:'source',operationId:'source-offer',audio:'source-audio'},'recipient',{}, {timeoutMs:250});await source.ready;
   const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'source',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'wss_device'};
   target=BrowserRtcConnection.beginReplacement(client,{conversationId:'room',sessionId:'session',connectionId:'target',operationId:'move',audio:'audio'},'recipient',{},root,{timeoutMs:timeout});
   await run({client,source,target,f,peers,contexts,worklets,lateReply:()=>lateReply?.(),async decoded(){while(!worklets.length)await tick();worklets[0].port.onmessage({data:{epoch:0,timestampUs:1000,samples:new Float32Array(960)}});}});
  }finally{await target?.owner.shutdown();await source?.owner.shutdown();client.close();}
 },{handler:async({peer,frame})=>{
  const reply=async(type,payload)=>{await peer.emit(type,payload,frame.id,frame);await peer.emit('ack',{},frame.id,frame);return true;};
  if(frame.type==='operation.get')return reply('operation.updated',record(committed?'completed':'preparing'));
  if(frame.type!=='connection.offer')return false;
  const profile=frame.payload.replacement_preparation?.profile;
  if(profile==='connect-replacement-preparation-v1')return mode==='lost-allocation'?true:reply('operation.updated',record('preparing'));
  if(profile==='connect-replacement-media-v1'&&mode==='lost-media')return true;
  if(profile==='connect-replacement-ready-v1'){
   readyAttempts++;
   if(mode==='transient-ready'&&readyAttempts===1)return reply('error',{code:503,category:'storage',reason:'storage_unavailable'});
   committed=true;await peer.emit('connection.end',{},undefined,{cid:'room',sid:'session',connid:'source'});
   if(mode==='lost-ready'){lateReply=()=>peer.emit('connection.end',{},frame.id,{cid:'room',sid:'session',connid:'target'});return true;}
   const acknowledgement=record('completed');delete acknowledgement.result;return reply('operation.updated',acknowledgement);
  }
  return reply('connection.answer',{substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp},capabilities:{audio_codecs:[{name:'opus',params:{sample_rate:48000,channels:1}}]},streams_answered:[{id:frame.connid==='source'?'source-audio':'audio',kind:'audio',direction:'sendrecv',codec:{name:'opus',params:{sample_rate:48000,channels:1}}}],...(profile?{replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record',readiness:{nonce:'nonce',source_generation:1,target_generation:2,stream_revision:2}}}: {})});
 }});}finally{for(const[name,descriptor]of saved){if(descriptor)Object.defineProperty(globalThis,name,descriptor);else delete globalThis[name];}}
}
test('second RTC owner keeps the original connected until decoded readiness and original committed receipt',()=>scenario(async f=>{
 await f.target.prepared;assert.equal(f.peers.length,2);assert.equal(f.source.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.target.owner.replacementMayHaveCommitted,false);
 assert.equal(f.f.sent.filter(v=>v.payload.replacement_preparation?.profile==='connect-replacement-ready-v1').length,0);
 await f.decoded();const receipt=await f.target.completed;assert.equal(receipt.generation,2);assert.equal(f.target.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.source.owner.diagnostics.peerConnectionState,'closed');assert.equal(f.client.closed,false);
}));
test('target decoded readiness failure retires the target and leaves the original playable',()=>scenario(async f=>{
 await f.target.prepared;await assert.rejects(f.target.completed,{code:'timeout'});assert.equal(f.target.owner.diagnostics.closed,true);assert.equal(f.source.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.source.owner.diagnostics.liveRemoteTracks,1);assert.equal(f.client.closed,false);assert.ok(f.contexts.every(c=>c.closed));assert.ok(f.worklets.every(w=>w.port.closed));
},{timeout:70}));
test('lost ready reply after commit preserves the target and recovers status under the original operation',()=>scenario(async f=>{
 await f.target.prepared;await f.decoded();const receipt=await f.target.completed;assert.equal(receipt.generation,2);assert.equal(f.source.owner.diagnostics.closed,true);assert.equal(f.target.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.target.owner.replacementMayHaveCommitted,true);
 await f.lateReply();await tick();assert.equal(f.target.owner.diagnostics.peerConnectionState,'connected');
 const status=await f.client.operationStatus('room','move',{timeoutMs:200});assert.equal(status.state,'completed');assert.equal(f.f.sent.filter(v=>v.payload.replacement_preparation?.profile==='connect-replacement-ready-v1').length,1);assert.equal(f.client.closed,false);
},{mode:'lost-ready',timeout:90}));

for(const mode of ['lost-allocation','lost-media'])test(`${mode} reply retires only the second peer and preserves the original control/media`,()=>scenario(async f=>{
 await assert.rejects(f.target.prepared,{code:'timeout'});await assert.rejects(f.target.completed,{code:'timeout'});assert.equal(f.target.owner.diagnostics.closed,true);assert.equal(f.source.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.client.closed,false);
},{mode,timeout:70}));

test('unanswered original replacement reads retain a bounded late-reply window without retiring control',()=>fixture(async f=>{
 const client=await BrowserClient.open(f.options);
 try {
  for(let batch=0;batch<4;batch++) {
   const waits=Array.from({length:16},()=>client.requestReplacementContinuation('operation.get',{operation_id:'move'},{cid:'room',sid:'session',connid:'source'},{timeoutMs:40}));
   const outcomes=await Promise.allSettled(waits);assert.ok(outcomes.every(value=>value.status==='rejected'&&value.reason.code==='timeout'));assert.equal(client.closed,false);
  }
  assert.throws(()=>client.requestReplacementContinuation('operation.get',{operation_id:'move'},{cid:'room',sid:'session',connid:'source'},{timeoutMs:40}),{code:'backpressure'});assert.equal(client.closed,false);
 } finally {await client.shutdown();}
},{handler:async({frame})=>frame.type==='operation.get'}));

test('signed temporary readiness refusal recovers with identical proof under the original target and operation',()=>scenario(async f=>{
 await f.target.prepared;await f.decoded();const receipt=await f.target.completed;assert.equal(receipt.generation,2);
 const offers=f.f.sent.filter(v=>v.payload.replacement_preparation?.profile==='connect-replacement-ready-v1');assert.equal(offers.length,2);assert.deepEqual(offers[0].payload,offers[1].payload);
 for(const offer of offers){assert.equal(offer.connid,'target');assert.equal(offer.operation_id,'move');assert.equal(offer.expected_revision,0);}
 const index=f.f.sent.indexOf(offers[1]);assert.equal(f.f.sent[index-1].type,'operation.get');assert.equal(f.f.sent[index-1].connid,'source');assert.equal(f.peers.length,2);assert.equal(f.source.owner.diagnostics.closed,true);assert.equal(f.target.owner.diagnostics.peerConnectionState,'connected');assert.equal(f.client.closed,false);
},{mode:'transient-ready',timeout:500}));
