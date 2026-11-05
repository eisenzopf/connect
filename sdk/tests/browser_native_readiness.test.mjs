import {test} from 'node:test';
import assert from 'node:assert/strict';
import {BrowserNativeReadiness} from '../typescript/dist/browser_native_readiness.js';
import {ConnectError} from '../typescript/dist/errors.js';
import {envelope} from '../typescript/dist/uctp_wire.js';
// Control/codec observation model only, never an actual decoder or cell pass.
const root={recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'source',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'};
const scope={cid:'room',sid:'session',connid:'target',operation_id:'move',expected_revision:0};
const challenge={nonce:'nonce',source_generation:1,target_generation:2,stream_revision:2};
function fixture({video=true,beforeReady,status='completed',changed,expire=false}={}) {
 const streams=['audio',...(video?['send','receive']:[])],marker={profile:'connect-replacement-media-v1',operation_record_id:'record',readiness:challenge};
 const parameters={substrate_setup:{kind:'native-prepared/1',revision:1,offer_id:'offer',streams:[]},replacement_preparation:structuredClone(marker)};
 const answer=envelope('connection.answer',parameters,{...scope,in_reply_to:'offer'}),callback=envelope('connection.offer',structuredClone(parameters),scope);
 changed?.({answer,callback});
 const original={substrate:'webtransport',capabilities:{},streams_offered:[{id:'audio',kind:'audio',direction:'sendrecv'},...(video?[{id:'send',kind:'video',direction:'sendonly'},{id:'receive',kind:'video',direction:'recvonly'}]:[])],replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record'}};
 const owner=new BrowserNativeReadiness(root,original,scope,answer,callback,'offer','audio',video?'receive':undefined,video?2:0);
 const audio={connectionId:'target',streamId:'audio',generation:1,ssrc:7},vi={connectionId:'target',streamId:'receive',generation:1,ssrc:8};
 let current={audio,video:video?{identity:vi,epoch:'signed-epoch'}:undefined},calls=[],attempts=0,committed=false,revision=0;
 const readiness={stream_revision:3,streams},result={activation_state:'ready',connection_id:'target',replaces:'source',endpoint_id:'endpoint',device_id:'device',generation:2,retained_connection_ids:['retained'],admitted_connection_ids:['target'],retired_connection_ids:['source'],readiness,physical_readiness:readiness};
 const client={closed:false,async requestReplacementContinuation(type,payload,sentScope){calls.push({type,payload:structuredClone(payload),scope:{...sentScope}});
  if(type==='connection.offer'){attempts++;const error=beforeReady?.(attempts,{owner,setCommitted(){committed=true;}});if(error)throw error;committed=true;}
  const state=committed?status:'preparing';return [envelope('operation.updated',{record_id:'record',operation_id:'move',command_type:'connection.replace',revision:++revision,state,native_replacement_prepared:true,...(type==='operation.get'&&state==='completed'?{result}: {})},sentScope)];
 }};
 return {owner,original,calls,client,video,vi,audio,setCurrent:v=>{current=v;},get attempts(){return attempts;},
  observe({key=true,at=performance.now(),wrongAudio=false,wrongVideo=false}={}){
   for(let i=0;i<3;i++)owner.audioDecoded({identity:wrongAudio?{...audio,streamId:'foreign'}:audio,sequence:i,timestamp:i*960,arrived:at,samples:960});
   if(video)owner.videoDecoded({identity:wrongVideo?{...vi,generation:2}:vi,epoch:'signed-epoch',arrived:at,key,width:640,height:360});
  }, finish:(timeout=200,signal)=>owner.finish(client,performance.now()+(expire?-1:timeout),()=>current,signal)};
}
const readyCalls=f=>f.calls.filter(v=>v.type==='connection.offer');
const temporary=()=>new ConnectError('refused','temporary',{code:503,category:'storage',reason:'storage_unavailable'});
test('native proof requires actual eligible outputs and preserves immutable original offer/root',async()=>{
 const f=fixture();f.observe();const result=await f.finish();assert.equal(result.state,'completed');assert.equal(f.attempts,1);
 const ready=readyCalls(f)[0];assert.deepEqual(ready.scope,scope);assert.deepEqual(ready.payload.replacement_preparation.readiness,{...challenge,audio:{stream_id:'audio',decoded_samples:2880},video:{stream_id:'receive',decoded_frames:1,decoded_keyframes:1,decoder_startup:'fresh-peer-first-decoded-frame'}});
 const original=structuredClone(ready.payload);original.replacement_preparation={profile:'connect-replacement-media-v1',operation_record_id:'record'};assert.deepEqual(original,f.original);
 assert.equal(f.calls.at(-1).type,'operation.get');assert.equal(f.calls.at(-1).scope.connid,'source');
});
for(const change of [({callback})=>callback.payload.replacement_preparation.readiness.nonce='other',({answer})=>answer.in_reply_to='other',({callback})=>callback.connid='foreign',({callback})=>callback.payload.substrate_setup.revision=2,({answer,callback})=>{answer.payload.replacement_preparation.readiness.target_generation=3;callback.payload.replacement_preparation.readiness.target_generation=3;}])
 test('native proof refuses changed signed challenge, scope, negotiation or generation',()=>assert.throws(()=>fixture({changed:change}),{code:'protocol'}));
for(const observation of [null,{key:false},{wrongAudio:true},{wrongVideo:true},{at:performance.now()-201}])
 test('native arrival/incorrect/expired/missing-key observations cannot produce a ready offer',async()=>{
  const f=fixture();if(observation)f.observe(observation);await assert.rejects(f.finish(25),{code:'timeout'});assert.equal(f.attempts,0);assert.equal(f.owner.mayHaveCommitted,false);
 });
test('same frozen native proof retries after signed temporary refusal and original pending read',async()=>{
 const f=fixture({beforeReady:attempt=>attempt===1?temporary():undefined});f.observe();await f.finish(250);
 assert.equal(f.attempts,2);assert.deepEqual(readyCalls(f)[0],readyCalls(f)[1]);
 const index=f.calls.indexOf(readyCalls(f)[1]);assert.equal(f.calls[index-1].type,'operation.get');
});
test('lost post-capture ready reply recovers completed original root without duplicate activation',async()=>{
 const f=fixture({beforeReady(_attempt,state){state.setCommitted();return new ConnectError('timeout','lost original reply');}});f.observe();await f.finish();assert.equal(f.attempts,1);assert.equal(f.owner.mayHaveCommitted,true);
});
test('frozen source or epoch change disables replay and keeps status-only uncertainty',async()=>{
 let f;f=fixture({beforeReady(){f.setCurrent({audio:f.audio,video:{identity:f.vi,epoch:'other-epoch'}});return temporary();}});f.observe();await assert.rejects(f.finish(100),{code:'timeout'});assert.equal(f.attempts,1);
});
test('definitive refusal is not hidden by late-reply recovery',async()=>{
 const problem={code:403,category:'authorization',reason:'forbidden'},f=fixture({beforeReady:()=>new ConnectError('refused','definitive',problem)});f.observe();await assert.rejects(f.finish(),e=>e.problem===problem);assert.equal(f.attempts,1);
});
test('cancellation after original ready retains uncertain owner and stops new requests',async()=>{
 const abort=new AbortController(),f=fixture({beforeReady(){abort.abort();return new ConnectError('timeout','uncertain');}});f.observe();await assert.rejects(f.finish(200,abort.signal));const count=f.calls.length;await new Promise(r=>setTimeout(r,30));assert.equal(f.calls.length,count);assert.equal(f.owner.mayHaveCommitted,true);
});
test('audio-only native challenge binds generation and requires three actual outputs without video',async()=>{
 const f=fixture({video:false});f.observe();await f.finish();assert.equal(readyCalls(f)[0].payload.replacement_preparation.readiness.video,null);
});
test('original expired deadline refuses before any signed readiness submission',async()=>{
 const f=fixture({expire:true});f.observe();await assert.rejects(f.finish(),{code:'timeout'});assert.equal(f.calls.length,0);
});
test('authoritative cancelled root allows original target retirement',async()=>{
 const f=fixture({status:'cancelled'});f.observe();await assert.rejects(f.finish(),{code:'refused'});assert.equal(f.owner.mayHaveCommitted,false);
});

test('preparation silence to real public audio SSRC change resolves only by exact original completion',async()=>{
 let f;f=fixture({beforeReady(_attempt,state){state.setCommitted();f.setCurrent({audio:{...f.audio,ssrc:99},video:{identity:f.vi,epoch:'signed-epoch'}});return new ConnectError('timeout','captured ready reply lost');}});
 f.observe();const result=await f.finish();assert.equal(result.state,'completed');assert.equal(f.attempts,1);assert.equal(f.calls.at(-1).type,'operation.get');
 assert.equal(f.owner.savedReadiness.replacement_preparation.readiness.audio.decoded_samples,2880);
});
test('first definitive post-capture refusal remains refused while its genuinely completed original owner is retained',async()=>{
 const problem={code:403,category:'authorization',reason:'forbidden'};
 const f=fixture({beforeReady(_attempt,state){state.setCommitted();return new ConnectError('refused','current authority refused after capture',problem);}});
 f.observe();await assert.rejects(f.finish(),e=>e.problem===problem);assert.equal(f.attempts,1);assert.equal(f.owner.mayHaveCommitted,true);assert.equal(f.calls.at(-1).type,'operation.get');
});
test('first definitive refusal with unresolved original root cannot close potentially captured target or replay',async()=>{
 const problem={code:403,category:'authorization',reason:'forbidden'},f=fixture({beforeReady:()=>new ConnectError('refused','uncertain capture boundary',problem)});
 f.observe();await assert.rejects(f.finish(35),e=>e.problem===problem);assert.equal(f.attempts,1);assert.equal(f.owner.mayHaveCommitted,true);f.owner.close();
 await assert.rejects(f.finish(),{code:'closed'});
});

test('source change before first ready requires fresh current decoder observations',async()=>{
 const f=fixture();f.observe();f.setCurrent({audio:{...f.audio,ssrc:99},video:{identity:f.vi,epoch:'signed-epoch'}});
 await assert.rejects(f.finish(25),{code:'timeout'});assert.equal(f.attempts,0);assert.equal(f.owner.mayHaveCommitted,false);
});
test('duplicate original PCM output cannot create three-frame readiness',async()=>{
 const f=fixture({video:false}),arrived=performance.now();
 for(let i=0;i<3;i++)f.owner.audioDecoded({identity:f.audio,sequence:1,timestamp:960,arrived,samples:960});
 await assert.rejects(f.finish(25),{code:'timeout'});assert.equal(f.attempts,0);
});
