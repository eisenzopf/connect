import assert from 'node:assert/strict';
import {test} from 'node:test';
import {BrowserWebRtcReplacement} from '../typescript/dist/browser_webrtc_replacement.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'source',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'};
const scope={cid:'room',sid:'session',connid:'target',operation_id:'move',expected_revision:0};
const payload=()=>({by_participant:'participant',substrate:'websocket+webrtc',substrate_setup:{kind:'websocket+webrtc',sdp:'v=0\r\n'},capabilities:{},streams_offered:[{id:'audio',kind:'audio',direction:'sendrecv',codec_preferences:['opus']},{id:'send',kind:'video',direction:'sendonly',codec_preferences:['vp8']},{id:'receive',kind:'video',direction:'recvonly',codec_preferences:['vp8']}]});
const challenge=()=>({nonce:'nonce',source_generation:1,target_generation:2,stream_revision:2});
const answer=()=>({replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record',readiness:challenge()}});
const evidence=()=>({audio:{stream_id:'audio',decoded_samples:1920},video:{stream_id:'receive',decoded_frames:2,decoded_keyframes:1,decoder_startup:'fresh-peer-first-decoded-frame'}});
const ready={stream_revision:6,streams:['audio','send','receive'],participant_revision:2};
const result=()=>({activation_state:'ready',connection_id:'target',replaces:'source',generation:2,endpoint_id:'endpoint',device_id:'device',retained_connection_ids:['retained'],admitted_connection_ids:['target'],retired_connection_ids:['source'],readiness:structuredClone(ready),physical_readiness:structuredClone(ready)});
function fixture({loseReady=false,terminal='completed',changeResult,beforeReady,pendingState='preparing',pendingPrepared=true,statusFailure}={}) {
 let revision=0,committed=false,attempts=0,commits=0;const requests=[];
 const record=(connection,state,{receipt=true,prepared=true}={})=>envelope('operation.updated',{operation_id:'move',record_id:'record',command_type:'connection.replace',revision:++revision,state,native_replacement_prepared:prepared,...(state==='completed'&&receipt?{result:changeResult?.(result())??result()}: {})},{cid:'room',sid:'session',connid:connection});
 const client={controlTransport:'websocket',deviceId:'device',closed:false,requestReplacementContinuation(...args){return this.request(...args);},async request(type,body,sentScope){
  const request={type,body:structuredClone(body),scope:structuredClone(sentScope)};requests.push(request);
  if(type==='operation.get'){
   if(attempts){const failure=statusFailure?.();if(failure)throw failure;}
   const state=committed?terminal:attempts?pendingState:'preparing',prepared=committed||!attempts||pendingPrepared;
   request.observed={state,prepared};return [record('source',state,{prepared})];
  }
  if(body.replacement_preparation.profile==='connect-replacement-preparation-v1')return [record('target','preparing')];
  assert.equal(body.replacement_preparation.profile,'connect-replacement-ready-v1');attempts++;
  const failure=beforeReady?.(attempts);if(failure)throw failure;
  committed=true;commits++;
  if(loseReady)throw new ConnectError('transport','synthetic lost ready reply');
  // The real service deliberately strips results from every ready acknowledgement.
  return [record('target',terminal,{receipt:false})];
 }};
 return {client,requests,owner:new BrowserWebRtcReplacement(client,root),get attempts(){return attempts;},get commits(){return commits;}};
}
const readyRequests=f=>f.requests.filter(v=>v.type==='connection.offer'&&v.body.replacement_preparation.profile==='connect-replacement-ready-v1');
const transient=(category,reason)=>new ConnectError('refused','temporary pre-capture failure',{code:503,category,reason});
async function prepared(f){await f.owner.allocate(payload(),scope,{timeoutMs:200});f.owner.mediaOffer(payload());f.owner.answer(answer());}
test('RTC replacement sends allocation, media and decoded readiness under one immutable original root',async()=>{
 const f=fixture(),original=payload();await f.owner.allocate(original,scope,{timeoutMs:200});const media=f.owner.mediaOffer(original);original.streams_offered[0].id='mutated';assert.equal(media.streams_offered[0].id,'audio');assert.equal(media.replacement_preparation.profile,'connect-replacement-media-v1');f.owner.answer(answer());
 const receipt=await f.owner.complete(evidence(),{timeoutMs:200});assert.deepEqual(receipt,{recordId:'record',generation:2,streamRevision:6});
 assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,2);assert.ok(f.requests.filter(v=>v.type==='connection.offer').every(v=>v.scope.operation_id==='move'&&v.scope.connid==='target'));
 const sent=readyRequests(f).at(-1).body;assert.deepEqual(sent.replacement_preparation.readiness,{...challenge(),...evidence()});assert.equal(sent.streams_offered[0].id,'audio');
 const saved=f.owner.savedReadiness;saved.replacement_preparation.readiness.nonce='forged';assert.equal(f.owner.savedReadiness.replacement_preparation.readiness.nonce,'nonce');
 await assert.rejects(f.owner.complete(evidence(),{timeoutMs:200}),{code:'conflict'});
});
for(const alter of [p=>p.substrate_setup.sdp='v=0\r\nchanged',p=>p.streams_offered[0].id='other'])test('RTC replacement cannot change SDP or logical ports after allocation',async()=>{
 const f=fixture();await f.owner.allocate(payload(),scope,{timeoutMs:200});const changed=payload();alter(changed);assert.throws(()=>f.owner.mediaOffer(changed),{code:'protocol'});assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,1);
});
for(const alter of [a=>a.replacement_preparation.operation_record_id='foreign',a=>a.replacement_preparation.readiness.target_generation=3,a=>a.replacement_preparation.readiness.nonce='',a=>a.replacement_preparation.readiness.stream_revision=0])test('RTC replacement refuses a foreign or malformed decoder challenge before readiness submission',async()=>{
 const f=fixture();await f.owner.allocate(payload(),scope,{timeoutMs:200});const changed=answer();alter(changed);assert.throws(()=>f.owner.answer(changed),{code:'protocol'});assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,1);
});
for(const alter of [e=>e.audio.decoded_samples=0,e=>e.video.decoded_keyframes=0,e=>e.video.decoded_keyframes=3,e=>e.video.stream_id='send',e=>e.video=null])test('RTC readiness cannot use socket success or wrong/undecoded streams',async()=>{
 const f=fixture();await prepared(f);const changed=evidence();alter(changed);await assert.rejects(f.owner.complete(changed,{timeoutMs:200}),{code:'protocol'});assert.equal(f.owner.mayHaveCommitted,false);assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,1);
});
test('RTC lost ready response recovers original status without another mutation',async()=>{
 const f=fixture({loseReady:true});await prepared(f);const receipt=await f.owner.complete(evidence(),{timeoutMs:200});assert.equal(receipt.generation,2);assert.equal(f.requests.at(-1).type,'operation.get');assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,2);assert.equal(f.owner.mayHaveCommitted,true);
});
test('RTC definitive original cancellation permits target retirement',async()=>{
 const f=fixture({terminal:'cancelled'});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:200}),{code:'refused'});assert.equal(f.owner.mayHaveCommitted,false);
});
for(const changeResult of [r=>({...r,generation:3}),r=>({...r,endpoint_id:'other'}),r=>({...r,readiness:{...r.readiness,streams:['audio','send','foreign']},physical_readiness:{...r.readiness,streams:['audio','send','foreign']}})])test('RTC completion cannot adopt a foreign committed generation or target stream set',async()=>{
 const f=fixture({changeResult});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:200}),{code:'protocol'});assert.equal(f.owner.mayHaveCommitted,true);
});

