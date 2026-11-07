import {nativeIdentity,bindNativeStreams,nativeBoundary} from './native_activation_fixture.mjs';
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
 const parameters={substrate_setup:{kind:'native-prepared/1',revision:1,offer_id:'offer',streams:bindNativeStreams([{strm_id:'audio',kind:'audio',direction:'sendrecv',stream_local_id:1},...(video?[{strm_id:'receive',kind:'video',direction:'sendonly',stream_local_id:2}]:[])],2)},replacement_preparation:structuredClone(marker)};
 const answer=envelope('connection.answer',parameters,{...scope,in_reply_to:'offer'}),callback=envelope('connection.offer',structuredClone(parameters),scope);
 changed?.({answer,callback});
 const original={substrate:'webtransport',capabilities:{},streams_offered:[{id:'audio',kind:'audio',direction:'sendrecv'},...(video?[{id:'send',kind:'video',direction:'sendonly'},{id:'receive',kind:'video',direction:'recvonly'}]:[])],replacement_preparation:{profile:'connect-replacement-media-v1',operation_record_id:'record'}};
 const owner=new BrowserNativeReadiness(root,original,scope,answer,callback,'offer','audio',video?'receive':undefined,video?2:0);
 const audio={connectionId:'target',streamId:'audio',generation:1,ssrc:7},vi={connectionId:'target',streamId:'receive',generation:1,ssrc:8};
 let currentReads=0;
 let current={audio,video:video?{identity:vi,epoch:'signed-epoch'}:undefined},calls=[],attempts=0,committed=false,revision=0;
 const readiness={stream_revision:3,streams},result={activation_state:'ready',connection_id:'target',replaces:'source',endpoint_id:'endpoint',device_id:'device',generation:2,retained_connection_ids:['retained'],admitted_connection_ids:['target'],retired_connection_ids:['source'],readiness,physical_readiness:readiness,native_activation:nativeBoundary(root,parameters.substrate_setup.streams,2,3)};
 const client={closed:false,async requestReplacementContinuation(type,payload,sentScope){calls.push({type,payload:structuredClone(payload),scope:{...sentScope}});
  if(type==='connection.offer'){attempts++;const error=beforeReady?.(attempts,{owner,setCommitted(){committed=true;}});if(error)throw error;committed=true;}
  const state=committed?status:'preparing';return [envelope('operation.updated',{record_id:'record',operation_id:'move',command_type:'connection.replace',revision:++revision,state,native_replacement_prepared:true,...(type==='operation.get'&&state==='completed'?{result}: {})},sentScope)];
 }};
 return {owner,original,calls,client,video,vi,audio,setCurrent:v=>{current=v;},get attempts(){return attempts;},get currentReads(){return currentReads;},
  observe({key=true,at=performance.now(),wrongAudio=false,wrongVideo=false}={}){
   for(let i=0;i<3;i++)owner.audioDecoded({identity:wrongAudio?{...audio,streamId:'foreign'}:audio,sequence:i,timestamp:i*960,arrived:at,samples:960});
   if(video)owner.videoDecoded({identity:wrongVideo?{...vi,generation:2}:vi,epoch:'signed-epoch',arrived:at,key,width:640,height:360});
  }, finish:(timeout=200,signal)=>owner.finish(client,performance.now()+(expire?-1:timeout),()=>{currentReads++;return current;},signal)};
}
// Fixed terminal metadata only; model observations never qualify a matrix cell.
function capture(t, logger) {
 const rows=[];
 t.mock.method(console,'debug',(...args)=>{if(logger)return logger(...args);rows.push(args);});
 return rows;
}
const snapshot=rows=>{assert.equal(rows.length,1);assert.equal(rows[0][0],'Original native readiness terminal');return JSON.parse(rows[0][1]);};
test('native terminal diagnostics preserve success and snapshot accepted outputs before clear',async t=>{
 const rows=capture(t),f=fixture();f.observe();const result=await f.finish(),s=snapshot(rows);
 assert.equal(result.state,'completed');assert.equal(f.attempts,1);assert.equal(f.currentReads,3);
 assert.equal(s.audio_samples,2880);assert.equal(s.video_frames,1);assert.equal(s.video_keys,1);
 assert.equal(s.audio_present,true);assert.equal(s.video_present,true);assert.equal(s.frozen,true);assert.equal(s.attempted,true);assert.equal(s.completed,true);
 assert.equal(s.current_matches,true);assert.equal(s.current_audio_identity_matches,true);assert.equal(s.current_video_identity_matches,true);assert.equal(s.current_video_epoch_matches,true);
 assert.equal(s.target_generation,2);assert.equal(s.audio_generation,1);assert.equal(s.current_video_generation,1);
 assert.equal(s.audio_block_samples,960);assert.equal(s.required_audio_samples,2880);assert.equal(s.required_video_keys,1);assert.equal(s.max_output_age_ms,200);assert.equal(s.video_width,640);assert.equal(s.video_height,360);
 assert.ok(s.audio_arrival_age_ms>=0&&s.audio_arrival_age_ms<200);assert.ok(s.current_observation_age_ms>=0);
 assert.ok(Object.values(s).every(v=>v===null||typeof v==='boolean'||(typeof v==='number'&&Number.isFinite(v))));
 assert.equal(f.owner.observing,false);assert.equal(f.owner.savedReadiness.replacement_preparation.readiness.audio.decoded_samples,2880);
});
test('native terminal diagnostics expose missing accepted key without submitting readiness',async t=>{
 const rows=capture(t),f=fixture();f.observe({key:false});await assert.rejects(f.finish(25),{code:'timeout'});const s=snapshot(rows);
 assert.equal(s.audio_samples,2880);assert.equal(s.video_frames,1);assert.equal(s.video_keys,0);assert.equal(s.current_matches,true);
 assert.equal(s.frozen,false);assert.equal(s.attempted,false);assert.equal(s.completed,false);assert.equal(f.calls.length,0);assert.ok(s.deadline_remaining_ms<1);
});
test('native terminal diagnostics distinguish missing audio and video observations',async t=>{
 const rows=capture(t),f=fixture();await assert.rejects(f.finish(25),{code:'timeout'});const s=snapshot(rows);
 assert.equal(s.audio_samples,0);assert.equal(s.video_frames,0);assert.equal(s.video_keys,0);assert.equal(s.audio_arrival_age_ms,null);assert.equal(s.video_arrival_age_ms,null);
 assert.equal(s.audio_present,false);assert.equal(s.video_present,false);assert.equal(s.current_observed,true);assert.equal(s.current_matches,false);assert.equal(f.calls.length,0);
});
test('native terminal diagnostics expose consecutive audio reset and current generation epoch mismatch',async t=>{
 const rows=capture(t),f=fixture();f.observe();f.owner.audioDecoded({identity:f.audio,sequence:8,timestamp:8*960,arrived:performance.now(),samples:960});
 f.setCurrent({audio:{...f.audio,generation:2},video:{identity:{...f.vi,generation:2},epoch:'changed-private-epoch'}});
 await assert.rejects(f.finish(25),{code:'timeout'});const s=snapshot(rows);
 assert.equal(s.audio_samples,960);assert.equal(s.current_audio_generation,2);assert.equal(s.audio_generation,1);assert.equal(s.current_video_generation,2);
 assert.equal(s.audio_identity_accepted,true);assert.equal(s.video_identity_accepted,true);assert.equal(s.current_matches,false);assert.equal(s.current_audio_identity_matches,false);assert.equal(s.current_video_identity_matches,false);assert.equal(s.current_video_epoch_matches,false);assert.equal(f.attempts,0);
});
test('native terminal diagnostics retain stale accepted arrival ages and unchanged freshness cutoff',async t=>{
 const rows=capture(t),f=fixture();f.observe();await new Promise(r=>setTimeout(r,205));await assert.rejects(f.finish(20),{code:'timeout'});const s=snapshot(rows);
 assert.equal(s.audio_samples,2880);assert.equal(s.video_keys,1);assert.ok(s.audio_arrival_age_ms>=200);assert.ok(s.video_arrival_age_ms>=200);assert.equal(s.frozen,false);assert.equal(f.attempts,0);
});
test('native terminal diagnostics do not read current for an already expired original cutoff',async t=>{
 const rows=capture(t),f=fixture({expire:true});f.observe();await assert.rejects(f.finish(),{code:'timeout'});const s=snapshot(rows);
 assert.equal(f.currentReads,0);assert.equal(f.calls.length,0);assert.equal(s.current_observed,false);assert.equal(s.current_observation_age_ms,null);assert.equal(s.current_matches,null);assert.ok(s.deadline_remaining_ms<0);
});
test('native terminal diagnostics logger failure preserves exact original error and stack',async t=>{
 let emitted=0;capture(t,()=>{emitted++;throw new Error('diagnostic sink failed');});
 const original=new ConnectError('protocol','original failure'),stack=original.stack,f=fixture({beforeReady:()=>original});f.observe();
 await assert.rejects(f.finish(),e=>e===original&&e.code==='protocol'&&e.stack===stack);assert.equal(emitted,1);assert.equal(f.attempts,1);
});
test('native terminal diagnostics logger failure cannot replace a successful original receipt',async t=>{
 let emitted=0;capture(t,()=>{emitted++;throw new Error('diagnostic sink failed');});const f=fixture();f.observe();const result=await f.finish();assert.equal(result.state,'completed');assert.equal(f.attempts,1);assert.equal(emitted,1);assert.equal(f.currentReads,3);
});
test('native terminal diagnostics remain bounded once across repeated waits on an original owner',async t=>{
 const rows=capture(t),f=fixture({expire:true});await assert.rejects(f.finish(),{code:'timeout'});f.owner.close();await assert.rejects(f.finish(),{code:'closed'});
 snapshot(rows);assert.equal(f.currentReads,0);assert.equal(f.calls.length,0);
});

test('native terminal diagnostics metadata helper failure cannot replace original success',async t=>{
 let emitted=0;capture(t,()=>{emitted++;});const f=fixture(),original=f.owner.sameCurrent.bind(f.owner);let checks=0;
 t.mock.method(f.owner,'sameCurrent',current=>{if(++checks===3)throw new Error('diagnostic helper failed');return original(current);});
 f.observe();const result=await f.finish();assert.equal(result.state,'completed');assert.equal(f.attempts,1);assert.equal(f.currentReads,3);assert.equal(checks,3);assert.equal(emitted,0);
});
