// Native API/Root ownership models only; no browser, server F or wire pass.
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {WebRtcReceiveReservationIntentOwner} from '../typescript/dist/browser_webrtc_receive_reservation.js';
import {WebRtcReceiveRoles} from '../typescript/dist/browser_webrtc_receive_roles.js';
import {WebRtcPublicVideoPhaseOwner} from '../typescript/dist/browser_webrtc_public_video_phase.js';
import {observeWebRtcPublicVideoReadiness} from '../typescript/dist/browser_webrtc_public_video_readiness.js';
import {BrowserDeadline} from '../typescript/dist/browser_deadline.js';
const root=()=>({recordId:'record',operationId:'move',conversationId:'room',sessionId:'session',sourceConnectionId:'old',retainedConnectionId:'retained',targetConnectionId:'target',endpointId:'endpoint',deviceId:'device'});
const scope=()=>({recordId:'record',operationId:'move',targetConnectionId:'target',audioStreamId:'audio',previewStreamId:'camera-a',publicStreamId:'public-b'});
const intent=()=>({profile:'connect/webrtc-public-video-reservation/1',phase:'intent',reservation_id:'reservation',nonce:'intent-nonce',preview_stream_id:'camera-a',preview_role:'receive',public_stream_id:'public-b'});
const binding=()=>({stream_id:'public-b',preview_stream_id:'camera-a',mid:'2',stream_generation:1,physical_writer_incarnation:'writer-original',connection_generation:1,primary_ssrc:0,payload_type:96,clock_rate_hz:90000,codec:'vp8'});
const wire=(phase='cut-bound-post-f')=>({profile:'connect/webrtc-public-video-phase/1',phase,reservation_id:'reservation',operation_record_id:'record',operation_id:'move',target_connection_id:'target',target_generation:2,binding:binding(),...(phase==='discarded'?{source_cut_id:'opaque-source-cut',reason:'cancelled'}:phase==='negotiated-never-fed'?{}:{source_cut_id:'opaque-source-cut',public_readiness:{nonce:'public-nonce',decoder_startup:'fresh-receiver-first-decoded-frame'}})});
const original=(phase)=>({recordId:'record',operationId:'move',targetConnectionId:'target',targetGeneration:2,rootCompleted:phase==='activated'});
function model(){
 const owner=new WebRtcReceiveReservationIntentOwner(scope()),selected=owner.observe(intent(),scope()),reservation={owner,intent:selected};
 const transceivers=['audio','video','video'].map((kind,index)=>({direction:index===2?'recvonly':'sendrecv',currentDirection:null,mid:null,receiver:{track:{kind,id:`track-${index}`,readyState:'live',stop(){assert.fail('Observer stopped original native track');}}}}));
 const peer={connectionState:'new',signalingState:'stable',localDescription:null,remoteDescription:null,getTransceivers:()=>transceivers,getStats(){assert.fail('B must query its exact receiver, not peer-wide/audio statistics');},close(){assert.fail('B lease does not own native peer close');}};
 owner.claimPhysical(peer,selected,scope());const roles=new WebRtcReceiveRoles(peer,scope(),selected,{audio:transceivers[0],preview:transceivers[1],public:transceivers[2]});
 for(const[index,tx]of transceivers.entries()){tx.mid=String(index);tx.currentDirection=tx.direction;roles.accept({transceiver:tx,receiver:tx.receiver,track:tx.receiver.track});}peer.connectionState='connected';
 const phaseOwner=new WebRtcPublicVideoPhaseOwner(root(),scope(),reservation,2,'private-nonce');
 return{owner,roles,peer,transceivers,reservation,phaseOwner,observe(value){return phaseOwner.observe(value,original(value.phase));},close(){phaseOwner.close();roles.close();owner.close();}};
}
test('public video phase binds immutable original namespaces and keeps zero SSRC valid without a media grant',async()=>{
 const f=model();try{
  const n=f.observe(wire('negotiated-never-fed'));assert.equal(n.binding.primary_ssrc,0);assert.equal(n.binding.connection_generation,1);assert.equal(n.target_generation,2);
  assert.throws(()=>new WebRtcPublicVideoPhaseOwner(root(),scope(),f.reservation,2,'private-nonce'),{code:'conflict'});
  await assert.rejects(observeWebRtcPublicVideoReadiness(f.phaseOwner,f.peer,f.roles,new BrowserDeadline(500),new AbortController().signal,work=>work),{code:'protocol'},'No cut-bound proof means no B decoder allocation');
  const raw=wire(),cut=f.observe(raw);raw.binding.mid='changed';raw.public_readiness.nonce='changed';assert.equal(cut.binding.mid,'2');assert.equal(cut.public_readiness.nonce,'public-nonce');
  assert.ok(Object.isFrozen(cut)&&Object.isFrozen(cut.binding)&&Object.isFrozen(cut.public_readiness));assert.equal(f.phaseOwner.exactLocalPeer(f.peer),true);
  const ready=f.observe(wire('decoded-ready'));assert.equal(f.phaseOwner.accepts(cut),true);assert.equal(f.phaseOwner.activated(ready),false);
  assert.throws(()=>f.phaseOwner.observe(wire('activated'),{...original('activated'),rootCompleted:false}),{code:'protocol'});
  const active=f.observe(wire('activated'));assert.equal(f.phaseOwner.activated(active),true);assert.equal(f.phaseOwner.activated({...active}),false);
  assert.throws(()=>f.observe(wire('cut-bound-post-f')),{code:'protocol'});
 }finally{f.close();}
});
test('public video phase refuses foreign Root binding nonce premature cut and terminal replay without evaluating accessors',()=>{
 for(const mutate of [v=>v.operation_record_id='foreign',v=>v.operation_id='foreign',v=>v.reservation_id='foreign',v=>v.target_connection_id='foreign',v=>v.target_generation=3,v=>v.binding.stream_id='camera-a',v=>v.binding.preview_stream_id='foreign',v=>v.binding.primary_ssrc=0x100000000,v=>v.binding.payload_type=128,v=>v.binding.clock_rate_hz=48000,v=>v.binding.codec='h264',v=>v.binding.mid='bad mid',v=>v.binding.mid='x'.repeat(257),v=>v.binding.physical_writer_incarnation='é',v=>v.binding.physical_writer_incarnation='bad value',v=>v.source_cut_id='é',v=>v.source_cut_id='x'.repeat(257),v=>v.binding.stream_generation=Number.MAX_SAFE_INTEGER+1,v=>v.binding.connection_generation=0,v=>v.binding.extra=true,v=>v.public_readiness.nonce='private-nonce',v=>v.public_readiness.nonce='intent-nonce',v=>v.public_readiness.decoder_startup='fresh-peer-first-decoded-frame',v=>v.extra=true]){
  const f=model();try{const value=wire();mutate(value);assert.throws(()=>f.observe(value),{code:'protocol'});}finally{f.close();}
 }
 const f=model();try{
  const n=wire('negotiated-never-fed');n.source_cut_id='premature';assert.throws(()=>f.observe(n),{code:'protocol'});
  const v=wire();Object.defineProperty(v,'public_readiness',{enumerable:true,get(){assert.fail('Accessor must not run');}});assert.throws(()=>f.observe(v),{code:'protocol'});
  f.observe(wire());for(const mutate of [v=>v.binding.mid='3',v=>v.source_cut_id='other-cut',v=>v.public_readiness.nonce='other-nonce']){const value=wire();mutate(value);assert.throws(()=>f.observe(value),{code:'protocol'});}
  assert.throws(()=>f.observe({...wire('discarded'),source_cut_id:null}),{code:'protocol'},'known cut cannot disappear');
  assert.throws(()=>f.observe({...wire('discarded'),reason:'raw-unbounded-reason'}),{code:'protocol'});
  f.observe(wire('discarded'));assert.throws(()=>f.observe({...wire('discarded'),reason:'expired'}),{code:'protocol'});
  assert.throws(()=>f.observe(wire('activated')),{code:'protocol'});assert.equal(f.phaseOwner.accepts(wire()),false);
 }finally{f.close();}
 const beforeCut=model();try{const discarded=beforeCut.observe({...wire('discarded'),source_cut_id:null});assert.equal(discarded.source_cut_id,null);assert.throws(()=>beforeCut.observe(wire('discarded')),{code:'protocol'});}finally{beforeCut.close();}
});
async function fixture(run){
 const f=model(),saved=new Map(),elements=[],tasks=new Set(),pendingResolvers=[];let audioTouches=0,playHook;
 class Stream{constructor(tracks){this.tracks=tracks;}}
 const values={MediaStream:Stream,document:{createElement(kind){assert.equal(kind,'video');const element={muted:false,srcObject:null,pauses:0,cancels:0,next:0,callback:null,play(){playHook?.();return Promise.resolve();},pause(){this.pauses++;},requestVideoFrameCallback(fn){this.callback=fn;return++this.next;},cancelVideoFrameCallback(){this.callback=null;this.cancels++;}};elements.push(element);return element;}}};
 for(const[name,value]of Object.entries(values)){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,value});}
 for(const name of ['AudioContext','AudioWorkletNode']){saved.set(name,Object.getOwnPropertyDescriptor(globalThis,name));Object.defineProperty(globalThis,name,{configurable:true,get(){audioTouches++;assert.fail('Video-only B observation cannot touch audio');}});}
 const stats={rtp:{id:'inbound',type:'inbound-rtp',kind:'video',ssrc:0,trackIdentifier:'track-2',mid:'2',codecId:'codec',framesDecoded:1,keyFramesDecoded:1},codec:{id:'codec',type:'codec',mimeType:'video/VP8',clockRate:90000,payloadType:96}};
 let calls=0,behavior;const receiver=f.transceivers[2].receiver;
 const report=()=>new Map([['inbound',{...stats.rtp}],['codec',{...stats.codec}]]);
 receiver.getStats=()=>{calls++;return behavior?behavior():Promise.resolve(report());};
 const track=(work)=>{tasks.add(work);void work.then(()=>tasks.delete(work),()=>tasks.delete(work));return work;};
 const abort=new AbortController(),budget=new BrowserDeadline(500);f.observe(wire('negotiated-never-fed'));const cut=f.observe(wire());
 const begin=()=>observeWebRtcPublicVideoReadiness(f.phaseOwner,f.peer,f.roles,budget,abort.signal,track);
 const tick=()=>new Promise(resolve=>setImmediate(resolve));
 try{await run({...f,stats,report,elements,abort,budget,track,begin,cut,tasks,calls:()=>calls,
  behavior(value){behavior=value;},pending(){return new Promise(resolve=>pendingResolvers.push(resolve));},
  onPlay(fn){playHook=fn;},
  async started(){const until=Date.now()+300;while(!elements[0]?.callback){assert.ok(Date.now()<until);await tick();}},
  frame(n=1,extra={}){const element=elements[0],callback=element.callback;element.callback=null;assert.equal(typeof callback,'function');callback(0,{width:640,height:360,mediaTime:0,presentedFrames:n,...extra});},
 });}finally{
  f.close();abort.abort();for(const resolve of pendingResolvers)resolve(report());
  while(tasks.size)await Promise.allSettled([...tasks]);
  assert.equal(audioTouches,0);assert.ok(elements.every(element=>element.srcObject===null&&element.pauses===1));
  for(const[name,value]of saved)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];
 }
}
test('B readiness needs actual exact receiver output and retains one immutable lease through Activated without audio rerun',()=>fixture(async f=>{
 let reentered;f.onPlay(()=>{reentered=f.begin();});const pending=f.begin();assert.strictEqual(reentered,pending);assert.strictEqual(f.begin(),pending);await f.started();await new Promise(resolve=>setImmediate(resolve));let ready=false;void pending.then(()=>ready=true);
 assert.equal(ready,false,'RTP/codec counters alone are not decoded output');assert.equal(f.elements.length,1);assert.equal(f.elements[0].srcObject.tracks[0],f.transceivers[2].receiver.track);
 f.frame();const lease=await pending,marker=lease.readiness;assert.equal(marker.profile,'connect-replacement-public-video-ready-v1');assert.equal(marker.operation_record_id,'record');
 assert.deepEqual(marker.public_video_readiness,{reservation_id:'reservation',source_cut_id:'opaque-source-cut',nonce:'public-nonce',binding:binding(),decoded_frames:1,decoded_keyframes:1,decoder_startup:'fresh-receiver-first-decoded-frame'});
 assert.ok(Object.isFrozen(marker)&&Object.isFrozen(marker.public_video_readiness));assert.strictEqual(f.begin(),pending);
 assert.equal(lease.take(f.cut,f.transceivers[2].receiver.track),undefined);
 f.frame(2);assert.equal(marker.public_video_readiness.decoded_frames,1,'Retained callback high-water cannot rewrite original ready evidence');
 f.observe(wire('decoded-ready'));const active=f.observe(wire('activated'));
 assert.equal(lease.take(active,f.transceivers[1].receiver.track),undefined,'A can never substitute for B');
 const late=f.elements[0].callback,output=lease.take(active,f.transceivers[2].receiver.track);assert.equal(output.element,f.elements[0]);assert.equal(output.presentedFloor,2);
 assert.equal(lease.take(active,f.transceivers[2].receiver.track),undefined);late(0,{width:640,height:360,mediaTime:0,presentedFrames:3});assert.equal(f.elements[0].callback,null);
 await lease.release();assert.notEqual(output.element.srcObject,null,'Adopted output belongs only to renderer');output.element.pause();output.element.srcObject=null;
}));
test('B readiness optional keyframe counter follows current convention and replay cannot create a fresh budget or decoder',async()=>{
 await fixture(async f=>{
 delete f.stats.rtp.keyFramesDecoded;delete f.stats.rtp.framesDecoded;
 const pending=f.begin();await f.started();f.frame();const lease=await pending;
 assert.equal(lease.readiness.public_video_readiness.decoded_frames,1);assert.equal(lease.readiness.public_video_readiness.decoded_keyframes,null);
 await assert.rejects(observeWebRtcPublicVideoReadiness(f.phaseOwner,f.peer,f.roles,new BrowserDeadline(500),f.abort.signal,f.track),{code:'protocol'});
 await lease.release();assert.strictEqual(f.begin(),pending);assert.equal(f.elements.length,1);assert.equal(lease.take(f.observe(wire('activated')),f.transceivers[2].receiver.track),undefined);
 });
 await fixture(async f=>{
  f.stats.rtp.keyFramesDecoded=0;const pending=f.begin();let ready=false;void pending.then(()=>ready=true);await f.started();f.frame();
  while(!f.calls())await new Promise(resolve=>setImmediate(resolve));await new Promise(resolve=>setImmediate(resolve));assert.equal(ready,false,'A present zero keyframe count cannot be treated as absent');
  f.stats.rtp.keyFramesDecoded=1;const lease=await pending;assert.equal(lease.readiness.public_video_readiness.decoded_keyframes,1);await lease.release();
 });
 const originalPerformance=Object.getOwnPropertyDescriptor(globalThis,'performance');let now=0;
 Object.defineProperty(globalThis,'performance',{configurable:true,value:{now:()=>now}});
 try{
  await fixture(async f=>{
   const pending=f.begin();await f.started();f.frame();const lease=await pending;
   now=501;const active=f.observe(wire('activated'));assert.equal(f.abort.signal.aborted,false);
   assert.throws(()=>lease.take(active,f.transceivers[2].receiver.track),{code:'timeout'},'Ready must not extend the original activation deadline');
   assert.equal(f.elements[0].srcObject,null);assert.equal(f.elements[0].callback,null);assert.equal(f.elements[0].pauses,1);
   assert.strictEqual(f.begin(),pending,'Expiry keeps the original attempt tombstone');assert.equal(f.elements.length,1);
  });
  now=1000;
  await fixture(async f=>{
   const pending=f.begin();await f.started();f.frame();const lease=await pending;
   now=1501;f.frame(2);assert.equal(f.elements[0].srcObject,null,'A retained callback releases the expired unadopted picture');
   assert.equal(f.elements[0].callback,null);assert.equal(lease.take(f.observe(wire('activated')),f.transceivers[2].receiver.track),undefined);
   assert.strictEqual(f.begin(),pending);assert.equal(f.elements.length,1);
  });
 }finally{if(originalPerformance)Object.defineProperty(globalThis,'performance',originalPerformance);else delete globalThis.performance;}
});
test('B readiness rejects competing native rows and foreign SSRC codec MID track or immutable owner binding',async()=>{
 for(const mutate of [f=>f.stats.rtp.ssrc=7,f=>f.stats.rtp.mid='1',f=>f.stats.rtp.trackIdentifier='track-1',f=>f.stats.codec.payloadType=97,f=>f.stats.codec.mimeType='video/H264',f=>f.stats.rtp.framesDecoded=-1,f=>f.stats.rtp.keyFramesDecoded=NaN,f=>f.behavior(async()=>{const report=f.report();report.set('duplicate',{...f.stats.rtp,id:'duplicate'});return report;})])await fixture(async f=>{
  mutate(f);const pending=f.begin();f.frame();await assert.rejects(pending,{code:'protocol'});assert.strictEqual(f.begin(),pending);assert.equal(f.elements.length,1);
 });
});
test('B readiness cancellation and original report rejection retain exact failure and release only their local video lease',async()=>{
 await fixture(async f=>{
  const failure=Error('original receiver report rejection');f.behavior(()=>Promise.reject(failure));const pending=f.begin();await f.started();await assert.rejects(pending,error=>error===failure);assert.equal(f.elements[0].srcObject,null);
 });
 await fixture(async f=>{
  f.behavior(()=>f.pending());const pending=f.begin();await f.started();while(!f.calls())await new Promise(resolve=>setImmediate(resolve));const callback=f.elements[0].callback;
  f.abort.abort();await assert.rejects(pending,{code:'cancelled'});assert.ok(f.tasks.size>0,'Original native report remains owned until actual settlement');assert.equal(f.elements[0].srcObject,null);
  callback(0,{width:640,height:360,mediaTime:0,presentedFrames:1});assert.equal(f.elements[0].callback,null);
 });
 await fixture(async f=>{
  const pending=f.begin();await f.started();f.frame();const lease=await pending;f.observe(wire('discarded'));await lease.release();assert.equal(f.elements[0].srcObject,null);
  await assert.rejects(f.begin(),{code:'protocol'});
 });
 await fixture(async f=>{
  const pending=f.begin();await f.started();f.frame();const lease=await pending,active=f.observe(wire('activated')),failure=Error('original callback cancellation refused');
  f.elements[0].cancelVideoFrameCallback=()=>{throw failure;};
  assert.throws(()=>lease.take(active,f.transceivers[2].receiver.track),error=>error===failure);
  assert.notEqual(f.elements[0].srcObject,null,'Failed transfer must leave original lease owning cleanup');
  await assert.rejects(lease.release(),error=>error===failure);assert.equal(f.elements[0].srcObject,null);
 });
});
