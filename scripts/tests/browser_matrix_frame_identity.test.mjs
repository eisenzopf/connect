import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readMatrixFrameOrdinal,matrixFrameIdentity} from '../lib/browser_matrix_frame_identity.mjs';
import {DecodedRoute} from '../lib/browser_matrix_observation.mjs';
import {markerFrames} from '../video/marker_calibration.mjs';
import {markerCells} from '../video/source_identity.mjs';
const pixels=()=>{const p=new Uint8ClampedArray(128*72*4).fill(100);for(let i=3;i<p.length;i+=4)p[i]=255;return p;};
function footer(p,ordinal){for(let bit=0;bit<11;bit++){const offset=(71*128+Math.floor((16+32*bit)/5))*4,value=ordinal&(1<<bit)?255:0;p[offset]=p[offset+1]=p[offset+2]=value;}return p;}
function gray(source){const g=Array(64).fill(100);for(const [i,n] of [0,1,8,9].entries())g[n]=markerCells(source)[i];return g;}
function route(){const r=new DecodedRoute(0,0,{sourceFrameIdentity:true});r.start(0);return r;}
function observe(r,owner,ordinal,pts,at=1500){r.videoFrame(owner,pts,at,gray(0),footer(pixels(),ordinal),640,360);}

test('all2048 footer patterns decode actual pixel bits while public range and pulse/source checks remain strict',()=>{
 const p=pixels(),pulse=new Set(markerFrames().filter(n=>n<1860).flatMap(n=>[n,n+1,n+2]));
 for(let n=0;n<2048;n++){
  footer(p,n);assert.equal(readMatrixFrameOrdinal(p).ordinal,n);
  for(let source=0;source<4;source++){
   const actual=matrixFrameIdentity(p,pulse.has(n)?'pulse':String(source),source);
   assert.equal(actual.status,n<1860?'identified':'unavailable');
   if(n<1860){assert.equal(actual.ordinal,n);assert.equal(actual.observed_source_id,pulse.has(n)?null:source);assert.equal(actual.pulse_exception,pulse.has(n));}
  }
 }
});
test('same original ordinal remains duplicate after newPTS and physical public handoff',()=>{
 const r=route();observe(r,'old',0,100);observe(r,'new',0,200,1533);
 const v=r.report().video;assert.equal(v.duplicate_frames,1);assert.equal(v.timestamp_reuse_diagnostics.events.length,0);
 assert.equal(v.duplicate_diagnostics.events[0].first.owner,'old');assert.equal(v.duplicate_diagnostics.events[0].current.owner,'new');
 assert.equal(v.passed,false);
});
test('distinct original ordinals with same browserPTS retain timestamp anomaly without content-duplicate credit',()=>{
 const r=route();observe(r,'same',0,100);observe(r,'same',1,100,1533);
 const v=r.report().video;assert.equal(v.duplicate_frames,0);assert.equal(v.timestamp_reuse_diagnostics.events.length,1);
 assert.equal(v.timestamp_reuse_diagnostics.events[0].first.source_frame_identity.ordinal,0);
 assert.equal(v.timestamp_reuse_diagnostics.events[0].current.source_frame_identity.ordinal,1);
 assert.equal(v.changing_frames_per_second[0],1,'only actually changed decoded pixels count');
});
test('unreadable footer or wrongsource never qualifies, and the whitepulse exception cannot be guessed fromtime',()=>{
 const p=footer(pixels(),0),offset=(71*128+3)*4;p[offset]=p[offset+1]=p[offset+2]=127;
 assert.equal(matrixFrameIdentity(p,'0',0).status,'unavailable');
 assert.equal(matrixFrameIdentity(footer(pixels(),0),'1',0).status,'wrong-source');
 assert.equal(matrixFrameIdentity(footer(pixels(),0),'pulse',0).status,'unavailable');
 assert.equal(matrixFrameIdentity(footer(pixels(),9),'0',0).status,'unavailable');
 const pulse=matrixFrameIdentity(footer(pixels(),9),'pulse',0);assert.equal(pulse.status,'identified');assert.equal(pulse.observed_source_id,null);assert.equal(pulse.pulse_exception,true);
 const r=route();r.videoFrame('old',100,1500,gray(0),p,640,360);const v=r.report().video;
 assert.equal(v.source_frame_identity.status,'failed');assert.equal(v.source_frame_identity.unavailable_frames,1);assert.equal(v.changing_frames_per_second[0],0);assert.equal(v.passed,false);
});
test('frameidentity diagnostics stay capped and their original report survives working-owner retirement',()=>{
 const r=route();for(let n=0;n<40;n++)observe(r,'same',n%2,100,1500+n);
 const report=r.report();assert.equal(report.video.timestamp_reuse_diagnostics.events.length,16);assert.equal(report.video.timestamp_reuse_diagnostics.omitted,23);
 assert.equal(report.video.duplicate_diagnostics.events.length,16);assert.ok(report.video.duplicate_diagnostics.omitted>0);
 r.retireWorkingState();assert.equal(r.workingState().video_frame_identity_rows,0);
 assert.equal(report.video.duplicate_frames,38);assert.equal(report.video.timestamp_reuse_diagnostics.events[0].current.original_timestamp_us,100);
});

