import {test} from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import {pcmBoundary,MatrixMediaStages,browserMatrixRtcStages} from '../lib/browser_matrix_stages.mjs';
import {pcmIdentity} from '../lib/browser_matrix_pcm.mjs';
import {DecodedRoute,browserMatrixObservation} from '../lib/browser_matrix_observation.mjs';
import {installPeerDiagnostics} from '../video/browser_failure.mjs';
test('actual half-silence remains visible even when whole-frame tone identity accepts it',()=>{
 const pcm=Float32Array.from({length:960},(_,i)=>i<480?.05*Math.sin(2*Math.PI*(660*i/48000+.2)):0),original=pcm.slice();
 assert.equal(pcmIdentity(pcm,1).identified,true);const boundary=pcmBoundary(pcm);
 assert.equal(boundary.trailing_exact_zero_samples,480);assert.equal(boundary.longest_exact_zero_run_samples,480);
 assert.deepEqual(boundary.rms_1ms.slice(10),Array(10).fill(0));assert.deepEqual(boundary.rms_2ms.slice(5),Array(5).fill(0));
 assert.ok(boundary.rms_1ms.slice(0,10).every(value=>value>0));assert.deepEqual(pcm,original);
 assert.equal(Object.values(boundary).filter(Array.isArray).flat().length,30,'No raw PCM is retained');
});
test('capture discontinuity counters and bounded numeric evidence retain original times and do not retain a peer',()=>{
 const stages=new MatrixMediaStages(),samples=new Float32Array(960);samples[0]=.01;
 const peer={get diagnostics(){return {droppedPackets:3};},get routeDiagnostics(){return [{connectionId:'source',kind:'audio',audioCodec:{encodedFrames:8}}];}};
 for(let i=0;i<40;i++)stages.audio('source',0,i*40000,i*.04,samples,{identified:false,rms:.001},true,peer);
 const row=stages.report().owners[0];assert.equal(row.audio_input,40);assert.equal(row.audio_timestamp_gaps,39);
 assert.equal(row.last_audio_timestamp_us,1560000);assert.equal(row.suspect_audio.length,32);assert.equal(row.omitted_suspect_audio,8);
 assert.equal(row.suspect_audio[0].original_timestamp_us,0);assert.equal(row.suspect_audio[0].callback_seconds,0);
 assert.equal(row.peer,undefined);assert.notEqual(row.snapshots[0].peer,peer);
});
test('boundary diagnostics do not qualify silence, remove duplicate observations or repair callback timestamps',()=>{
 const route=new DecodedRoute(0,0);route.start(1000);const silence=new Float32Array(960);
 route.audioFrame('original',20000,2020,silence);route.audioFrame('original',20000,2040,silence);
 const report=route.report();assert.equal(report.audio.decoded_seconds,.04);assert.equal(report.audio.duplicate_frames,1);
 assert.equal(report.audio.bad_identity_frames,2);assert.equal(report.audio.passed,false);
 assert.equal(report.audio_boundary_diagnostics.events[0].callback_seconds,1.02);
 assert.equal(report.audio_boundary_diagnostics.events[0].original_timestamp_us,20000);
});
test('RTC numeric sampler owns one pending read and clears its timer on original close',async()=>{
 const originalSet=globalThis.setInterval,originalClear=globalThis.clearInterval;let callback,cleared=0,resolve,calls=0;
 globalThis.setInterval=fn=>{callback=fn;return 1;};globalThis.clearInterval=()=>{cleared++;};
 const sampler=browserMatrixRtcStages(()=>{calls++;return new Promise(done=>{resolve=done;});});
 try{sampler.start(performance.now());await Promise.resolve();callback();callback();assert.equal(calls,1);
  resolve([[{type:'outbound-rtp',framesEncoded:4}]]);for(let i=0;i<10;i++)await Promise.resolve();
  assert.equal(sampler.report().samples.length,1);assert.equal((await sampler.close()).joined,true);assert.equal(cleared,1);
  callback();await Promise.resolve();assert.equal(calls,1);
 }finally{await sampler.close();globalThis.setInterval=originalSet;globalThis.clearInterval=originalClear;}
});
test('all sixteen exact native peers can be labeled and closed peers release references before later stats',async()=>{
 class Peer{constructor(){this.connectionState='new';this.iceConnectionState='new';this.iceGatheringState='new';this.signalingState='stable';this.reads=0;this.listeners=[];}
  getTransceivers(){return [];}addEventListener(type,fn){this.listeners.push([type,fn]);}removeEventListener(type,fn){this.listeners=this.listeners.filter(row=>row[0]!==type||row[1]!==fn);}
  close(){this.connectionState=this.iceConnectionState=this.signalingState='closed';}
  async getStats(){this.reads++;return new Map([['v',{type:'outbound-rtp',kind:'video',timestamp:123,framesEncoded:5,framesSent:4,qualityLimitationReason:'cpu'}]]);}}
 const scope={RTCPeerConnection:Peer,performance:{now:()=>17}};vm.createContext(scope);
 vm.runInContext('('+installPeerDiagnostics.toString()+')({peerBound:16})',scope);
 const peers=Array.from({length:16},()=>new scope.RTCPeerConnection());
 for(const [index,peer]of peers.entries())assert.equal(scope.connectPeerTag(peer,'connection-'+index),true);
 const first=await scope.connectPeerMediaDiagnostics();assert.equal(first.length,16);assert.equal(first[15][0].connection_id,'connection-15');
 assert.equal(first[0][0].framesSent,4);assert.equal(first[0][0].qualityLimitationReason,'cpu');
 peers[0].close();assert.equal(peers[0].listeners.length,0);await scope.connectPeerMediaDiagnostics();assert.equal(peers[0].reads,1);
 const report=scope.connectPeerDiagnostics();assert.equal(report.peer_bound,16);assert.equal(report.omitted_peers,0);
 assert.equal(report.peers[0].live_owner_retained,false);assert.equal(report.peers[0].current.connection,'closed');
 assert.equal(scope.connectPeerCodecDiagnostics()[0].closed,true);
 for(const peer of peers)peer.close();
 assert.ok(scope.connectPeerDiagnostics().peers.every(value=>value.live_owner_retained===false));
});