test('RTC explicit fresh-decoder fallback retains unavailable keyframe counter as null',async()=>{
 const f=fixture();await prepared(f);const value=evidence();value.video.decoded_keyframes=null;
 await f.owner.complete(value,{timeoutMs:200});assert.equal(readyRequests(f).at(-1).body.replacement_preparation.readiness.video.decoded_keyframes,null);
});
test('RTC uncertain readiness deadline retains original target for status-only recovery',async()=>{
 const f=fixture({terminal:'preparing'});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:25}),{code:'timeout'});
 assert.equal(f.owner.mayHaveCommitted,true);assert.equal(f.requests.filter(v=>v.type==='connection.offer').length,2);
});

for(const [category,reason] of [['authorization','authority_unavailable'],['storage','storage_unavailable']])test(`RTC readiness retries frozen evidence after current original status proves pending: ${reason}`,async()=>{
 const value=evidence(),f=fixture({beforeReady(attempt){if(attempt===1){value.audio.decoded_samples=777;value.video.stream_id='changed';return transient(category,reason);}}});await prepared(f);
 const receipt=await f.owner.complete(value,{timeoutMs:500});assert.equal(receipt.generation,2);assert.equal(f.attempts,2);assert.equal(f.commits,1);
 const offers=readyRequests(f);assert.deepEqual(offers[0].body,offers[1].body);assert.deepEqual(offers[0].scope,offers[1].scope);assert.equal(offers[1].body.replacement_preparation.readiness.audio.decoded_samples,1920);assert.equal(offers[1].body.replacement_preparation.readiness.video.stream_id,'receive');
 const index=f.requests.indexOf(offers[1]);assert.equal(f.requests[index-1].type,'operation.get');assert.deepEqual(f.requests[index-1].observed,{state:'preparing',prepared:true});assert.equal(f.requests.at(-1).type,'operation.get');
});
test('RTC ready acknowledgement without a result is followed by authoritative original receipt',async()=>{
 const f=fixture();await prepared(f);assert.equal((await f.owner.complete(evidence(),{timeoutMs:200})).generation,2);assert.equal(f.requests.at(-1).type,'operation.get');assert.equal(f.attempts,1);assert.equal(f.commits,1);
});
for(const problem of [{code:403,category:'authorization',reason:'forbidden'},{code:409,category:'state',reason:'revision_conflict'},{code:400,category:'state',reason:'operation_rejected'}])test(`RTC readiness does not replay definitive refusal ${problem.reason}`,async()=>{
 const f=fixture({beforeReady:()=>new ConnectError('refused','definitive',problem)});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:200}),error=>error.problem===problem);assert.equal(f.attempts,1);assert.equal(f.commits,0);
});
for(const [pendingState,pendingPrepared] of [['unknown',true],['interrupted',true],['committing',true],['preparing',false]])test(`RTC readiness does not replay without current prepared status: ${pendingState}/${pendingPrepared}`,async()=>{
 const f=fixture({pendingState,pendingPrepared,beforeReady:()=>transient('storage','storage_unavailable')});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:120}),{code:'timeout'});assert.equal(f.attempts,1);assert.equal(f.commits,0);assert.equal(f.owner.mayHaveCommitted,true);
});
test('RTC uncertain status reads cannot authorize ready replay',async()=>{
 const f=fixture({beforeReady:()=>transient('storage','storage_unavailable'),statusFailure:()=>transient('authorization','authority_unavailable')});await prepared(f);await assert.rejects(f.owner.complete(evidence(),{timeoutMs:120}),{code:'timeout'});assert.equal(f.attempts,1);assert.equal(f.commits,0);assert.equal(f.owner.mayHaveCommitted,true);
});
test('RTC readiness retry shares the original deadline and stops immediately on cancellation',async()=>{
 const abort=new AbortController();const f=fixture({beforeReady(attempt){if(attempt===2)abort.abort();return transient('storage','storage_unavailable');}});await prepared(f);
 await assert.rejects(f.owner.complete(evidence(),{timeoutMs:1000,signal:abort.signal}));const count=f.requests.length;await new Promise(resolve=>setTimeout(resolve,80));assert.equal(f.requests.length,count);assert.equal(f.attempts,2);assert.equal(f.commits,0);
});
test('RTC repeated temporary refusal has a finite eight-attempt recovery window',async()=>{
 const f=fixture({beforeReady:()=>transient('storage','storage_unavailable')});await prepared(f);const started=performance.now();await assert.rejects(f.owner.complete(evidence(),{timeoutMs:4200}),{code:'timeout'});assert.equal(f.attempts,8);assert.equal(f.commits,0);assert.ok(performance.now()-started<4700);assert.equal(f.owner.mayHaveCommitted,true);
});
