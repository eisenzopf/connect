import {nativeIdentity,bindNativeStreams,nativeBoundary} from './native_activation_fixture.mjs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserMediaPeer} from '../typescript/dist/browser_media.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';

// Signed control and codec-callback ownership model only; no browser media pass.
const root=()=>({recordId:'record',operationId:'original-move',conversationId:'room',sessionId:'session',sourceConnectionId:'old',retainedConnectionId:'remote',targetConnectionId:'replacement_record',endpointId:'endpoint',deviceId:'device'});
const input=()=>({conversationId:'room',sessionId:'session',connectionId:'replacement_record',operationId:'original-move',audio:'audio',video:'camera',receiveVideo:'incoming-camera',nativeVideoFeedback:true});
async function fixture(run,{mode,generation=2}={}) {
 const names=['AudioEncoder','AudioDecoder','AudioData','EncodedAudioChunk','VideoEncoder','VideoDecoder','VideoFrame','EncodedVideoChunk'];
 const originals=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);
 class Frame {constructor(value){Object.assign(this,value);this.sampleRate=48000;this.numberOfChannels=1;this.numberOfFrames=960;this.displayWidth=640;this.displayHeight=360;}close(){}}
 class Encoder {static async isConfigSupported(){return {supported:true};}state='unconfigured';encodeQueueSize=0;decodeQueueSize=0;constructor(callbacks){this.callbacks=callbacks;}configure(){this.state='configured';}close(){this.state='closed';}reset(){}
  encode(frame){queueMicrotask(()=>this.callbacks.output({timestamp:frame.timestamp,type:'key',byteLength:1,copyTo:bytes=>bytes.set([9])}));}}
 class Decoder extends Encoder {decode(chunk){queueMicrotask(()=>{if(this.state!=='closed')this.callbacks.output(new Frame({timestamp:chunk.timestamp}));});}}
 const values=[Encoder,Decoder,Frame,Frame,Encoder,Decoder,Frame,Frame];
 names.forEach((name,i)=>Object.defineProperty(globalThis,name,{configurable:true,value:values[i]}));
 const sent=[],events=new Set(),lifetimes=new Set();let complete=false,reads=0,parameters,reader;
 const status=()=>({record_id:'record',operation_id:'original-move',command_type:'connection.replace',expected_revision:7,state:complete?(mode==='cancelled'?'cancelled':'completed'):'preparing',revision:complete?8:3,native_replacement_prepared:mode!=='not-prepared',
  ...(complete&&mode!=='cancelled'?{result:{activation_state:mode==='stopped'?'stopped':'ready',connection_id:'replacement_record',replaces:'old',endpoint_id:'endpoint',device_id:'device',generation:mode==='receipt-generation'?generation+1:generation,retained_connection_ids:['remote'],admitted_connection_ids:['replacement_record'],retired_connection_ids:['old'],readiness:{stream_revision:6,streams:['audio','camera','incoming-camera']},physical_readiness:{stream_revision:6,streams:['audio','camera','incoming-camera']},native_activation:nativeBoundary(root(),parameters.substrate_setup.streams,generation,6)}}:{})});
 const client={closed:false,controlTransport:'webtransport',deviceId:'device',
  datagrams:{maxDatagramSize:1200,readable:new ReadableStream({start(c){reader=c;}}),writable:new WritableStream()},
  onEvent:fn=>{events.add(fn);return()=>events.delete(fn);},onClose:fn=>{lifetimes.add(fn);return()=>lifetimes.delete(fn);},
  close(){if(this.closed)return;this.closed=true;for(const fn of [...lifetimes])fn(Error('closed'));},
  requestReplacementContinuation(...args){return this.request(...args);},
  async request(type,payload,scope){sent.push({type,payload:structuredClone(payload),scope:{...scope}});
   if(type==='operation.get'){reads++;const value=status();if(mode==='changed-root'&&reads>1)value.record_id='foreign';return [envelope('operation.updated',value,scope),envelope('ack',{},scope)];}
   assert.equal(type,'connection.offer');const phase=payload.replacement_preparation.profile;
   if(phase==='connect-replacement-preparation-v1') {if(mode==='lost-allocation')throw new ConnectError('timeout','allocation response lost');}
   else {assert.equal(phase,'connect-replacement-ready-v1');assert.equal(payload.replacement_preparation.readiness.audio.decoded_samples,2880);assert.equal(payload.replacement_preparation.readiness.video.decoded_keyframes,1);if(mode!=='uncertain')complete=true;}
   const value=status();delete value.result;return [envelope('operation.updated',value,scope),envelope('ack',{},scope)];
  },
  begin(type,payload,scope){sent.push({type,payload:structuredClone(payload),scope:{...scope}});const id='media-request';
   const streams=payload.streams_offered.map((stream,index)=>({strm_id:stream.id,kind:stream.kind,direction:mode==='wrong-direction'&&stream.id==='incoming-camera'?'recvonly':stream.direction==='sendonly'?'recvonly':stream.direction==='recvonly'?'sendonly':'sendrecv',stream_local_id:index+1,codec:{name:stream.kind==='audio'?'opus':'vp8',params:{sample_rate:stream.kind==='audio'?48000:90000,channels:stream.kind==='audio'?1:0,payload_type:stream.kind==='audio'?111:96}}}));
   parameters={by_participant:'participant',substrate:'webtransport',capabilities:{video_feedback:'rtcp-pli-epoch/1'},substrate_setup:{kind:'native-prepared/1',revision:1,offer_id:id,streams},replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record',readiness:{nonce:'nonce',source_generation:generation-1,target_generation:generation,stream_revision:2}}};
   for(const stream of streams.filter(s=>s.kind==='video'))stream.video_feedback={profile:'rtcp-pli-epoch/1',original_operation:'original-move',original_offer:id,stream_generation:1,connection_generation:mode==='mixed-generation'&&stream.strm_id==='incoming-camera'?generation+1:generation};
   bindNativeStreams(streams,generation);
   const answer=structuredClone(parameters);if(mode==='changed-challenge')answer.replacement_preparation.readiness.nonce='other';
   queueMicrotask(()=>{for(const fn of events)fn(envelope('connection.offer',parameters,scope));});
   return {id,response:Promise.resolve([envelope('connection.answer',answer,{...scope,in_reply_to:id})])};
  },
  async send(type,payload,scope){sent.push({type,payload:structuredClone(payload),scope:{...scope}});},
  async operationStatus(){return {operation_id:'server-reconcile',state:'completed'};},
 };
 const peer=new BrowserMediaPeer(client);
 const decoded=()=>{
  const epoch='a'.repeat(32);for(const fn of events)fn(envelope('stream.epoch',{profile:'rtcp-pli-epoch/1',strm_id:'incoming-camera',stream_local_id:3,original_operation:'original-move',original_offer:'media-request',incarnation:'b'.repeat(32),stream_generation:1,connection_generation:generation,revision:1,direction:'receiver',epoch,ssrc:8,first_sequence:1,first_timestamp:3000},{cid:'room',sid:'session',connid:'replacement_record'}));
  const packet=(audio,seq)=>{const offset=audio?8:24,payload=audio?[1]:[0x10,0,1,2],wire=new Uint8Array(offset+12+payload.length),v=new DataView(wire.buffer);wire[0]=1;wire[1]=audio?0:1;v.setUint16(2,audio?1:3);v.setUint32(4,seq);if(!audio)wire.set(Buffer.from(epoch,'hex'),8);wire[offset]=0x80;wire[offset+1]=(audio?111:96)|128;v.setUint16(offset+2,seq);v.setUint32(offset+4,audio?seq*960:3000);v.setUint32(offset+8,audio?7:8);wire.set(payload,offset+12);return wire;};
  for(let i=1;i<=3;i++)reader.enqueue(packet(true,i));reader.enqueue(packet(false,1));
 };
 try {await run({peer,client,sent,decoded,reconcile(){const payload=structuredClone(parameters);payload.substrate_setup.revision=7;payload.substrate_setup.offer_id='server-request';delete payload.replacement_preparation;for(const fn of events)fn(envelope('connection.offer',payload,{cid:'room',sid:'session',connid:'replacement_record',operation_id:'server-reconcile'}));}});}
 finally {await peer.shutdown();for(const[name,d]of originals)if(d)Object.defineProperty(globalThis,name,d);else delete globalThis[name];}
}
test('native replacement exposes prepared IO, then actual decoder output drives one original ready continuation',()=>fixture(async({peer,sent,decoded,reconcile})=>{
 const original=root(),offer=input(),begun=peer.beginReplacementConnection(offer,original,{timeoutMs:500});original.recordId='changed';offer.audio='changed';let finished=false;void begun.completed.then(()=>{finished=true;});
 const prepared=await begun.prepared;assert.equal(finished,false);assert.equal(prepared.revision,1);assert.deepEqual(prepared.streams.map(s=>[s.streamId,s.direction]),[['audio','sendrecv'],['camera','sendonly'],['incoming-camera','recvonly']]);
 prepared.encodeAudio('audio',new Float32Array(960),100000);await new Promise(r=>setTimeout(r,5));assert.equal(sent.filter(f=>f.payload?.replacement_preparation?.profile==='connect-replacement-ready-v1').length,0,'capture/packet write cannot prove decoded readiness');
 decoded();assert.deepEqual(await begun.completed,{recordId:'record',generation:2,streamRevision:6});assert.equal(prepared.revision,6);
 const offers=sent.filter(f=>f.type==='connection.offer');assert.deepEqual(offers.map(f=>f.payload.replacement_preparation.profile),['connect-replacement-preparation-v1','connect-replacement-media-v1','connect-replacement-ready-v1']);
 assert.deepEqual(offers[0].scope,offers[1].scope);assert.deepEqual(offers[1].scope,offers[2].scope);
 for(const offer of offers){const actual=structuredClone(offer.payload);delete actual.replacement_preparation;const expected=structuredClone(offers[0].payload);delete expected.replacement_preparation;assert.deepEqual(actual,expected);}
 assert.equal(sent.filter(f=>f.type==='connection.answer').length,1);assert.ok(sent.filter(f=>f.type==='operation.get').every(f=>f.scope.connid==='old'));
 reconcile();const until=performance.now()+200;while(prepared.revision!==7){assert.ok(performance.now()<until);await new Promise(r=>setTimeout(r,1));}
 assert.equal(sent.filter(f=>f.type==='connection.answer'&&f.scope.operation_id==='server-reconcile').length,1);
}));
for(const [mode,code,count]of [['not-prepared','conflict',0],['lost-allocation','timeout',1],['wrong-direction','protocol',2],['changed-challenge','protocol',2],['mixed-generation','protocol',2]])
 test(`native replacement ${mode} refuses without another original allocation or leaked target`,()=>fixture(async({peer,sent})=>{const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:200});await assert.rejects(begun.prepared,{code});await assert.rejects(begun.completed,{code});assert.equal(sent.filter(f=>f.type==='connection.offer').length,count);assert.equal(peer.diagnostics.activeBindings,0);},{mode}));