test('exact native public cut and quality window exclude private or prewindow ordinals before duplicate/FPS derivation',async()=>{
 const {nativeMembership}=await import('../../sdk/typescript/dist/browser_native_activation.js');
 const {nativeIdentity}=await import('../../sdk/tests/native_activation_fixture.mjs');
 const r=route(),identity=nativeIdentity(1),cut={stream_id:'video',stream_local_id:1,stream_generation:1,
  physical_incarnation:identity.physical_incarnation,sequence_space:'native-outer-u32',public_sequence_floor:4,cursor_ordinal:4,submission_observed:false};
 const native=seq=>({profile:'connect/native-decoded-sequence/1',identity,datagramSequences:[seq],ssrc:7,rtpTimestamp:seq*3000});
 const add=(owner,ordinal,pts,at,seq)=>r.videoFrame(owner,pts,at,gray(0),footer(pixels(),ordinal),640,360,native(seq));
 add('candidate-private',1,1,1400,3);add('old-public',1,2,1500,4);
 add('prewindow-public',2,3,900,4);add('old-public',2,4,1533,5);
 add('preprogramme',2047,6,950,5);
 const included=row=>nativeMembership(cut,row.native)==='public';
 let v=r.report(included).video;assert.equal(v.duplicate_frames,0);assert.equal(v.source_frame_identity.identified_frames,2);assert.equal(v.changing_frames_per_second[0],1);
 add('another-public-owner',1,5,1566,6);v=r.report(included).video;assert.equal(v.duplicate_frames,1);
 assert.equal(v.duplicate_diagnostics.events[0].first.owner,'old-public');assert.equal(v.duplicate_diagnostics.events[0].current.owner,'another-public-owner');
 assert.equal(v.passed,false);
});

test('reserved preprogramme sentinel inside the actual qualification window fails instead of borrowing a programme epoch',()=>{
 const r=route();observe(r,'public',2047,900,1500);const v=r.report().video;
 assert.equal(v.source_frame_identity.status,'failed');assert.equal(v.source_frame_identity.unavailable_frames,1);
 assert.equal(v.source_frame_identity.identified_frames,0);assert.equal(v.passed,false);
});


test('report-derived duplicate snapshots retain original presentation scalars beyond sample cap and after retirement',()=>{
 const r=route(),add=(owner,ordinal,at,p)=>r.videoFrame(owner,0,at,gray(0),footer(pixels(),ordinal),640,360,null,p);
 const base=(element_id,presentedFrames)=>({profile:'connect/rtc-video-presentation/1',element_id,presentedFrames});
 for(let i=0;i<16;i++)add('old',0,500+i,base(1,i+1));
 const first={...base(1,17),callbackTime:100,rtpTimestamp:0,mediaTime:0,captureTime:0,receiveTime:1.25};
 const second={...base(2,1),callbackTime:133,rtpTimestamp:3000,mediaTime:0,captureTime:10,receiveTime:11};
 add('old',1,1500,first);add('new',1,1533,second);
 add('new',2,1566,{...base(2,2),mediaTime:0,rtpTimestamp:null});
 add('new',2,1599,base(2,3));
 first.rtpTimestamp=999;second.receiveTime=999;
 const report=r.report(),v=report.video,events=v.duplicate_diagnostics.events;
 assert.equal(v.duplicate_frames,2);assert.equal(v.passed,false);
 assert.deepEqual(events[0].first.presentation,{profile:'connect/rtc-video-presentation/1',element_id:1,
  callbackTime:100,mediaTime:0,presentedFrames:17,rtpTimestamp:0,captureTime:0,receiveTime:1.25});
 assert.equal(events[0].current.presentation.rtpTimestamp,3000);assert.equal(events[0].current.presentation.receiveTime,11);
 assert.equal(events[0].first.native,null);assert.equal('ssrc' in events[0].first.presentation,false);
 assert.equal(events[1].first.presentation.mediaTime,0);assert.equal(events[1].first.presentation.rtpTimestamp,null);
 for(const key of ['callbackTime','mediaTime','rtpTimestamp','captureTime','receiveTime'])assert.equal(events[1].current.presentation[key],null);
 assert.equal(r.report(row=>row.owner!=='old').video.duplicate_frames,1);
 assert.ok(Object.isFrozen(events[0].first.presentation));
 const saved=JSON.stringify(events);r.retireWorkingState();
 assert.equal(r.workingState().video_rows,0);assert.equal(JSON.stringify(events),saved);
});
