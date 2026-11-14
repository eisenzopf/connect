import {DecodedRoute} from '../lib/browser_matrix_observation.mjs';
import {markerCells} from '../video/source_identity.mjs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {rtcSourceFrameCandidates,RTC_SOURCE_FRAME_PROFILE} from '../lib/browser_matrix_source_frame_witness.mjs';
const row=(rtp,ordinal=7,owner='receiver',counter=rtp)=>({owner,row_index:counter,seconds:10+counter/1000,
 presentation:{profile:'connect/rtc-video-presentation/1',connection_id:owner,stream_id:'video',track_id:'track',element_id:1,
  presentedFrames:counter,rtpTimestamp:rtp},sourceFrameIdentity:{status:'identified',ordinal},changed:counter===1,duplicate:counter!==1});
const snapshot=r=>({owner:r.owner,row_index:r.row_index,callback_seconds:r.seconds,presentation:{...r.presentation}});
const pair=(a,b)=>({first:snapshot(a),current:snapshot(b),matching_original_row_available:true});
test('distinct RTP and same draw are pending proof only; raw rows and changing flags remain immutable',()=>{
 const a=row(100,7,'receiver',1),b=row(200,7,'receiver',2),before=structuredClone([a,b]);
 const value=rtcSourceFrameCandidates([a,b],[pair(a,b)],0,true,true);
 assert.equal(value.profile,RTC_SOURCE_FRAME_PROFILE);assert.equal(value.collection_passed,true);assert.equal(value.qualified,false);
 assert.equal(value.state,'pending-original-capture-and-mapping');assert.equal(value.legacy_draw_repeat_frames,1);
 assert.deepEqual([a,b],before);assert.equal(b.changed,false);assert.equal(b.duplicate,true);
 assert.ok(Object.isFrozen(value)&&Object.isFrozen(value.pairs[0].current.presentation));
 // Actual incoming presentation -> stored row -> public-window report seam.
 // Metadata completeness is positive; two frames alone MUST NOT pass quality.
 const route=new DecodedRoute(1,0,{sourceFrameIdentity:true,rtcSourceFrameWitness:true,initialOrdinaryReceiver:true});route.start(0);
 const gray=Array(64).fill(100);for(const [i,n]of[0,1,8,9].entries())gray[n]=markerCells(0)[i];
 const pixels=new Uint8ClampedArray(128*72*4).fill(100);for(let i=3;i<pixels.length;i+=4)pixels[i]=255;
 for(let bit=0;bit<11;bit++){const offset=(71*128+Math.floor((16+32*bit)/5))*4,value=1371&(1<<bit)?255:0;pixels[offset]=pixels[offset+1]=pixels[offset+2]=value;}
 for(let i=1;i<=2;i++)route.videoFrame('receiver',0,1500+i*33,gray,pixels,640,360,null,
  {profile:'connect/rtc-video-presentation/1',element_id:1,presentedFrames:i,rtpTimestamp:i*100,
   connection_id:'receiver',stream_id:'video',track_id:'track'});
 assert.deepEqual([route.video[0].presentation.connection_id,route.video[0].presentation.stream_id,route.video[0].presentation.track_id],['receiver','video','track']);
 const actual=route.report();assert.equal(actual.video.original_source_frame_witness.complete,true);
 assert.equal(actual.video.original_source_frame_witness.missing_presentation_rows,0);
 assert.equal(actual.video.duplicate_frames,1);assert.equal(actual.video.passed,false);assert.equal(actual.passed,false);
 route.retireWorkingState();
});
test('same actual RTP with changed footer is a hard refusal and no availability failure is hidden',()=>{
 const a=row(100,7,'receiver',1),b=row(100,8,'receiver',2);
 assert.equal(rtcSourceFrameCandidates([a,b],[],0,true,true).collection_passed,false);
 assert.equal(rtcSourceFrameCandidates([a,b],[],0,true,true).repeated_receiver_rtp_frames,1);
 const fresh=row(200,7,'receiver',2);
 assert.equal(rtcSourceFrameCandidates([a,fresh],[pair(a,fresh)],0,false,true).collection_passed,false);
});
test('prepared/cross-owner/missing/omitted metadata never activates the pending source-frame path',()=>{
 const a=row(100,7,'receiver',1),b=row(200,7,'receiver',2);
 for(const [rows,pairs,omitted,initial]of[
  [[a,b],[pair(a,b)],0,false],[[a,b],[pair(a,b)],1,true],
  [[a,{...b,presentation:null}],[pair(a,b)],0,true],
  [[a,row(200,7,'other',2)],[pair(a,row(200,7,'other',2))],0,true],
  [[a,{...b,presentation:{...b.presentation,element_id:2}}],[pair(a,{...b,presentation:{...b.presentation,element_id:2}})],0,true],
 ])assert.equal(rtcSourceFrameCandidates(rows,pairs,omitted,true,initial).collection_passed,false);
});