test('one failed peer cannot erase other original counters and opaque report identities never escape',async()=>{
 class Peer{
  constructor(){this.connectionState=this.iceConnectionState='new';this.iceGatheringState='new';this.signalingState='stable';this.reverse=false;}
  addEventListener(){}removeEventListener(){}getTransceivers(){return [];}close(){this.connectionState=this.signalingState='closed';}
  async getStats(){if(this.fail)throw Error('private-stats-secret');const rows=[
   ['private-video-source',{type:'media-source',kind:'video',timestamp:123,frames:32,framesPerSecond:30,trackIdentifier:'private-track'}],
   ['private-video-out',{type:'outbound-rtp',kind:'video',timestamp:123,framesEncoded:30,framesSent:29,encoderImplementation:'private-implementation'}],
   ['private-video-in',{type:'inbound-rtp',kind:'video',timestamp:123,framesReceived:28,framesDecoded:27,framesDropped:1,packetsLost:-1}],
  ];return new Map(this.reverse?rows.reverse():rows);}
 }
 const scope={RTCPeerConnection:Peer,performance:{now:()=>17}};vm.createContext(scope);
 vm.runInContext('('+installPeerDiagnostics.toString()+')({peerBound:16})',scope);
 const failed=new scope.RTCPeerConnection(),healthy=new scope.RTCPeerConnection();failed.fail=true;
 scope.connectPeerTag(failed,'original-failed');scope.connectPeerTag(healthy,'original-healthy');
 const first=await scope.connectPeerMediaDiagnostics();assert.equal(first[0][0].unavailable,true);
 assert.equal(first[1][0].frames,32);assert.equal(first[1][1].framesEncoded,30);assert.equal(first[1][2].framesDecoded,27);
 assert.equal(first[1][2].packetsLost,-1);healthy.reverse=true;
 const second=await scope.connectPeerMediaDiagnostics();
 assert.deepEqual(Array.from(second[1],row=>row.report_index),[2,1,0]);
 assert.equal(JSON.stringify([first,second]).includes('private-'),false);
 failed.close();healthy.close();
 assert.ok((await scope.connectPeerMediaDiagnostics()).every(rows=>rows.length===0));
});

test('an original pending RTC statistics read cannot be reported as joined by observer retirement',async()=>{
 const names=['AudioContext','CanvasRenderingContext2D','HTMLVideoElement','connectPeerMediaDiagnostics'];
 const saved=names.map(name=>[name,Object.getOwnPropertyDescriptor(globalThis,name)]);let resolve,reads=0;
 class AudioContext{createMediaStreamSource(){throw Error('No renderer capture is started in this fixture');}}
 class CanvasRenderingContext2D{drawImage(){}}
 class HTMLVideoElement{requestVideoFrameCallback(){}}
 class Rtc{render(){}}
 class Renderer{audio(){}video(){}}
 Object.assign(globalThis,{AudioContext,CanvasRenderingContext2D,HTMLVideoElement,
  connectPeerMediaDiagnostics:()=>{reads++;return new Promise(done=>{resolve=done;});}});
 const observer=browserMatrixObservation(Renderer,{routes:[]},null,Rtc);
 try{
  observer.start({originMs:performance.now()});await Promise.resolve();assert.equal(reads,1);
  const first=observer.close(),second=observer.close();assert.equal(first,second);
  const closed=await first;assert.equal(closed.joined,false);assert.equal(closed.statistics.joined,false);
  assert.equal(closed.statistics.outstanding_original_snapshot,true);
  resolve([]);for(let i=0;i<10;i++)await Promise.resolve();
  assert.equal((await observer.close()).joined,false,'A late completion cannot rewrite the original timeout receipt');
  assert.equal(reads,1);
 }finally{
  resolve?.([]);await observer.close();
  for(const[name,value]of saved)if(value)Object.defineProperty(globalThis,name,value);else delete globalThis[name];
 }
});


test('RTC statistics attempts remain bounded when every original read rejects',async()=>{
 const originalSet=globalThis.setInterval,originalClear=globalThis.clearInterval;let callback,cleared=0,calls=0;
 globalThis.setInterval=fn=>{callback=fn;return 1;};globalThis.clearInterval=()=>{cleared++;};
 const sampler=browserMatrixRtcStages(async()=>{calls++;throw Error('Unavailable original native statistics');});
 try{
  sampler.start(performance.now());
  for(let i=0;i<80;i++){for(let tick=0;tick<10;tick++)await Promise.resolve();callback();}
  for(let tick=0;tick<10;tick++)await Promise.resolve();
  const report=sampler.report();assert.equal(calls,64);assert.equal(report.attempts,64);
  assert.equal(report.samples.length,0);assert.equal(report.failures.length,16);assert.equal(cleared,1);
  assert.equal((await sampler.close()).joined,true);callback();await Promise.resolve();assert.equal(calls,64);
 }finally{await sampler.close();globalThis.setInterval=originalSet;globalThis.clearInterval=originalClear;}
});
