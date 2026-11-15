import {DecodedRoute} from '../lib/browser_matrix_observation.mjs';
import {markerCells} from '../video/source_identity.mjs';
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {rtcSourceFrameCandidates,rtcSourceFrameControlRoute,RTC_SOURCE_FRAME_PROFILE} from '../lib/browser_matrix_source_frame_witness.mjs';
const row=(rtp,ordinal=7,owner='receiver',counter=rtp)=>({owner,row_index:counter-1,seconds:2.2+counter*.017,
 presentation:{profile:'connect/rtc-video-presentation/1',connection_id:owner,stream_id:'video',track_id:'track',element_id:1,
  presentedFrames:counter,rtpTimestamp:rtp},sourceFrameIdentity:{status:'identified',ordinal},changed:counter===1,duplicate:counter!==1});
const snapshot=r=>({owner:r.owner,row_index:r.row_index,callback_seconds:r.seconds,presentation:{...r.presentation}});
const pair=(a,b)=>({first:snapshot(a),current:snapshot(b)});
const ledger=()=>[{owner:'receiver',receive_stream_id:'video',send_stream_id:'sent-video',start_ms:0,end_ms:null,previous:null,operation_id:null,basis:'original-initial-public-render'}];
const candidate=(rows,pairs=[],omitted=0,passed=true,control=null,publicOwners=ledger())=>rtcSourceFrameCandidates(rows,pairs,omitted,passed,true,control,publicOwners);
const unpack=value=>JSON.parse(value.census_json);
test('distinct RTP and same draw are pending proof only; raw rows and changing flags remain immutable',()=>{
 const a=row(100,7,'receiver',1),b=row(200,7,'receiver',2),before=structuredClone([a,b]);
 const value=candidate([a,b],[pair(a,b)]);assert.equal(value.profile,RTC_SOURCE_FRAME_PROFILE);
 assert.equal(value.collection_passed,true);assert.equal(value.qualified,false);assert.deepEqual(unpack(value).pairs,[[0,1]]);
 assert.deepEqual([a,b],before);assert.ok(Object.isFrozen(value));
 // Existing presentation -> stored rows -> SAME public-filtered report seam.
 const route=new DecodedRoute(2,0,{sourceFrameIdentity:true,rtcSourceFrameWitness:true});route.start(0);route.publicOwnerIntervals=ledger();
 const gray=Array(64).fill(100);for(const [i,n]of[0,1,8,9].entries())gray[n]=markerCells(0)[i];
 const pixels=new Uint8ClampedArray(128*72*4).fill(100);for(let i=3;i<pixels.length;i+=4)pixels[i]=255;
 for(let bit=0;bit<11;bit++){const offset=(71*128+Math.floor((16+32*bit)/5))*4,v=1371&(1<<bit)?255:0;pixels[offset]=pixels[offset+1]=pixels[offset+2]=v;}
 const plan={routes:Array.from({length:6},(_,index)=>({index,session:index<4?'A':'B'})),moves:[{source:0},{source:4},{source:0},{source:4}]};
 route.associationControl=rtcSourceFrameControlRoute(plan);assert.equal(route.associationControl.receiver_route_index,2);assert.equal(route.associationControl.source_route_index,1);
 for(let i=1;i<=2;i++)route.videoFrame('receiver',0,2200+i*33,gray,pixels,640,360,null,
  {profile:'connect/rtc-video-presentation/1',element_id:1,presentedFrames:i,rtpTimestamp:i*100,connection_id:'receiver',stream_id:'video',track_id:'track'});
 const raw=structuredClone(route.video),report=route.report(),actual=report.video.original_source_frame_witness;
 assert.equal(actual.complete,true);assert.deepEqual(unpack(actual).control.rows,[0,1]);assert.equal(report.passed,false);
 assert.equal(report.video.duplicate_frames,1);assert.deepEqual(route.video,raw);
 assert.equal(route.report(r=>r.row_index!==0).video.original_source_frame_witness.collection_passed,false);
 route.retireWorkingState();
 // Complete census is independent of diagnostic16; all39 aliases retained.
 const many=Array.from({length:40},(_,i)=>row(100+i*100,7,'receiver',i+1));
 const all=candidate(many,Array.from({length:16},(_,i)=>pair(many[0],many[i+1])),23);
 assert.equal(all.collection_passed,true);assert.equal(all.pair_count,39);assert.equal(unpack(all).pairs.length,39);
 assert.equal(all.legacy_diagnostic_omitted,23);assert.ok(all.encoded_bytes<8*1024*1024);
});
test('same actual RTP with changed footer is a hard refusal and no availability failure is hidden',()=>{
 const a=row(100,7,'receiver',1),b=row(100,8,'receiver',2);assert.equal(candidate([a,b]).collection_passed,false);
 assert.equal(candidate([a,b]).repeated_receiver_rtp_frames,1);
 const fresh=row(200,7,'receiver',2);assert.equal(candidate([a,fresh],[pair(a,fresh)],0,false).collection_passed,false);
 const original=Array.from({length:10001},(_,i)=>({...row(i,7,'receiver',i+1),seconds:2.2+i*.00001}));assert.equal(candidate(original,[],10000).complete,false);
});
test('prepared/cross-owner/missing/omitted metadata never activates the pending source-frame path',()=>{
 const a=row(100,7,'receiver',1),b=row(200,7,'receiver',2);
 for(const rows of [[a,{...b,presentation:null}],[a,row(200,7,'private',2)],
  [a,{...b,presentation:{...b.presentation,element_id:2}}],[a,{...b,presentation:{...b.presentation,track_id:'other'}}]])
  assert.equal(candidate(rows,[pair(rows[0],rows[1])]).collection_passed,false);
 assert.equal(candidate([a,b],[pair(a,b)],0,true,null,[]).complete,false);
 const moved=[{...ledger()[0],end_ms:2250},{owner:'new',receive_stream_id:'new-video',send_stream_id:'new-send',start_ms:2250,end_ms:null,previous:'receiver',operation_id:'op',basis:'validated-receipts-local-public-render-cut'}];
 const c=row(300,8,'new',1),d=row(400,8,'new',2);c.seconds=3;d.seconds=3.017;
 c.presentation.stream_id=d.presentation.stream_id='new-video';assert.equal(candidate([c,d],[pair(c,d)],0,true,null,moved).complete,true);
 const beforeCut={...c,seconds:2.249};assert.equal(candidate([beforeCut,d],[pair(beforeCut,d)],0,true,null,moved).complete,false);
 const control={receiver_route_index:2,source_route_index:1,window_start_seconds:2,window_end_seconds:3};
 assert.equal(candidate([a,b],[pair(a,b)],0,true,control).complete,true);
 assert.equal(candidate([a],[ ],0,true,control).complete,false);
 assert.throws(()=>rtcSourceFrameControlRoute({routes:[{index:0,session:'A'},{index:1,session:'A'}],moves:[{source:0}]}));
});