for(const generation of [2,3])test(`native actual decoded challenge and final receipt bind generation ${generation}`,()=>fixture(async({peer,decoded})=>{const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:500});await begun.prepared;decoded();assert.equal((await begun.completed).generation,generation);},{generation}));
for(const mode of ['changed-root','receipt-generation','stopped','cancelled'])test(`native final ${mode} never reports a successful partial route`,()=>fixture(async({peer,decoded})=>{const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:100});await begun.prepared;decoded();await assert.rejects(begun.completed,{code:['stopped','cancelled'].includes(mode)?'refused':'protocol'});assert.equal(peer.diagnostics.activeBindings,0);},{mode}));
test('native uncertainty after actual ready preserves original target for recovery without any new mutation',()=>fixture(async({peer,client,sent,decoded})=>{const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:80}),connection=await begun.prepared;let closed=0;connection.onClose(()=>closed++);decoded();await assert.rejects(begun.completed,{code:'timeout'});assert.equal(peer.replacementMayHaveCommitted(connection.id),true);assert.equal(peer.diagnostics.activeBindings,3);assert.equal(client.closed,false);assert.equal(closed,0);assert.equal(sent.filter(f=>f.type==='connection.offer').length,3);assert.equal(sent.filter(f=>['connection.replace','connection.end'].includes(f.type)).length,0);},{mode:'uncertain'}));
test('native missing decoded readiness expires with exact original target cleanup',()=>fixture(async({peer,client,sent})=>{const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:50});await begun.prepared;await assert.rejects(begun.completed,{code:'timeout'});assert.equal(peer.diagnostics.activeBindings,0);assert.equal(client.closed,true);assert.equal(sent.filter(f=>f.type==='connection.offer').length,2);}));
test('native mapped video cannot substitute untagged history for authenticated decoder readiness',()=>fixture(async({peer})=>{assert.throws(()=>peer.beginReplacementConnection({...input(),nativeVideoFeedback:false},root()),{code:'invalid-request'});}));

test('native queued reconciliation cannot retire an unresolved original ready owner',()=>fixture(async({peer,client,decoded,reconcile})=>{
 const begun=peer.beginReplacementConnection(input(),root(),{timeoutMs:80}),connection=await begun.prepared;
 decoded();reconcile();await assert.rejects(begun.completed,{code:'timeout'});
 await new Promise(resolve=>setTimeout(resolve,5));
 assert.equal(peer.replacementMayHaveCommitted(connection.id),true);assert.equal(peer.diagnostics.activeBindings,3);assert.equal(client.closed,false);
},{mode:'uncertain'}));
